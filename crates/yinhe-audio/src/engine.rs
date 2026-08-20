use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::{ChannelGroupConfig, ParallelismOptions, SynthEvent, SynthFormat};
use xsynth_core::soundfont::SoundfontBase;
use xsynth_core::{AudioStreamParams, ChannelCount};

use yinhe_core::YinModel;
use yinhe_mixer::{InsertProcessor, MixerGraph, MixerParams};
use yinhe_types::KEY_COUNT;

use crate::audio_model::{ActiveNote, AudibleNote, AudioModel, SortedCC};
use crate::channel::ChaseSkip;
use crate::channel_layout::ChannelLayout;
use crate::channel_set::ChannelSet;
use crate::soundfont::SoundFontManager;
use crate::spawn::AudioCommand;

/// 引擎渲染块长（帧）。实时渲染块（audio_renderer 的 scratch）与之对齐；
/// 导出用更大的块，首次 render 时 mixer/暂存按实际块长 resize（每引擎至多一次）。
pub(crate) const ENGINE_BLOCK_FRAMES: usize = 512;

/// Core MIDI synthesis engine.  Owned by the renderer thread.
pub(crate) struct AudioEngine {
    pub(crate) channel_set: ChannelSet,
    /// 混音处理图：通道数 = compacted 通道数，dense 索引与 channel_set 对齐。
    pub(crate) mixer: MixerGraph,
    /// 混音台持久化参数的引擎侧副本（按源通道索引）：resize 重建 strip 用，
    /// 由 SetMixerParams / SetChannelStrip / SetMasterParams 命令维护。
    pub(crate) mixer_params: MixerParams,
    /// 被替换/移除的 insert 处理器：渲染线程不能 deactivate 插件，
    /// 攒在这里由 renderer 送回 UI 线程回收。
    pub(crate) insert_returns: Vec<Box<dyn InsertProcessor>>,
    /// 被替换/移除的乐器处理器：同样不能在渲染线程 deactivate，攒在
    /// 这里由 renderer 送回 UI 线程回收（与 insert 同通道回流）。
    pub(crate) instrument_returns: Vec<(u16, yinhe_clap::ClapProcessor)>,
    /// CLAP 乐器实例，长度 = `compacted_channels`，只有乐器 dense 槽位非空。
    /// 索引 = 全局 dense（= midi_compacted + 乐器通道排序位置）。
    pub(crate) instruments: Vec<Option<crate::instrument::InstrumentSource>>,
    /// 当前渲染块的起始 sample：dispatch 时把 tick 域事件换算成块内 frame offset
    /// 喂 CLAP 乐器（`ClapInputEvent::time`）。render() 每块开头设置。
    pub(crate) block_start_sample: u64,
    /// 不可变通道布局：active_mask + channel_map + num_channels。
    /// 创建后定型，若 model 结构变化必须 teardown + 重建引擎。
    pub(crate) channel_layout: ChannelLayout,
    pub(crate) sf_manager: SoundFontManager,
    pub(crate) sample_rate: u32,
    pub(crate) sample_position: u64,
    /// dispatch 基准（tick 域）：与 sample_position 同步推进（每块末更新，
    /// seek/load 时由 sample→tick 初始化）。事件比较全在 tick 域。
    pub(crate) current_tick: u32,
    pub(crate) playing: bool,
    pub(crate) duration_samples: u64,

    pub(crate) note_cursor: [usize; KEY_COUNT],
    /// Reference to the full YinModel. 保留供 GPU 路径和 PrepareModel 命令合并使用；
    /// 音频 dispatch/seek 改读 `audible_notes`（已过滤 vel≤1 + tick→sample 预转换）。
    pub(crate) yin_model: Option<Arc<YinModel>>,
    /// KEY_COUNT 个 key 桶的可听音事件（vel > 1），由 worker 线程预构建。
    /// 音频线程的 seek / dispatch 只读这份列表。
    pub(crate) audible_notes: Box<[Vec<AudibleNote>; KEY_COUNT]>,

    /// `Arc` 共享给 worker 线程做 chase 计算，避免每次 Seek clone 几十万条 CC。
    pub(crate) cc_events: Arc<Vec<SortedCC>>,
    pub(crate) cc_cursor: usize,
    /// 自最近一次 `seek_to` 以来**实际发送**给合成器的控制器打点。
    /// dispatch 每真实发送一个 CC/PB/RPN/PC 事件就 mark，`seek_to` 清零。
    /// `apply_chase_result` 据此跳过这些控制器，避免旧 chase 值覆盖已实发的新值。
    /// 与旧实现（扫描 `[chase_cc_base, cc_cursor)` 区间）的区别：mute 期间被
    /// cc_cursor 越过但未发送的事件不会被误标，unmute 后 chase 能正确恢复。
    pub(crate) dispatched_skip: ChaseSkip,
    /// min-heap by end_sample：堆顶是最早结束的音符。
    /// NoteOff 检测从 O(V) retain 全扫降到 O(ended × log V) 逐个 pop。
    pub(crate) active_notes: BinaryHeap<Reverse<ActiveNote>>,
    pub(crate) ended_notes: Vec<ActiveNote>,
    pub(crate) model: Option<AudioModel>,
    pub(crate) skip_track: Vec<bool>,
    /// AR 自动化 lane 的 M/S 试听旁通集：dispatch 用它预计算 `am_lane_skip`
    ///（chase 计算在 worker 侧直接查本 map）。随 `SetAmMs` 命令更新。
    pub(crate) am_ms: Arc<crate::spawn::AmMsMap>,
    /// 每条音轨的 lane 跳过掩码：`mask[track][lane_idx] = true` 表示 dispatch 时
    /// 跳过该 lane 的自动化事件（AM M/S 动态旁通，与 `skip_track` 并列）。
    /// 加载模型 / `SetAmMs` 时重建（O(轨道 × lane 数)）。
    pub(crate) am_lane_skip: Vec<Vec<bool>>,
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

