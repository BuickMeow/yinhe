use std::collections::BinaryHeap;
use std::cmp::Reverse;
use std::sync::Arc;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::{
    ChannelGroup, ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat,
};
use xsynth_core::soundfont::SoundfontBase;
use xsynth_core::{AudioStreamParams, ChannelCount};

use yinhe_core::YinModel;

use crate::audio_model::{ActiveNote, AudioModel, AudibleNote, SortedCC};
use crate::channel_layout::ChannelLayout;
use crate::soundfont::SoundFontManager;
use crate::spawn::AudioCommand;

/// Core MIDI synthesis engine.  Owned by the renderer thread.
pub(crate) struct AudioEngine {
    pub(crate) channel_group: ChannelGroup,
    /// 不可变通道布局：active_mask + channel_map + num_channels。
    /// 创建后定型，若 model 结构变化必须 teardown + 重建引擎。
    pub(crate) channel_layout: ChannelLayout,
    pub(crate) sf_manager: SoundFontManager,
    pub(crate) sample_rate: u32,
    pub(crate) sample_position: u64,
    pub(crate) playing: bool,
    pub(crate) duration_samples: u64,

    pub(crate) note_cursor: [usize; 128],
    /// Reference to the full YinModel. 保留供 GPU 路径和 PrepareModel 命令合并使用；
    /// 音频 dispatch/seek 改读 `audible_notes`（已过滤 vel≤1 + tick→sample 预转换）。
    pub(crate) yin_model: Option<Arc<YinModel>>,
    /// 128 个 key 桶的可听音事件（vel > 1），由 worker 线程预构建。
    /// 音频线程的 seek / dispatch 只读这份列表。
    pub(crate) audible_notes: Box<[Vec<AudibleNote>; 128]>,

    /// `Arc` 共享给 worker 线程做 chase 计算，避免每次 Seek clone 几十万条 CC。
    pub(crate) cc_events: Arc<Vec<SortedCC>>,
    pub(crate) cc_cursor: usize,
    /// min-heap by end_sample：堆顶是最早结束的音符。
    /// NoteOff 检测从 O(V) retain 全扫降到 O(ended × log V) 逐个 pop。
    pub(crate) active_notes: BinaryHeap<Reverse<ActiveNote>>,
    pub(crate) ended_notes: Vec<ActiveNote>,
    pub(crate) model: Option<AudioModel>,
    pub(crate) skip_track: Vec<bool>,
    /// Set when Play arrives during async model loading.
    pub(crate) pending_play_from_sample: Option<u64>,
    /// Linear/Curve 自动化段播放时的中间事件 tick 间隔（默认 1）。
    pub(crate) automation_density: u32,
    /// 每次 `apply_prepared_model` / `load_model` 替换 cc_events 时 `+1`。
    /// renderer 发 `PrepareChase` 时带上当前 generation，worker 回传的
    /// `ChaseResult` 也带 generation，renderer 据此丢弃过期的 chase 结果
    ///（cc_events 已被新 PrepareModel 替换的旧结果）。
    pub(crate) chase_generation: u64,

    /// GPU 合成器 — 启用后渲染走 GpuSynth 而非 xsynth
    #[cfg(feature = "gpu")]
    pub(crate) gpu_synth: Option<yinhe_synth::GpuSynth>,
}

impl AudioEngine {
    pub(crate) fn new(sample_rate: u32, layout: ChannelLayout) -> Self {
        Self::with_parallelism(sample_rate, layout, ParallelismOptions::AUTO_PER_CHANNEL)
    }

    pub(crate) fn with_parallelism(
        sample_rate: u32,
        layout: ChannelLayout,
        parallelism: ParallelismOptions,
    ) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
            let compacted_channels = layout.compacted_channels();

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
                parallelism,
            };

            Self {
                channel_group: ChannelGroup::new(config),
                channel_layout: layout,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                playing: false,
                duration_samples: 0,
                note_cursor: [0; 128],
                yin_model: None,
                audible_notes: Box::new(core::array::from_fn(|_| Vec::new())),
                cc_events: Arc::new(Vec::new()),
                cc_cursor: 0,
                active_notes: BinaryHeap::new(),
                ended_notes: Vec::new(),
                model: None,
                skip_track: Vec::new(),
                pending_play_from_sample: None,
                automation_density: 1,
                chase_generation: 0,
                #[cfg(feature = "gpu")]
                gpu_synth: None,
            }
        })
    }

    pub(crate) fn sample_position(&self) -> u64 {
        self.sample_position
    }

    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    pub(crate) fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    pub(crate) fn voice_count(&self) -> u64 {
        self.channel_group.voice_count()
    }

    pub(crate) fn channel_layout(&self) -> &ChannelLayout {
        &self.channel_layout
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
            AudioCommand::UpdateNotes { model } => {
                // 只更新音符，不重建 cc_events，不 chase。
                // `apply_notes_only` 由 renderer 在收到 `PreparedNotes` 时调用，
                // 这里只在直接 handle_command 时退化成 load_model（测试路径）。
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