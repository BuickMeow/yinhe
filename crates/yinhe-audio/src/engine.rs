use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::Mutex;

use xsynth_core::channel::ChannelInitOptions;
use xsynth_core::channel_group::{ChannelGroupConfig, ParallelismOptions, SynthFormat};
use xsynth_core::soundfont::SoundfontBase;
use xsynth_core::{AudioStreamParams, ChannelCount};

use yinhe_core::YinModel;
use yinhe_mixer::{InsertProcessor, InstrumentProcessor, MixerGraph, MixerParams};
use yinhe_types::{Interpolation, KEY_COUNT};

use crate::audio_model::{ActiveNote, AudioModel, SortedCC};
use crate::channel::ChaseSkip;
use crate::channel_layout::ChannelLayout;
use crate::channel_set::ChannelSet;
use crate::soundfont::SoundFontManager;
use crate::spawn::AudioCommand;

/// 引擎渲染块长（帧）。实时渲染块（audio_renderer 的 scratch）与之对齐；
/// 导出用更大的块，首次 render 时 mixer/暂存按实际块长 resize（每引擎至多一次）。
pub(crate) const ENGINE_BLOCK_FRAMES: usize = 512;

/// 引擎可能请求的最大渲染块长（帧）。
///
/// **插件激活契约**：所有 insert/乐器插件必须按此能力激活（`yinhe-egui` 的
/// `activate` 调用），否则插件内部按小块分配的缓冲会被更大的块越界
/// （此前的 4096 大块曾让 512 激活的插件每块只处理前 512 帧）。
/// 引擎任何路径（实时/导出）的块长都不得超过该值。
pub const MAX_ENGINE_BLOCK_FRAMES: usize = 4096;

