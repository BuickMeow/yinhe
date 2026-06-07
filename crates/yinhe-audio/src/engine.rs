use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, TryRecvError, unbounded};
use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::ParallelismOptions;
use xsynth_core::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, SynthFormat};
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
    LoadMidi { midi: Arc<MidiFile> },
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

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

/// Frames rendered per chunk by the pre-render thread.
const CHUNK_FRAMES: usize = 256;
/// Total number of f32 samples per chunk.
const CHUNK_LEN: usize = CHUNK_FRAMES * STEREO_CHANNELS;

/// Number of chunks in the ring buffer (~186ms at 44.1kHz).
const RING_CHUNKS: usize = 32;

/// Type of a single audio chunk (stack-allocated, no heap allocation).
pub type AudioChunk = [f32; CHUNK_LEN];

struct SortedCC {
    sample: u64,
    channel: u32,
    event: ChannelAudioEvent,
}

struct ActiveNote {
    key: u8,
    channel: u8,
    end_sample: u64,
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
    active_notes: Vec<ActiveNote>,
    midi: Option<Arc<MidiFile>>,
}

impl AudioEngine {
    fn new(sample_rate: u32, num_channels: u32, active_mask: Vec<bool>) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
            let num_channels = num_channels.max(16);
            let config = ChannelGroupConfig {
                channel_init_options: ChannelInitOptions {
                    fade_out_killing: true,
                },
                format: SynthFormat::Custom {
                    channels: num_channels,
                },
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
                interleaved_buffer: vec![0.0f32; sample_rate as usize * STEREO_CHANNELS],
                duration_samples: 0,
                note_cursors: [0; 128],
                cc_events: Vec::new(),
                cc_cursor: 0,
                active_notes: Vec::new(),
                midi: None,
            }
        })
    }

    fn load_midi(&mut self, midi: &MidiFile) {
        self.setup_percussion(midi);

        self.cc_events.clear();
        self.cc_cursor = 0;
        self.active_notes.clear();
        let sr = self.sample_rate as f64;

        for evt in &midi.control_events {
            let (sample, channel, event) = match evt {
                MidiControlEvent::ControlChange {
                    tick,
                    channel,
                    controller,
                    value,
                    ..
                } => (
                    (midi.tick_to_seconds(*tick as u64) * sr) as u64,
                    *channel as u32,
                    ChannelAudioEvent::Control(ControlEvent::Raw(*controller, *value)),
                ),
                MidiControlEvent::ProgramChange {
                    tick,
                    channel,
                    program,
                    ..
                } => (
                    (midi.tick_to_seconds(*tick as u64) * sr) as u64,
                    *channel as u32,
                    ChannelAudioEvent::ProgramChange(*program),
                ),
                MidiControlEvent::PitchBend {
                    tick,
                    channel,
                    value,
                    ..
                } => (
                    (midi.tick_to_seconds(*tick as u64) * sr) as u64,
                    *channel as u32,
                    ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                        *value as f32 / 8192.0,
                    )),
                ),
            };
            self.cc_events.push(SortedCC {
                sample,
                channel,
                event,
            });
        }
        self.cc_events.sort_by_key(|e| e.sample);

        self.note_cursors = [0; 128];
        self.duration_samples = (midi.tick_to_seconds(midi.tick_length) * sr) as u64;
    }

    fn setup_percussion(&mut self, midi: &MidiFile) {
        let num_ports = self.num_channels / 16;
        for port in 0..num_ports {
            let ch = (port * 16 + 9) as usize;
            if ch < self.num_channels as usize && self.active_mask.get(ch).copied().unwrap_or(false)
            {
                self.channel_group.send_event(SynthEvent::Channel(
                    ch as u32,
                    ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
                ));
            }
        }
        for evt in &midi.control_events {
            if let MidiControlEvent::ControlChange {
                channel,
                controller: 0,
                value,
                ..
            } = evt
            {
                let ch = *channel as usize;
                if ch < self.num_channels as usize
                    && self.active_mask.get(ch).copied().unwrap_or(false)
                {
                    self.channel_group.send_event(SynthEvent::Channel(
                        ch as u32,
                        ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(*value >= 120)),
                    ));
                }
            }
        }
    }

    fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) {
        let base_ch = (port as u32) * 16;
        if base_ch >= self.num_channels {
            return;
        }
        let end_ch = (base_ch + 16).min(self.num_channels);
        let has_active =
            (base_ch..end_ch).any(|ch| self.active_mask.get(ch as usize).copied().unwrap_or(false));
        if !has_active {
            return;
        }
        let _ =
            self.sf_manager
                .load_for_port(port, paths, &mut self.channel_group, &self.active_mask);
    }

    fn seek_to(&mut self, sample: u64) {
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::ResetControl,
            )));

        self.sample_position = sample;
        self.note_cursors = [0; 128];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self.cc_events.partition_point(|cc| cc.sample < sample);

        if let Some(ref midi) = self.midi {
            let sr = self.sample_rate as f64;
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                self.note_cursors[key] = notes.partition_point(|n| {
                    let note_start = (midi.tick_to_seconds(n.start_tick as u64) * sr) as u64;
                    note_start < sample
                });
            }
        }

        self.inject_chase(sample);
    }

    fn inject_chase(&mut self, target_sample: u64) {
        let mut state = [ChannelState::default(); 256];
        for cc in &self.cc_events {
            if cc.sample >= target_sample {
                break;
            }
            let ch = cc.channel as usize;
            if ch >= 256 {
                continue;
            }
            state[ch].apply(&cc.event);
        }

        for ch in 0..256u32 {
            if !self.active_mask.get(ch as usize).copied().unwrap_or(false) {
                continue;
            }
            state[ch as usize].send_to(ch, &mut self.channel_group);
        }
    }

    fn handle_command(&mut self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::Play { from_sample } => {
                self.seek_to(from_sample);
                self.playing = true;
            }
            AudioCommand::Resume => self.playing = true,
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

    fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
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
            self.channel_group.send_event(SynthEvent::Channel(
                cc.channel,
                ChannelEvent::Audio(cc.event),
            ));
            self.cc_cursor += 1;
        }

        if let Some(ref midi) = self.midi {
            // NoteOn + track active notes (single pass over 128 keys)
            for key in 0..128usize {
                let notes = &midi.key_notes[key];
                while self.note_cursors[key] < notes.len() {
                    let note = &notes[self.note_cursors[key]];
                    if note.velocity <= 1 {
                        self.note_cursors[key] += 1;
                        continue;
                    }
                    let note_start = (midi.tick_to_seconds(note.start_tick as u64) * sr) as u64;
                    if note_start >= end {
                        break;
                    }

                    self.channel_group.send_event(SynthEvent::Channel(
                        note.channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: note.key,
                            vel: note.velocity,
                        }),
                    ));

                    self.active_notes.push(ActiveNote {
                        key: note.key,
                        channel: note.channel,
                        end_sample: (midi.tick_to_seconds(note.end_tick as u64) * sr) as u64,
                    });

                    self.note_cursors[key] += 1;
                }
            }

            // NoteOff: only check active notes (O(active) not O(128 * 1024))
            self.active_notes.retain(|an| {
                if an.end_sample >= start && an.end_sample < end {
                    self.channel_group.send_event(SynthEvent::Channel(
                        an.channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
                    false // remove from active list
                } else if an.end_sample < start {
                    false // already past, clean up
                } else {
                    true // still active
                }
            });
        }

        let interleaved = &mut self.interleaved_buffer[..frames * STEREO_CHANNELS];
        interleaved.fill(0.0);
        self.channel_group.read_samples(interleaved);
        output[..frames * STEREO_CHANNELS].copy_from_slice(interleaved);

        self.sample_position = end;
    }
}

