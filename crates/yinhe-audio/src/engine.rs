use std::sync::Arc;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat,
};
use xsynth_core::soundfont::SoundfontBase;
use xsynth_core::{AudioStreamParams, ChannelCount};

use yinhe_core::YinModel;

use crate::audio_model::{ActiveNote, AudioModel, SortedCC};
use crate::soundfont::SoundFontManager;
use crate::spawn::AudioCommand;

/// Core MIDI synthesis engine.  Owned by the renderer thread.
pub(crate) struct AudioEngine {
    pub(crate) channel_group: ChannelGroup,
    /// Map: source MIDI channel (0..256) → compacted XSynth channel index.
    pub(crate) channel_map: Box<[u32; 256]>,
    pub(crate) active_mask: Vec<bool>,
    pub(crate) sf_manager: SoundFontManager,
    pub(crate) sample_rate: u32,
    pub(crate) sample_position: u64,
    pub(crate) playing: bool,
    pub(crate) duration_samples: u64,

    pub(crate) note_cursor: [usize; 128],
    /// Reference to the full YinModel (notes are read directly from
    /// `yin_model.notes[key]` with real-time tick→sample conversion).
    pub(crate) yin_model: Option<Arc<YinModel>>,

    pub(crate) cc_events: Vec<SortedCC>,
    pub(crate) cc_cursor: usize,
    pub(crate) active_notes: Vec<ActiveNote>,
    pub(crate) ended_notes: Vec<ActiveNote>,
    pub(crate) model: Option<AudioModel>,
    pub(crate) skip_track: Vec<bool>,
    /// Set when Play arrives during async model loading.
    pub(crate) pending_play_from_sample: Option<u64>,
    /// Linear/Curve 自动化段播放时的中间事件 tick 间隔（默认 1）。
    pub(crate) automation_density: u32,
}

impl AudioEngine {
    pub(crate) fn new(sample_rate: u32, _num_channels: u32, active_mask: Vec<bool>) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
            let mut channel_map = Box::new([u32::MAX; 256]);
            let mut next_dense: u32 = 0;
            for (src, &alive) in active_mask.iter().enumerate().take(256) {
                if alive {
                    channel_map[src] = next_dense;
                    next_dense += 1;
                }
            }
            let compacted_channels = next_dense.max(1);

            let config = ChannelGroupConfig {
                channel_init_options: ChannelInitOptions {
                    fade_out_killing: true,
                },
                format: SynthFormat::Custom {
                    channels: compacted_channels,
                },
                audio_params: AudioStreamParams {
                    sample_rate,
                    channels: ChannelCount::Stereo,
                },
                parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
            };

            Self {
                channel_group: ChannelGroup::new(config),
                channel_map,
                active_mask,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                playing: false,
                duration_samples: 0,
                note_cursor: [0; 128],
                yin_model: None,
                cc_events: Vec::new(),
                cc_cursor: 0,
                active_notes: Vec::new(),
                ended_notes: Vec::new(),
                model: None,
                skip_track: Vec::new(),
                pending_play_from_sample: None,
                automation_density: 1,
            }
        })
    }

    pub(crate) fn sample_position(&self) -> u64 {
        self.sample_position
    }

    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    pub(crate) fn sample_rate_hz(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    pub(crate) fn voice_count(&self) -> u64 {
        self.channel_group.voice_count()
    }

    pub(crate) fn channel_map_clone(&self) -> Box<[u32; 256]> {
        self.channel_map.clone()
    }

    pub(crate) fn active_mask(&self) -> &[bool] {
        &self.active_mask
    }

    pub(crate) fn model_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub(crate) fn set_pending_play(&mut self, from_sample: u64) {
        self.pending_play_from_sample = Some(from_sample);
    }

    pub(crate) fn send_all_notes_off(&mut self) {
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
    }

    pub(crate) fn clear_active_notes(&mut self) {
        self.active_notes.clear();
    }

    pub(crate) fn set_layer_count(&mut self, count: Option<usize>) {
        use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
        use xsynth_core::channel_group::SynthEvent;
        self.channel_group
            .send_event(SynthEvent::AllChannels(ChannelEvent::Config(
                ChannelConfigEvent::SetLayerCount(count),
            )));
    }

    pub(crate) fn handle_command(&mut self, cmd: AudioCommand) {
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
            AudioCommand::LoadModel { model } => {
                self.playing = false;
                self.load_model(&model);
            }
            AudioCommand::ReloadNotes { model } => {
                self.send_all_notes_off();
                self.active_notes.clear();
                self.load_model(&model);
            }
            AudioCommand::LoadSoundFont { port, paths } => {
                self.load_soundfont_for_port(port, &paths);
            }
            AudioCommand::SkipTracks { skip } => {
                self.skip_track = skip;
            }
            AudioCommand::SetLayerCount { count } => {
                self.set_layer_count(count);
            }
            AudioCommand::SetAutomationDensity { density } => {
                self.automation_density = density.max(1);
            }
        }
    }

    pub(crate) fn load_soundfont_paths(
        sample_rate: u32,
        paths: &[String],
    ) -> Result<Vec<Arc<dyn SoundfontBase>>, String> {
        SoundFontManager::new(sample_rate).load_paths(paths)
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;