/// yinhe-synth CPU 后端槽位（后端选择由 `SynthEngine` 在 spawn 时决定）。
pub(crate) type CpuSynthSlot = Option<yinhe_synth::CpuSynth>;

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
    pub(crate) instrument_returns: Vec<(u64, Box<dyn InstrumentProcessor>)>,
    /// 乐器实例，长度 = `compacted_channels`，只有乐器 dense 槽位非空。
    /// 索引 = 全局 dense（= midi_compacted + 乐器通道排序位置）。
    pub(crate) instruments: Vec<Option<crate::instrument::InstrumentSource>>,
    /// 乐器求和用的临时缓冲：链内第 2 个及以后的乐器输出先写这里再累加。按需增长。
    pub(crate) scratch_l: Vec<f32>,
    pub(crate) scratch_r: Vec<f32>,
    /// 乐器事件合并用的临时缓冲（广播事件 + 定向参数事件）。每块复用。
    pub(crate) event_scratch: Vec<yinhe_mixer::PluginEvent>,
    /// 内置音源通道处理段（CC7/10/11/71/74），索引 = dense 通道。
    /// 挂插件乐器的通道不处理（CC 透传插件）。见 `channel_dsp` 模块文档。
    pub(crate) channel_dsp: Vec<crate::channel_dsp::ChannelDspChain>,
    /// 当前渲染块的起始 sample：dispatch 时把 tick 域事件换算成块内 frame offset
    /// 喂乐器插件（`PluginEvent::time`）。render() 每块开头设置。
    pub(crate) block_start_sample: u64,
    /// 不可变通道布局：active_mask + channel_map + num_channels。
    /// 创建后定型，若 model 结构变化必须 teardown + 重建引擎。
    pub(crate) channel_layout: ChannelLayout,
    pub(crate) sf_manager: SoundFontManager,
    pub(crate) sample_rate: u32,
    pub(crate) sample_position: u64,
    /// 循环回绕累计的绝对位置偏移：`绝对 sample = 逻辑 sample + offset`。
    /// UI 侧 cpal 回调用 `consumer_abs - offset` 得到逻辑播放位置，循环回绕
    /// 后播放头才不会永久领先/落后一个 ring。seek/stop 时归零。
    pub(crate) loop_wrap_offset: u64,
    /// 当前 per-key layer 设置（`SetLayerCount` 保存；worker 创建 yinhe
    /// 合成器后据此应用——创建晚于命令，不能只在线应用）。
    pub(crate) layer_count: Option<usize>,
    /// 忽略力度 ≤ 该值的音符（0 = 不忽略）。dispatch 时逐音符实时判断，
    /// 改阈值不需要重建任何缓存。
    pub(crate) ignore_velocity: u8,
    /// 最大复音数（`SetMaxVoices` 保存；worker 创建合成器后补应用）。
    pub(crate) max_voices: usize,
    /// dispatch 基准（tick 域）：与 sample_position 同步推进（每块末更新，
    /// seek/load 时由 sample→tick 初始化）。事件比较全在 tick 域。
    pub(crate) current_tick: u32,
    pub(crate) playing: bool,
    /// 暂停/停止时的**输入侧**淡出：剩余帧数（0 = 无）。
    /// 淡的是喂给 insert 链之前的干声，效果器尾音不会被一起淡掉。
    pub(crate) pause_fade_remaining: usize,
    /// 本次淡出的总帧数（用于计算线性增益）。
    pub(crate) pause_fade_total: usize,
    pub(crate) duration_samples: u64,

    /// 每 key 桶的 dispatch 游标（直接读模型音符，不再维护平铺副本）。
    pub(crate) note_cursor: [yinhe_types::NoteCursor; KEY_COUNT],
    /// Reference to the full YinModel：dispatch/seek 直接从 `notes[key]` 顺序读，
    /// 力度过滤与 0 BPM 剪切在派发时逐音符现算。
    pub(crate) yin_model: Option<Arc<YinModel>>,

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
    /// 每轨电平读数端（峰值，0..∞）：渲染线程每块写、UI 读。
    pub(crate) track_levels: Arc<Mutex<Vec<f32>>>,
    pub(crate) model: Option<AudioModel>,
    pub(crate) skip_track: Vec<bool>,
    /// 已解码音频素材（uuid → PCM + 峰值）。由 UI 后台解码后经
    /// `AudioCommand::SetAudioSource` 推入；素材池只增不减（换工程时随引擎重建清空）。
    /// 音频轨渲染按片段的 `source` uuid 在此查找。
    pub(crate) audio_sources:
        std::collections::HashMap<String, Arc<crate::audio_source::DecodedAudio>>,
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

    /// 插件预览的活动音符：到期/停止时向对应乐器通道发 NoteOff。
    /// 与 xsynth 预览（PreviewEngine）并列；`off_sample = None` 表示持续音。
    pub(crate) plugin_previews: Vec<PluginPreviewNote>,

    /// 音符试听预览合成器（独立合成器实例，不经主引擎 voice）。
    /// 预览音频累加进 mixer 通道缓冲，与播放共用同一条通道 DSP + 混音台链路，
    /// 因此试听反映**当前**的 CC7/10/11/71/74 与推子/master/insert 状态。
    /// 由 `init_engine` 在创建引擎后按后端注入（测试可不设置）。
    pub(crate) preview: Option<crate::preview_engine::PreviewEngine>,

    /// yinhe-synth CPU 合成器 — 启用后渲染走 CpuSynth 而非 xsynth；
    /// 未创建时走 xsynth ChannelSet。
    pub(crate) cpu_synth: CpuSynthSlot,
    /// 当前 dispatch 段起点的绝对 sample：CpuSynth 的事件按 sample 生效，
    /// dispatch 用它作为事件的 sample 依据（每段渲染前由 render 循环更新）。
    pub(crate) segment_start_sample: u64,
    /// 导出模式：忽略走带循环（导出必须线性跑完整时间轴）。
    pub(crate) export_mode: bool,
    /// 走带循环开关（tick 域；`loop_end_tick <= loop_start_tick` 视为关闭）。
    pub(crate) loop_enabled: bool,
    pub(crate) loop_start_tick: u32,
    pub(crate) loop_end_tick: u32,
    /// 循环起点 chase 快照：由 renderer 在 loop 变化/模型变化后异步预计算，
    /// 回绕时在渲染线程同步应用（音频线程不查询事件流）。
    pub(crate) loop_chase: Option<LoopChase>,
    /// 本块内发生循环回绕的输出帧偏移（渲染后统一做去咔哒淡出/淡入）。
    pub(crate) loop_wrap_offsets: Vec<usize>,
    /// 跨块延续的循环回绕淡入剩余帧数（回绕点正好落在块尾时用）。
    pub(crate) loop_fade_in_pending: usize,
}