// ── Chase state helper ──

#[derive(Clone, Copy, Default)]
struct ChannelState {
    bank_msb: u8,
    bank_lsb: u8,
    program: u8,
    volume: u8,
    pan: u8,
    expression: u8,
    sustain: u8,
    cutoff: u8,
    resonance: u8,
    attack: u8,
    release: u8,
    pitch_bend: f32,
    rpn_msb: u8,
    rpn_lsb: u8,
    data_entry_msb: u8,
    data_entry_lsb: u8,
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
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            101,
            self.rpn_msb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            100,
            self.rpn_lsb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            6,
            self.data_entry_msb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            38,
            self.data_entry_lsb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            0,
            self.bank_msb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            32,
            self.bank_lsb,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            7,
            self.volume,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(10, self.pan)));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            11,
            self.expression,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            64,
            self.sustain,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            73,
            self.attack,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            72,
            self.release,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            74,
            self.cutoff,
        )));
        send(ChannelAudioEvent::Control(ControlEvent::Raw(
            71,
            self.resonance,
        )));
        send(ChannelAudioEvent::ProgramChange(self.program));
        send(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
            self.pitch_bend,
        )));
    }
}

// ── Spawn cpal stream with engine inside callback ──

pub struct CpalAudioHandle {
    pub handle: AudioHandle,
    pub sample_rate: u32,
    pub num_channels: u32,
    _stream: cpal::Stream,
}

