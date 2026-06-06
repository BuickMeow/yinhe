use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, SynthFormat};
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};
use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel_group::ParallelismOptions;

use yinhe_midi::MidiFile;

use crate::scheduler::MidiEventScheduler;
use crate::soundfont::SoundFontManager;

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
            yinhe_midi::MidiControlEvent::ControlChange { channel, .. }
            | yinhe_midi::MidiControlEvent::ProgramChange { channel, .. }
            | yinhe_midi::MidiControlEvent::PitchBend { channel, .. } => *channel as usize,
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

pub struct AudioEngine {
    channel_group: ChannelGroup,
    num_channels: u32,
    active_mask: Vec<bool>,
    scheduler: MidiEventScheduler,
    pub sf_manager: SoundFontManager,
    sample_rate: u32,
    sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    interleaved_buffer: Vec<f32>,
    duration_samples: u64,
}

impl AudioEngine {
    pub fn new(sample_rate: u32, num_channels: u32, active_mask: Vec<bool>) -> Self {
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

        let channel_group = ChannelGroup::new(config);

        Self {
            channel_group,
            num_channels,
            active_mask,
            scheduler: MidiEventScheduler::new(),
            sf_manager: SoundFontManager::new(sample_rate),
            sample_rate,
            sample_position: Arc::new(AtomicU64::new(0)),
            playing: Arc::new(AtomicBool::new(false)),
            interleaved_buffer: Vec::new(),
            duration_samples: 0,
        }
    }

    pub fn num_channels(&self) -> u32 {
        self.num_channels
    }

    pub fn load_midi(&mut self, midi: &MidiFile) {
        self.setup_percussion(midi);
        self.scheduler.build(midi, self.sample_rate);
        self.duration_samples = (midi.duration * self.sample_rate as f64) as u64;
        self.pre_allocate_buffer();
    }

    fn setup_percussion(&mut self, midi: &MidiFile) {
        let num_ports = self.num_channels / 16;
        for port in 0..num_ports {
            let ch = (port * 16 + 9) as usize;
            if ch < self.num_channels as usize && self.active_mask.get(ch).copied().unwrap_or(false) {
                self.channel_group.send_event(SynthEvent::Channel(
                    ch as u32,
                    ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
                ));
            }
        }

        for evt in &midi.control_events {
            if let yinhe_midi::MidiControlEvent::ControlChange {
                channel,
                controller: 0,
                value,
                ..
            } = evt
            {
                let ch = *channel as usize;
                if ch < self.num_channels as usize && self.active_mask.get(ch).copied().unwrap_or(false) {
                    let is_drum = *value >= 120;
                    self.channel_group.send_event(SynthEvent::Channel(
                        ch as u32,
                        ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(is_drum)),
                    ));
                }
            }
        }
    }

    fn pre_allocate_buffer(&mut self) {
        let max_frames = self.sample_rate as usize;
        self.interleaved_buffer = vec![0.0f32; max_frames * 2];
    }

    pub fn load_soundfonts(&mut self, midi: &MidiFile) -> Result<(), String> {
        self.sf_manager.load_for_midi(midi, &mut self.channel_group)
    }

    pub fn load_soundfont_for_port(&mut self, port: u8, paths: &[String]) -> Result<(), String> {
        let base_ch = (port as u32) * 16;
        if base_ch >= self.num_channels {
            return Ok(());
        }
        let end_ch = (base_ch + 16).min(self.num_channels);
        let has_active = (base_ch..end_ch).any(|ch| {
            self.active_mask.get(ch as usize).copied().unwrap_or(false)
        });
        if !has_active {
            return Ok(());
        }
        self.sf_manager
            .load_for_port(port, paths, &mut self.channel_group, &self.active_mask)
    }

    pub fn read_samples(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;
        if frames == 0 {
            return;
        }

        let start = self.sample_position.load(Ordering::Relaxed);
        let end = start + frames as u64;

        self.scheduler
            .push_events(start, end, &mut self.channel_group);

        let interleaved = &mut self.interleaved_buffer[..frames * 2];
        interleaved.fill(0.0);
        self.channel_group.read_samples(interleaved);

        output[..frames * 2].copy_from_slice(interleaved);

        self.sample_position.store(end, Ordering::Relaxed);
    }

    pub fn seek(&mut self, sample: u64) {
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Audio(xsynth_core::channel::ChannelAudioEvent::AllNotesOff),
        ));
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Audio(xsynth_core::channel::ChannelAudioEvent::ResetControl),
        ));

        self.scheduler.seek(sample);
        self.sample_position.store(sample, Ordering::Relaxed);

        self.scheduler
            .inject_chase(sample, &mut self.channel_group);
    }

    pub fn play(&self) {
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        self.playing.store(false, Ordering::Relaxed);
        self.seek(0);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
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

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    pub fn reset(&mut self) {
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Audio(xsynth_core::channel::ChannelAudioEvent::AllNotesOff),
        ));
        self.channel_group.send_event(SynthEvent::AllChannels(
            ChannelEvent::Audio(xsynth_core::channel::ChannelAudioEvent::ResetControl),
        ));
        self.scheduler.reset();
        self.sample_position.store(0, Ordering::Relaxed);
    }
}