/// 循环回绕用的控制状态快照。
pub(crate) struct LoopChase {
    pub generation: u64,
    pub start_tick: u32,
    pub states: Box<[Option<crate::channel::ChannelState>; 256]>,
    pub plugin_params: Vec<(u8, u64, u32, f32)>,
}

/// 插件预览的一条活动音符（引擎侧调度 NoteOn/NoteOff）。
pub(crate) struct PluginPreviewNote {
    /// 乐器 dense 通道索引。
    pub dense: usize,
    /// 通道内 MIDI 通道（低 4 位）。
    pub midi_channel: u8,
    pub key: u8,
    /// 1 ~ 127（发送时转 0..1）。
    pub velocity: u8,
    /// 触发位置（采样；组内按 target_tick 差错开）。
    pub on_sample: u64,
    /// None = 持续音（等 `PreviewInstrumentStop`）。
    pub off_sample: Option<u64>,
    /// 是否已发 NoteOn（未触发的音符在新组到来时被替换）。
    pub triggered: bool,
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
                scratch_l: Vec::new(),
                scratch_r: Vec::new(),
                event_scratch: Vec::new(),
                channel_dsp: (0..compacted)
                    .map(|_| crate::channel_dsp::ChannelDspChain::new(sample_rate))
                    .collect(),
                block_start_sample: 0,
                channel_layout: layout,
                sf_manager: SoundFontManager::new(sample_rate),
                sample_rate,
                sample_position: 0,
                loop_wrap_offset: 0,
                layer_count: Some(4),
                ignore_velocity: 1,
                max_voices: crate::DEFAULT_MAX_VOICES,
                current_tick: 0,
                playing: false,
                pause_fade_remaining: 0,
                pause_fade_total: 0,
                duration_samples: 0,
                note_cursor: [yinhe_types::NoteCursor::default(); KEY_COUNT],
                yin_model: None,
                cc_events: Arc::new(Vec::new()),
                cc_cursor: 0,
                dispatched_skip: ChaseSkip::default(),
                active_notes: BinaryHeap::new(),
                ended_notes: Vec::new(),
                track_levels: Arc::new(Mutex::new(Vec::new())),
                model: None,
                skip_track: Vec::new(),
                audio_sources: std::collections::HashMap::new(),
                am_ms: Arc::new(crate::spawn::AmMsMap::new()),
                am_lane_skip: Vec::new(),
                pending_play_from_sample: None,
                automation_density: 1,
                chase_generation: 0,
                plugin_previews: Vec::new(),
                preview: None,
                cpu_synth: None,
                segment_start_sample: 0,
                export_mode: false,
                loop_enabled: false,
                loop_start_tick: 0,
                loop_end_tick: 0,
                loop_chase: None,
                loop_wrap_offsets: Vec::new(),
                loop_fade_in_pending: 0,
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

    /// 设置暂停/停止前的输入侧淡出帧数（由渲染线程在 pause 前调用）。
    pub(crate) fn set_pause_fade_out(&mut self, frames: usize) {
        self.pause_fade_remaining = frames;
        self.pause_fade_total = frames.max(1);
    }