pub fn spawn_cpal_audio(
    sample_rate: u32,
    num_channels: u32,
    active_mask: Vec<bool>,
) -> Result<CpalAudioHandle, String> {
    use crossbeam_queue::ArrayQueue;

    let (cmd_tx, cmd_rx) = unbounded::<AudioCommand>();
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));

    let sp = Arc::clone(&sample_position);
    let pl = Arc::clone(&playing);
    let dur = Arc::clone(&duration_samples);

    // Ring buffer: 32 chunks of 256 stereo frames ≈ 186ms at 44.1kHz
    let ring = Arc::new(ArrayQueue::<AudioChunk>::new(RING_CHUNKS));
    let ring_clone = Arc::clone(&ring);

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let channels = supported.channels() as usize;

    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let mut engine = AudioEngine::new(sample_rate, num_channels, active_mask);

    // ── Pre-render thread: owns the engine, renders ahead into the ring buffer ──
    std::thread::Builder::new()
        .name("yinhe-prerender".into())
        .spawn(move || {
            yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
                let mut buf = [0.0f32; CHUNK_LEN];
                let mut initialized = false;

                loop {
                    // Drain pending commands (non-blocking when playing)
                    loop {
                        match cmd_rx.try_recv() {
                            Ok(cmd) => {
                                let is_load_midi = matches!(&cmd, AudioCommand::LoadMidi { .. });
                                if let AudioCommand::LoadMidi { ref midi } = cmd {
                                    dur.store(
                                        (midi.tick_to_seconds(midi.tick_length)
                                            * engine.sample_rate as f64)
                                            as u64,
                                        Ordering::Relaxed,
                                    );
                                }
                                engine.handle_command(cmd);
                                if is_load_midi {
                                    initialized = true;
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }

                    if engine.playing && initialized {
                        buf.fill(0.0);
                        engine.render(&mut buf);

                        sp.store(engine.sample_position, Ordering::Relaxed);
                        pl.store(true, Ordering::Relaxed);

                        // Push chunk into ring buffer (one memcpy of 2KB, not 512 CAS ops)
                        if ring_clone.push(buf).is_err() {
                            // Buffer full — consumer is slow or device underrun.
                            // Brief yield then try again next iteration;
                            // meanwhile check for commands.
                            std::thread::yield_now();
                            // Check for Pause/Stop
                            loop {
                                match cmd_rx.try_recv() {
                                    Ok(cmd) => engine.handle_command(cmd),
                                    Err(TryRecvError::Empty) => break,
                                    Err(TryRecvError::Disconnected) => return,
                                }
                            }
                        }
                    } else {
                        // Paused or not initialized — block until next command
                        pl.store(false, Ordering::Relaxed);
                        sp.store(engine.sample_position, Ordering::Relaxed);
                        match cmd_rx.recv() {
                            Ok(cmd) => {
                                let is_load_midi = matches!(&cmd, AudioCommand::LoadMidi { .. });
                                if let AudioCommand::LoadMidi { ref midi } = cmd {
                                    dur.store(
                                        (midi.tick_to_seconds(midi.tick_length)
                                            * engine.sample_rate as f64)
                                            as u64,
                                        Ordering::Relaxed,
                                    );
                                }
                                engine.handle_command(cmd);
                                if is_load_midi {
                                    initialized = true;
                                }
                            }
                            Err(_) => return, // channel closed, exit thread
                        }
                    }
                }
            })
        })
        .map_err(|e| format!("Failed to spawn pre-render thread: {e}"))?;

    // ── Audio callback: pop chunks from ring buffer and copy to output ──
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut offset = 0;
                while offset < data.len() {
                    match ring.pop() {
                        Some(chunk) => {
                            let n = chunk.len().min(data.len() - offset);
                            data[offset..offset + n].copy_from_slice(&chunk[..n]);
                            offset += n;
                        }
                        None => {
                            // Underrun — fill remaining with silence
                            data[offset..].fill(0.0);
                            break;
                        }
                    }
                }
            },
            |err| eprintln!("Audio stream error: {}", err),
            None,
        )
        .map_err(|e| format!("Failed to build stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start stream: {e}"))?;

    Ok(CpalAudioHandle {
        handle: AudioHandle {
            cmd_tx,
            sample_position,
            playing,
            duration_samples,
        },
        sample_rate,
        num_channels,
        _stream: stream,
    })
}