            let compacted = layout.compacted_channels() as usize;
            let mut mixer = MixerGraph::new(ENGINE_BLOCK_FRAMES);
            mixer.resize(compacted, ENGINE_BLOCK_FRAMES, &[]);
            Self {
                channel_set: ChannelSet::new(config, ENGINE_BLOCK_FRAMES),
                mixer,
                mixer_params: MixerParams::default(),
                insert_returns: Vec::new(),
                instrument_returns: Vec::new(),
                instruments: (0..compacted).map(|_| None).collect(),
                block_start_sample: 0,
                channel_layout: layout,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                current_tick: 0,
                playing: false,
                duration_samples: 0,
                note_cursor: [0; KEY_COUNT],
                yin_model: None,
                audible_notes: Box::new(core::array::from_fn(|_| Vec::new())),
                cc_events: Arc::new(Vec::new()),
                cc_cursor: 0,
                dispatched_skip: ChaseSkip::default(),
                active_notes: BinaryHeap::new(),
                ended_notes: Vec::new(),
                model: None,
                skip_track: Vec::new(),
                am_ms: Arc::new(crate::spawn::AmMsMap::new()),
                am_lane_skip: Vec::new(),
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

    /// 当前 dispatch 基准 tick（renderer 发 PrepareChase / 预览时使用）。
    pub(crate) fn current_tick(&self) -> u32 {
        self.current_tick
    }

    /// tick→sample（渲染段边界用）。tempo 信息来自 yin_model。
    pub(crate) fn tick_to_sample(&self, tick: u32) -> u64 {
        let Some(m) = &self.yin_model else { return 0 };
        crate::audio_model::tick_to_sample(
            tick,
            &m.tempo_map.tempo_segments,
            m.tempo_map.ticks_per_beat,
            self.sample_rate as f64,
        )
    }

    /// sample→tick（块边界/seek 基准）。返回满足 `tick_to_sample(t) <= sample`
    /// 的最大 t（浮点误差已校验修正，见 `sample_to_tick`）。
    pub(crate) fn sample_to_tick(&self, sample: u64) -> u32 {
        let Some(m) = &self.yin_model else { return 0 };
        crate::audio_model::sample_to_tick(
            sample,
            &m.tempo_map.tempo_segments,
            m.tempo_map.ticks_per_beat,
            self.sample_rate as f64,
        )
    }

    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    pub(crate) fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    pub(crate) fn voice_count(&self) -> u64 {
        self.channel_set.voice_count()
    }

    pub(crate) fn model_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub(crate) fn set_pending_play(&mut self, from_sample: u64) {
        self.pending_play_from_sample = Some(from_sample);
    }

    pub(crate) fn send_all_notes_off(&mut self) {
        self.channel_set
            .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                ChannelAudioEvent::AllNotesOff,
            )));
    }

    pub(crate) fn set_layer_count(&mut self, count: Option<usize>) {
        use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
        use xsynth_core::channel_group::SynthEvent;
        self.channel_set
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
                // am_ms 掩码在 renderer 侧随 SetAmMs 命令维护（engine 只做模型重载）。
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
            AudioCommand::SetAmMs { am_ms } => {
                // renderer 在命令层已处理（更新掩码 + 异步 chase）；
                // 这里兜底同步本字段，保证直接 handle_command（如测试/未来直连）语义一致。
                self.am_ms = am_ms;
                if let Some(model) = self.yin_model.clone() {
                    self.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &self.am_ms);
                }
            }
            AudioCommand::SetLayerCount { count } => {
                self.set_layer_count(count);
            }
            AudioCommand::SetAutomationDensity { density } => {
                self.automation_density = density.max(1);
            }
            AudioCommand::SetMixerParams { params } => self.set_mixer_params(*params),
            AudioCommand::SetChannelStrip { channel, params } => {
                self.set_channel_strip(channel, params);
            }
            AudioCommand::SetMasterParams { params } => self.set_master_params(params),
            AudioCommand::InsertAdd {
                channel,
                slot,
                processor,
            } => self.insert_add(channel, slot, processor),
            AudioCommand::InsertRemove { channel, slot } => self.insert_remove(channel, slot),
            AudioCommand::InsertReplace {
                channel,
                slot,
                processor,
            } => self.insert_replace(channel, slot, processor),
            AudioCommand::SetInstrument { channel, processor } => {
                self.set_instrument(channel, processor.map(|p| *p))
            }
            // 预览命令由渲染器处理（独立预览合成器 + 渲染时钟），引擎层忽略。
            AudioCommand::PreviewNotes { .. } | AudioCommand::PreviewStop => {}
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