    /// 每轨电平表共享读数端（句柄 clone 用）。
    pub(crate) fn track_levels(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.track_levels)
    }

    /// 发布每轨电平（渲染线程每块调用）。
    ///
    /// YinheCpu：直接读合成器的每轨峰值（该轨全部 voice 的实时合值）。
    /// xsynth 回退后端没有轨道级信号，用该轨所属通道的混音台电平近似；
    /// 未播放或被 mute 的轨 → 0。
    pub(crate) fn publish_track_levels(&self) {
        let Ok(mut g) = self.track_levels.lock() else {
            return;
        };
        let Some(model) = self.model.as_ref() else {
            g.clear();
            return;
        };
        let n = model.track_channels.len();
        if g.len() != n {
            g.resize(n, 0.0);
        }
        if !self.playing {
            g.fill(0.0);
            return;
        }
        for (i, slot) in g.iter_mut().enumerate() {
            if self.skip_track.get(i).copied().unwrap_or(false) {
                *slot = 0.0;
                continue;
            }
            *slot = if let Some(cs) = self.cpu_synth.as_ref() {
                cs.track_peak(i)
            } else {
                let dense = self
                    .channel_layout
                    .dense_for(model.track_channel(i) as usize);
                if dense == u32::MAX {
                    0.0
                } else {
                    self.mixer
                        .channel_meter_reading(dense as usize)
                        .map(|m| {
                            let (l, r) = m.read();
                            l.max(r)
                        })
                        .unwrap_or(0.0)
                }
            };
        }
    }

    pub(crate) fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    /// 当前活跃 voice 数（当前合成后端）。
    /// 导出尾音判定用它（`export.rs`）：后端切换后语义保持不变。
    pub(crate) fn voice_count(&self) -> u64 {
        if let Some(synth) = &self.cpu_synth {
            return synth.voice_count() as u64;
        }
        self.channel_set.voice_count()
    }

    /// xsynth `ChannelSet` 当前是否参与渲染（yinhe CPU 后端启用时不参与）。
    #[inline]
    pub(crate) fn cpu_synth_active(&self) -> bool {
        !self.yinhe_cpu_active()
    }

    /// yinhe-synth CPU 后端是否启用（`CpuSynth`）。
    #[inline]
    pub(crate) fn yinhe_cpu_active(&self) -> bool {
        self.cpu_synth.is_some()
    }

    pub(crate) fn model_loaded(&self) -> bool {
        self.model.is_some()
    }

    /// 是否有插件（insert/乐器）参与当前渲染。
    /// 插件的尾音无法像 xsynth 那样用 voice_count 探测，导出尾音阶段据此
    /// 决定是否渲染满上限。
    pub(crate) fn has_active_plugins(&self) -> bool {
        self.mixer.has_inserts() || self.instruments.iter().any(|i| i.is_some())
    }

    /// 最大复音数：0/None → 系统推荐（`DEFAULT_MAX_VOICES`）。合成器
    /// 创建晚于命令，worker 创建后会补应用本字段。
    pub(crate) fn set_max_voices(&mut self, max: Option<usize>) {
        self.max_voices = max.unwrap_or(crate::DEFAULT_MAX_VOICES);
        if let Some(cs) = self.cpu_synth.as_mut() {
            cs.set_max_voices(self.max_voices);
        }
    }

    pub(crate) fn set_pending_play(&mut self, from_sample: u64) {
        self.pending_play_from_sample = Some(from_sample);
    }

    pub(crate) fn set_layer_count(&mut self, count: Option<usize>) {
        // 保存设置：yinhe CPU 合成器在 worker 里**异步创建**（音色库加载
        // 完成时），创建时刻晚于本命令 → 必须在创建后重新应用（否则设置被
        // 静默丢弃、合成器永远用默认 layer=4；xsynth 因内部立即应用而无此问题）。
        self.layer_count = count;
        use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
        use xsynth_core::channel_group::SynthEvent;
        self.channel_set
            .send_event(SynthEvent::AllChannels(ChannelEvent::Config(
                ChannelConfigEvent::SetLayerCount(count),
            )));
        // yinhe CPU 后端的 per-key layer 语义（对齐 xsynth）
        if let Some(cs) = self.cpu_synth.as_mut() {
            cs.set_layer_count(count);
        }
    }

    pub(crate) fn handle_command(&mut self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::Play { from_sample } => {
                self.seek_to(from_sample);
                self.playing = true;
            }
            AudioCommand::Resume => self.resume(),
            AudioCommand::Pause => self.pause(),
            AudioCommand::Stop => {
                self.playing = false;
                self.seek_to(0);
            }
            AudioCommand::StopTransport => self.stop_transport(),
            AudioCommand::Seek { sample } => self.seek_to(sample),
            AudioCommand::SetLoop {
                enabled,
                start_tick,
                end_tick,
            } => self.set_loop(enabled, start_tick, end_tick),
            AudioCommand::LoadModel { model } => {
                self.playing = false;
                self.load_model(&model);
            }
            AudioCommand::ReloadNotes { .. } => {
                // renderer 在命令层处理（pending_reload → PrepareModel 全量重建），
                // 引擎不接受该命令——旧版直接 load_model 的路径已废弃，
                // 避免绕过 renderer 的掩码/位置管理造成语义分裂。
            }
            AudioCommand::UpdateNotes { model } => {
                // 只更新音符，不重建 cc_events，不 chase。
                // `apply_notes_only` 由 renderer 在收到 `PreparedNotes` 时调用，
                // 这里只在直接 handle_command 时退化成 load_model（测试路径）。
                self.load_model(&model);
            }
            AudioCommand::SetSynthParams { channel, params } => {
                self.apply_synth_params(channel, params);
            }
            AudioCommand::SetSoundFonts { configs } => {
                for (channel, paths) in configs.iter() {
                    self.load_soundfont_for_channel(*channel, paths);
                }
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
            AudioCommand::SetIgnoreVelocity { threshold } => {
                self.ignore_velocity = threshold;
            }
            AudioCommand::SetMaxVoices { max } => {
                self.set_max_voices(max);
            }
            AudioCommand::SetAutomationDensity { density } => {
                self.automation_density = density.max(1);
            }
            AudioCommand::SetMixerParams { params } => self.set_mixer_params(*params),
            AudioCommand::SetChannelStrip { channel, params } => {
                self.set_channel_strip(channel, params);
            }
            AudioCommand::SetAudioStrip { channel, params } => {
                self.set_audio_strip(channel, params);
            }
            AudioCommand::SetMasterParams { params } => self.set_master_params(params),
            AudioCommand::InsertAdd {
                target,
                slot,
                processor,
            } => self.insert_add(target, slot, processor),
            AudioCommand::InsertRemove { target, slot } => self.insert_remove(target, slot),
            AudioCommand::InsertReplace {
                target,
                slot,
                processor,
            } => self.insert_replace(target, slot, processor),
            AudioCommand::SetBusStrip { bus, params } => self.set_bus_strip(bus, params),
            AudioCommand::SyncBusConfig {
                buses,
                sends,
                bus_sends,
                audio_sends,
            } => {
                self.sync_bus_config(*buses, *sends, *bus_sends, *audio_sends);
            }
            AudioCommand::SetInstrumentSlot {
                channel,
                slot_id,
                processor,
            } => self.set_instrument_slot(channel, slot_id, processor),
            AudioCommand::SetAudioSource { uuid, decoded } => {
                self.audio_sources.insert(uuid, decoded);
            }
            AudioCommand::RefreshLatency => self.refresh_latency(),
            // 预览命令由渲染器处理（独立预览合成器 + 渲染时钟），引擎层忽略。
            AudioCommand::PreviewNotes { .. }
            | AudioCommand::PreviewStop
            | AudioCommand::PreviewStopKey { .. } => {}
            // 插件预览：引擎直接调度（乐器通道 NoteOn/NoteOff，停止状态空闲渲染出声）。
            AudioCommand::PreviewInstrumentNotes {
                channel,
                notes,
                exclusive,
            } => {
                self.preview_instrument_notes(channel, notes, exclusive);
            }
            AudioCommand::PreviewInstrumentStop { channel, key } => {
                self.preview_instrument_stop(channel, key);
            }
            // 导出由渲染器执行（导出模式状态机 + WAV 写盘），引擎层忽略。
            AudioCommand::ExportStart { .. } => {}
        }
    }

    /// 采样插值方式（须在加载音色库之前设置；运行中切换不追溯已加载的
    /// 音色库，重建音频/重新加载后生效）。
    pub(crate) fn set_interpolation(&mut self, interpolation: Interpolation) {
        self.sf_manager.set_interpolation(interpolation);
    }

    pub(crate) fn load_soundfont_paths(
        sample_rate: u32,
        paths: &[String],
        interpolation: Interpolation,
    ) -> Result<Vec<Arc<dyn SoundfontBase>>, String> {
        let mut manager = SoundFontManager::new(sample_rate);
        manager.set_interpolation(interpolation);
        manager.load_paths(paths)
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