// ── Unit tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_midi::MidiFile;

    fn make_midi_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> MidiFile {
        let mut midi = MidiFile::default();
        midi.ticks_per_beat = 480;
        midi.tempo_segments = vec![yinhe_midi::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: 500_000, // 120 BPM
        }];
        for (key, start_tick, end_tick, velocity, channel) in notes {
            midi.key_notes[key as usize].push(yinhe_midi::Note {
                start_tick,
                end_tick,
                key,
                velocity,
                channel,
                track: 0,
            });
            midi.tick_length = midi.tick_length.max(end_tick as u64);
        }
        midi
    }

    #[test]
    fn test_channels_for_midi_basic() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 100, 0), // ch0
            (64, 0, 480, 100, 1), // ch1
            (67, 0, 480, 100, 9), // ch9 (drum)
        ]);
        let (num_ch, mask) = channels_for_midi(&midi);
        assert_eq!(num_ch, 16);
        assert!(mask[0]); // ch0 active
        assert!(mask[1]); // ch1 active
        assert!(mask[9]); // ch9 active
        assert!(!mask[2]); // ch2 inactive
    }

    #[test]
    fn test_channels_for_midi_multi_port() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 100, 0),  // port 0, ch0
            (60, 0, 480, 100, 16), // port 1, ch0
        ]);
        let (num_ch, mask) = channels_for_midi(&midi);
        assert_eq!(num_ch, 32);
        assert!(mask[0]);
        assert!(mask[16]);
        assert!(!mask[15]);
    }

    #[test]
    fn test_channels_for_midi_skips_velocity_0_1() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 0, 0), // vel 0 — should be skipped
            (61, 0, 480, 1, 0), // vel 1 — should be skipped
            (62, 0, 480, 2, 0), // vel 2 — active
        ]);
        let (_num_ch, mask) = channels_for_midi(&midi);
        assert!(mask[0]);
        // ch0 has 1 active note (vel 2)
    }

    #[test]
    fn test_channels_for_midi_cc_activates_channel() {
        let mut midi = MidiFile::default();
        midi.control_events.push(MidiControlEvent::ControlChange {
            tick: 0,
            channel: 5,
            controller: 7,
            value: 100,
            track: 0,
        });
        let (num_ch, mask) = channels_for_midi(&midi);
        assert!(num_ch >= 16);
        assert!(mask[5]);
    }

    #[test]
    fn test_channels_for_midi_empty() {
        let midi = MidiFile::default();
        let (num_ch, mask) = channels_for_midi(&midi);
        assert_eq!(num_ch, 16); // minimum 16
        assert!(mask.iter().all(|&b| !b)); // no active channels
    }

    #[test]
    fn test_channel_state_apply() {
        let mut state = ChannelState::default();
        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)));
        assert_eq!(state.volume, 100);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(10, 64)));
        assert_eq!(state.pan, 64);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)));
        assert_eq!(state.rpn_msb, 0);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(100, 0)));
        assert_eq!(state.rpn_lsb, 0);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::Raw(6, 12)));
        assert_eq!(state.data_entry_msb, 12);

        state.apply(&ChannelAudioEvent::ProgramChange(42));
        assert_eq!(state.program, 42);

        state.apply(&ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
            0.5,
        )));
        assert!((state.pitch_bend - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_channel_state_default() {
        let state = ChannelState::default();
        assert_eq!(state.volume, 0);
        assert_eq!(state.pan, 0);
        assert_eq!(state.program, 0);
        assert!((state.pitch_bend).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sorted_cc_ordering() {
        let mut cc = vec![
            SortedCC {
                sample: 100,
                channel: 0,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
            },
            SortedCC {
                sample: 50,
                channel: 0,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
            },
            SortedCC {
                sample: 200,
                channel: 0,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 60)),
            },
        ];
        cc.sort_by_key(|e| e.sample);
        assert_eq!(cc[0].sample, 50);
        assert_eq!(cc[1].sample, 100);
        assert_eq!(cc[2].sample, 200);
    }

    #[test]
    fn test_active_mask_length() {
        let midi = make_midi_with_notes(vec![
            (60, 0, 480, 100, 0),
            (60, 0, 480, 100, 31), // port 1, ch15
        ]);
        let (num_ch, mask) = channels_for_midi(&midi);
        assert_eq!(num_ch, 32);
        assert_eq!(mask.len(), 32);
    }
}
