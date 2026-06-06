use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, SynthFormat};
use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel_group::ParallelismOptions;
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use yinhe_midi::MidiFile;
use yinhe_types::MidiControlEvent;

use crate::soundfont::SoundFontManager;

// ── Public API ──

pub enum AudioCommand {
    Play { from_sample: u64 },
    Resume,
    Pause,
    Stop,
    Seek { sample: u64 },
    LoadMidi { midi: MidiFile },
    LoadSoundFont { port: u8, paths: Vec<String> },
}

pub struct AudioHandle {
    pub(crate) cmd_tx: Sender<AudioCommand>,
    sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    duration_samples: Arc<AtomicU64>,
}

impl AudioHandle {
    pub fn send(&self, cmd: AudioCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    pub fn sample_position_arc(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.sample_position)
    }

    pub fn playing_arc(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.playing)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn duration_samples(&self) -> u64 {
        self.duration_samples.load(Ordering::Relaxed)
    }
}

// ── Channel analysis ──

pub fn channels_for_midi(midi: &MidiFile) -> (u32, Vec<bool>) {
    let mut ch_active = [0u32; 256];
    for notes in &midi.key_notes {
        for note in notes {
            if note.velocity > 1 {
                ch_active[note.channel as usize] += 1;
            }
        }
    }
    for evt in &midi.control_events {
        let ch = match evt {
            MidiControlEvent::ControlChange { channel, .. }
            | MidiControlEvent::ProgramChange { channel, .. }
            | MidiControlEvent::PitchBend { channel, .. } => *channel as usize,
        };
        if ch < 256 {
            ch_active[ch] = ch_active[ch].max(1);
        }
    }

    let max_active_ch = ch_active.iter().rposition(|&c| c > 0).unwrap_or(0);
    let num_ports = ((max_active_ch / 16) + 1).max(1);
    let num_channels = (num_ports as u32) * 16;

    let active_mask: Vec<bool> = ch_active[..num_channels as usize]
        .iter()
        .map(|&c| c > 0)
        .collect();
    (num_channels, active_mask)
}

// ── Internal engine ──

struct SortedCC {
    sample: u64,
    channel: u32,
    event: ChannelAudioEvent,
}

struct AudioEngine {
    channel_group: ChannelGroup,
    num_channels: u32,
    active_mask: Vec<bool>,
    sf_manager: SoundFontManager,
    sample_rate: u32,
    sample_position: u64,
    playing: bool,
    interleaved_buffer: Vec<f32>,
    duration_samples: u64,

    note_cursors: [usize; 128],
    cc_events: Vec<SortedCC>,
    cc_cursor: usize,
    midi: Option<MidiFile>,
}

impl AudioEngine {
    fn new(sample_rate: u32, num_channels: u32, active_mask: Vec<bool>) -> Self {
        let num_channels = num_channels.max(16);
        let config = ChannelGroupConfig {
            channel_init_options: ChannelInitOptions { fade_out_killing: true },
            format: SynthFormat::Custom { channels: num_channels },
            audio_params: AudioStreamParams {
                sample_rate,
                channels: ChannelCount::Stereo,
            },
            parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
        };

        Self {
            channel_group: ChannelGroup::new(config),
            num_channels,
            active_mask,
            sf_manager: SoundFontManager::new(sample_rate),
            sample_rate,
            sample_position: 0,
            playing: false,
            interleaved_buffer: vec![0.0f32; sample_rate as usize * 2],
            duration_samples: 0,
            note_cursors: [0; 128],
            cc_events: Vec::new(),
            cc_cursor: 0,
            midi: None,
        }
    }

    fn load_midi(&mut self, midi: &MidiFile) {
        self.setup_percussion(midi);

        self.cc_events.clear();
        self.cc_cursor = 0;
        let sr = self.sample_rate as f64;

        for evt in &midi.control_events {
            let (sample, channel, event) = match evt {
                MidiControlEvent::ControlChange { tick, channel, controller, value, .. } => {
                    ((midi.tick_to_seconds(*tick) * sr) as u64, *channel as u32,
                     ChannelAudioEvent::Control(ControlEvent::Raw(*controller, *value)))
                }
                MidiControlEvent::ProgramChange { tick, channel, program, .. } => {
                    ((midi.tick_to_seconds(*tick) * sr) as u64, *channel as u32,
                     ChannelAudioEvent::ProgramChange(*program))
                }
                MidiControlEvent::PitchBend { tick, channel, value, .. } => {
                    ((midi.tick_to_seconds(*tick) * sr) as u64, *channel as u32,
                     ChannelAudioEvent::Control(ControlEvent::PitchBendValue(*value as f32 / 8192.0)))
                }
            };
            self.cc_events.push(SortedCC { sample, channel, event });
        }
        self.cc_events.sort_by_key(|e| e.sample);

        self.note_cursors = [0; 128];
        self.duration_samples = (midi.duration * sr) as u64;
    }

    fn setup_percussion(&mut self, midi: &MidiFile) {
        let num_ports = self.num_channels / 16;
        for port in 0..num_ports {
            let ch = (port * 16 + 9) as usize;
            if ch < self.num_channels as usize && self.active_mask.get(ch).copied().unwrap_or(false) {
                self.channel_group.send_event(SynthEvent::Channel(
                    ch as u32, ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
                ));
            }
        }
        for evt in &midi.control_events {
            if let MidiControlEvent::ControlChange { channel, controller: 0, value, .. } = evt {
                let ch = *channel as usize;
                if ch < self.num_channels as usize && self.active_mask.get(ch).copied().unwrap_or(false) {
                    self.channel_group.send_event(SynthEvent::Channel(
                        ch as u32, ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(*value >= 120)),
                    ));
                }
            }
        }
    }

    fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
        let base_ch = (port as u32) * 16;
        if base_ch >= self.num_channels { return; }
        let end_ch = (base_ch + 16).min(self.num_channels);
        let has_active = (base_ch..end_ch).any(|ch| self.active_mask.get(ch as usize).copied().unwrap_or(false));
        if !has_active { return; }
        let _ = self.sf_manager.load_for_port(port, paths, &mut self.channel_group, &self.active_mask);
    }

    fn seek_to(&mut self, sample: u64) {
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Audio(ChannelAudioEvent::AllNotesOff)));
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Audio(ChannelAudioEvent::ResetControl)));

        self.sample_position = sample;
        self.note_cursors = [0; 128];
        self.cc_cursor = 0;

        // Advance CC cursor
        self.cc_cursor = self.cc_events.partition_point(|cc| cc.sample < sample);

        // Advance note cursors — skip notes that started before seek position
        if let Some(ref midi) = self.midi {
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                self.note_cursors[key] = notes.partition_point(|n| {
                    let note_start = (n.start * self.sample_rate as f64) as u64;
                    note_start < sample
                });
            }
        }

        // Inject chase: replay CC state up to sample position
        self.inject_chase(sample);
    }

    fn inject_chase(&mut self, target_sample: u64) {
        // Collect latest state per channel from CC events before target
        let mut state = [ChannelState::default(); 256];
        for cc in &self.cc_events {
            if cc.sample >= target_sample { break; }
            let ch = cc.channel as usize;
            if ch >= 256 { continue; }
            state[ch].apply(&cc.event);
        }

        // Send chase events for active channels
        for ch in 0..256u32 {
            if !self.active_mask.get(ch as usize).copied().unwrap_or(false) { continue; }
            let s = &state[ch as usize];
            s.send_to(ch, &mut self.channel_group);
        }
    }

    fn process_commands(&mut self, rx: &Receiver<AudioCommand>) {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                AudioCommand::Play { from_sample } => {
                    self.seek_to(from_sample);
                    self.playing = true;
                }
                AudioCommand::Resume => {
                    self.playing = true;
                }
                AudioCommand::Pause => self.playing = false,
                AudioCommand::Stop => {
                    self.playing = false;
                    self.seek_to(0);
                }
                AudioCommand::Seek { sample } => self.seek_to(sample),
                AudioCommand::LoadMidi { midi } => {
                    self.playing = false;
                    self.load_midi(&midi);
                    self.midi = Some(midi);
                }
                AudioCommand::LoadSoundFont { port, paths } => {
                    self.load_soundfont_for_port(port, &paths);
                }
            }
        }
    }

    fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;
        if frames == 0 || !self.playing {
            output.fill(0.0);
            return;
        }

        let start = self.sample_position;
        let end = start + frames as u64;
        let sr = self.sample_rate as f64;

        // Push CC events
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].sample < end {
            let cc = &self.cc_events[self.cc_cursor];
            self.channel_group.send_event(SynthEvent::Channel(cc.channel, ChannelEvent::Audio(cc.event.clone())));
            self.cc_cursor += 1;
        }

        // Push NoteOn events
        if let Some(ref midi) = self.midi {
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                while self.note_cursors[key] < notes.len() {
                    let note = &notes[self.note_cursors[key]];
                    if note.velocity <= 1 {
                        self.note_cursors[key] += 1;
                        continue;
                    }
                    let note_start = (note.start * sr) as u64;
                    if note_start >= end { break; }

                    self.channel_group.send_event(SynthEvent::Channel(
                        note.channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: note.key, vel: note.velocity }),
                    ));
                    self.note_cursors[key] += 1;
                }
            }

            // Push NoteOff events: scan recent notes for end times in this window
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                let from = self.note_cursors[key].saturating_sub(1024);
                let to = self.note_cursors[key].min(notes.len());
                for i in from..to {
                    let note = &notes[i];
                    if note.velocity <= 1 { continue; }
                    let note_end = (note.end * sr) as u64;
                    if note_end >= start && note_end < end {
                        self.channel_group.send_event(SynthEvent::Channel(
                            note.channel as u32,
                            ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: note.key }),
                        ));
                    }
                }
            }
        }

        // Render
        let interleaved = &mut self.interleaved_buffer[..frames * 2];
        interleaved.fill(0.0);
        self.channel_group.read_samples(interleaved);
        output[..frames * 2].copy_from_slice(interleaved);

        self.sample_position = end;
    }
}

// ── Chase state helper ──

#[derive(Clone, Copy, Default)]
struct ChannelState {
    bank_msb: u8, bank_lsb: u8, program: u8,
    volume: u8, pan: u8, expression: u8, sustain: u8,
    cutoff: u8, resonance: u8, attack: u8, release: u8,
    pitch_bend: f32,
    rpn_msb: u8, rpn_lsb: u8, data_entry_msb: u8, data_entry_lsb: u8,
}

impl ChannelState {
    fn apply(&mut self, event: &ChannelAudioEvent) {
        match event {
            ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => match cc {
                0 => self.bank_msb = *val,
                6 => self.data_entry_msb = *val,
                7 => self.volume = *val,
                10 => self.pan = *val,
                11 => self.expression = *val,
                32 => self.bank_lsb = *val,
                38 => self.data_entry_lsb = *val,
                64 => self.sustain = *val,
                71 => self.resonance = *val,
                72 => self.release = *val,
                73 => self.attack = *val,
                74 => self.cutoff = *val,
                100 => self.rpn_lsb = *val,
                101 => self.rpn_msb = *val,
                _ => {}
            },
            ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => self.pitch_bend = *v,
            ChannelAudioEvent::ProgramChange(p) => self.program = *p,
            _ => {}
        }
    }

    fn send_to(&self, ch: u32, cg: &mut ChannelGroup) {
        let mut send = |event: ChannelAudioEvent| {
            cg.send_event(SynthEvent::Channel(ch, ChannelEvent::Audio(event)));
        };
        // RPN order: MSB → LSB → Data Entry
        send(ChannelAudioEvent::Control(ControlEvent::Raw(101, self.rpn_msb)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(100, self.rpn_lsb)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(6, self.data_entry_msb)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(38, self.data_entry_lsb)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(0, self.bank_msb)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(32, self.bank_lsb)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(7, self.volume)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(10, self.pan)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(11, self.expression)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(64, self.sustain)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(73, self.attack)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(72, self.release)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(74, self.cutoff)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(71, self.resonance)));
        send(ChannelAudioEvent::ProgramChange(self.program));
        send(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(self.pitch_bend)));
    }
}

// ── Spawn cpal stream with engine inside callback ──

pub struct CpalAudioHandle {
    pub handle: AudioHandle,
    pub sample_rate: u32,
    _stream: cpal::Stream,
}

pub fn spawn_cpal_audio(
    sample_rate: u32,
    num_channels: u32,
    active_mask: Vec<bool>,
) -> Result<CpalAudioHandle, String> {
    let (cmd_tx, cmd_rx) = unbounded::<AudioCommand>();
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));

    let sp = Arc::clone(&sample_position);
    let pl = Arc::clone(&playing);
    let dur = Arc::clone(&duration_samples);

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let channels = supported.channels() as usize;

    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    // Engine lives inside the cpal callback — no Mutex, no contention
    let mut engine = AudioEngine::new(sample_rate, num_channels, active_mask);
    let mut initialized = false;

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            // Process commands (non-blocking)
            loop {
                match cmd_rx.try_recv() {
                    Ok(cmd) => {
                        match &cmd {
                            AudioCommand::LoadMidi { midi } => {
                                dur.store((midi.duration * engine.sample_rate as f64) as u64, Ordering::Relaxed);
                            }
                            AudioCommand::Play { .. } | AudioCommand::Pause | AudioCommand::Stop => {
                                // Sync playing state to atomics
                            }
                            _ => {}
                        }
                        // Apply command directly — we own the engine
                        match cmd {
                            AudioCommand::Play { from_sample } => {
                                engine.seek_to(from_sample);
                                engine.playing = true;
                            }
                            AudioCommand::Resume => {
                                engine.playing = true;
                            }
                            AudioCommand::Pause => engine.playing = false,
                            AudioCommand::Stop => {
                                engine.playing = false;
                                engine.seek_to(0);
                            }
                            AudioCommand::Seek { sample } => engine.seek_to(sample),
                            AudioCommand::LoadMidi { midi } => {
                                engine.playing = false;
                                engine.load_midi(&midi);
                                engine.midi = Some(midi);
                                initialized = true;
                            }
                            AudioCommand::LoadSoundFont { port, paths } => {
                                engine.load_soundfont_for_port(port, &paths);
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        data.fill(0.0);
                        return;
                    }
                }
            }

            // Sync state to atomics for UI to read
            sp.store(engine.sample_position, Ordering::Relaxed);
            pl.store(engine.playing, Ordering::Relaxed);

            // Render
            if initialized {
                engine.render(data);
            } else {
                data.fill(0.0);
            }
        },
        |err| eprintln!("Audio stream error: {}", err),
        None,
    ).map_err(|e| format!("Failed to build stream: {}", e))?;

    stream.play().map_err(|e| format!("Failed to start stream: {}", e))?;

    Ok(CpalAudioHandle {
        handle: AudioHandle { cmd_tx, sample_position, playing, duration_samples },
        sample_rate,
        _stream: stream,
    })
}
