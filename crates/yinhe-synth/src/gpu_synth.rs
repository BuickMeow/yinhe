//! GPU 合成器高层封装 — 统一播放和导出接口。
//!
//! 和 xsynth 的 ChannelGroup 对等，但**只做音源层**：
//! - `note_on` / `note_off` / 控制事件接收（音量/声像/滤波等 DSP CC 由
//!   yinhe-dsp 效果器处理，本合成器忽略；见 `docs/spec-yinhe-dsp.md`）
//! - 32 通道 MIDI 状态机（pitch bend/RPN 调音、damper、ADSR CC、bank/program）
//! - `render` 一次性渲染整个 block（输出无限幅：限幅由调用方统一处理）
//! - `load_events` 批量加载预排序事件列表（用于导出/Seek）
//!
//! voice 管理、通道状态、ADSR 推进封装在内部。

use std::collections::HashMap;
use std::sync::Arc;

use crate::sf_parser;
use crate::synth::GpuAudioRenderer;
use crate::synth::buffers::MAX_VOICE_SLOTS;
use crate::synth::{GpuVoiceState, RENDER_SEGMENT_FRAMES, RenderSegment};
use crate::wgpu;

use crate::channel_state::ChannelState;
pub use crate::channel_state::ChaseSkip;

mod schedule;

use schedule::{SegBuffers, Voice};

pub use crate::sf_cache::prefetch_key_maps;
use crate::sf_cache::{
    SampleBundle, cached_sample_bundle, load_key_maps_merged, store_sample_bundle,
};

/// MIDI 通道数（dense 通道 = port×16+ch，支持 2 端口 32 通道）。
pub use crate::channel_state::MAX_CHANNELS;

/// 合成器事件（sample 域，按 sample 排序后由 `load_events` 加载）。
#[derive(Clone, Copy, Debug)]
pub enum SynthEvent {
    /// NoteOn 携带音符结束时间 `end_sample`：voice 到期自行 release。
    /// 引擎的事件表**不再生成 NoteOff**（分页装载时 NoteOff 的时间戳跨窗口会
    /// 破坏事件顺序；自带 end 后事件量也减半）。
    NoteOn {
        sample: u64,
        channel: u8,
        key: u8,
        velocity: u8,
        end_sample: u64,
    },
    /// 立即释放该 (channel, key) 最老的未释放 voice。
    /// 引擎事件表不使用（NoteOn 自带 end 取代）；保留给实时 MIDI 输入/提前释放。
    NoteOff { sample: u64, channel: u8, key: u8 },
    Control {
        sample: u64,
        channel: u8,
        event: ControlEvent,
    },
}

impl SynthEvent {
    pub fn sample(&self) -> u64 {
        match self {
            SynthEvent::NoteOn { sample, .. }
            | SynthEvent::NoteOff { sample, .. }
            | SynthEvent::Control { sample, .. } => *sample,
        }
    }
}

/// 通道控制事件（语义与 xsynth `ControlEvent` 对齐）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlEvent {
    /// 原始 MIDI CC 事件 (controller, value)。
    Raw(u8, u8),
    /// 弯音值 -1..1。
    PitchBend(f32),
    /// 弯音灵敏度（半音）。
    PitchBendSensitivity(f32),
    /// 微调（音分）。
    FineTune(f32),
    /// 粗调（半音）。
    CoarseTune(f32),
    /// 音色更换：选择该通道的音色库条目（bank, preset 语义见 `PercussionMode`）。
    ProgramChange(u8),
    /// 鼓组模式（等价 xsynth `SetPercussionMode` 配置）：true 置 bank=128，false 置 bank=0。
    /// 由 yinhe-audio 在模型加载/seek 时根据通道声明注入，后续 CC0 不能改鼓组 bank。
    PercussionMode(bool),
}

/// GPU 合成器 — 封装 GPU 渲染器 + voice 管理 + 通道状态 + 事件调度 + 限幅。
///
/// 接口设计参照 xsynth ChannelGroup：
/// - 播放时通过 `load_events` 加载预排序事件，`render` 逐块渲染
/// - 通道状态机处理 CC7/10/11/64/100/101/6/38 + pitch bend + RPN
// layer 超限淘汰的杀音计数（诊断；破坏文档列表的 doc 注释已改普通注释）。
pub static LAYER_KILLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
// 全局上限（evict_excess）的杀音计数（诊断）。
pub static EVICT_KILLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct GpuSynth {
    renderer: GpuAudioRenderer,
    /// 每 port 的音色库条目列表（bank/preset → key map），`channel_port` 决定通道用哪个 port。
    /// `Arc` 共享：单音色库时直接指向进程级解析缓存，多通道零克隆。
    port_key_maps: Vec<Arc<Vec<sf_parser::KeyMapEntry>>>,
    /// dense 通道 → port 映射（由 `load_port_soundfonts` 按 layout 填表）。
    channel_port: [u8; MAX_CHANNELS],
    /// 已加载过的音色库路径（拼接缓存的 key 组成，排序去重后使用）。
    sample_paths: Vec<std::path::PathBuf>,
    /// 采样数据在 GPU 上传块中的 (offset, len)，按 Arc 身份（指针 as usize）去重
    sample_offsets: HashMap<usize, (u32, u32)>,
    voices: Vec<Voice>,
    /// 紧凑 env_stage 读回缓冲（voice 清理用；全长缓冲复用）
    voice_stage_buf: Vec<u32>,
    /// 压缩时的全字段读回缓冲（复用，避免每块分配）
    /// 每通道上次的 pitch_multiplier（块起点比对；变化才产生段 0 的 ChState）
    channel_speed_cache: [f32; MAX_CHANNELS],
    /// 32 通道混音缓冲（GPU 输出读回，CPU 通道滤波 + 求和）
    channel_mix: Vec<f32>,
    /// 32 通道 MIDI 控制状态
    channels: [ChannelState; MAX_CHANNELS],
    /// 全局 voice 上限（黑乐谱长 sustain/无 note_off 的 voice 会累积，
    /// 超限时淘汰最老的 release 中 voice，否则最老的 active——与 xsynth voice 限制同思路）
    max_voices: usize,
    /// 每 key 同时活跃 voice 上限（`SetLayerCount`；None = 不限制）。
    /// xsynth 默认 4；超限时按 xsynth 语义杀该 key velocity 最低的 voice。
    max_layers: Option<usize>,
    /// 峰值 voice 数统计（诊断用）
    peak_voices: usize,
    sample_rate: u32,
    /// 采样插值方式（`Interpolation::code()`；加载音色库时写入 KeyInfo）。
    interpolation: u32,
    /// 排序好的事件列表（导出/Seek 用）
    events: Vec<SynthEvent>,
    event_cursor: usize,
    /// 本块内 CC 事件位置（升序，含重复 sample；collect_block 每块收集一次，
    /// 替代逐段从 event_cursor 重扫全部事件）
    cc_scratch: Vec<u64>,
    /// 分段渲染的段缓冲（复用，消除每块 4×段数 次 Vec 分配）
    seg_scratch: Vec<SegBuffers>,
    /// 当前渲染位置
    sample_position: u64,
    /// 提交游标（下一个待提交块的起点绝对 sample）。
    render_position: u64,
    /// 流水线：已提交未收割的块（FIFO）。
    pending: std::collections::VecDeque<PendingGpuBlock>,
    /// GPU 权威全字段 voice 状态（每次收割读回；compact 重传前用它覆盖
    /// CPU 镜像，否则重传过期位置/包络会把 voice 状态重置）。
    states_buf: Vec<GpuVoiceState>,
    /// 输出 ring：已渲染（GPU 读回）但未交给调用方的交错 PCM
    /// （帧 × MAX_CHANNELS × 2 f32）。预渲染的块先入 ring，调用方按所需
    /// 帧数取用——因此支持块大小变化（测试尾块/导出块）。
    ring: std::collections::VecDeque<f32>,
    /// 最近一次 seek 时的 event_cursor：`chase_skip` 用它计算"seek 后已处理
    /// 的控制事件区间"，chase 应用时跳过这些控制器（与 yinhe-audio 的
    /// `chase_cc_base` 对称，避免异步 chase 覆盖 seek 后已生效的新值）。
    chase_base: usize,
}

mod pipeline;
#[cfg(test)]
mod tests;
mod upload;

/// 已提交未收割的块。
struct PendingGpuBlock {
    readback: crate::synth::renderer::PendingReadback,
    frames: usize,
    /// 提交时的 voice 数（槽位一一对应；收割时只更新前 N 个）。
    voice_count: usize,
}

impl GpuSynth {
    /// 创建合成器（自动创建 wgpu device/queue）。音色库稍后按 port 加载。
    pub fn new_default(sample_rate: u32) -> Result<Self, String> {
        let renderer = GpuAudioRenderer::new_default()
            .map_err(|e| format!("GPU renderer init failed: {}", e))?;
        Self::from_renderer(renderer, sample_rate)
    }

    /// 创建合成器（使用指定的 wgpu device/queue）。音色库稍后按 port 加载。
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        sample_rate: u32,
    ) -> Result<Self, String> {
        let renderer = GpuAudioRenderer::new(device, queue)
            .map_err(|e| format!("GPU renderer init failed: {}", e))?;
        Self::from_renderer(renderer, sample_rate)
    }

    fn from_renderer(renderer: GpuAudioRenderer, sample_rate: u32) -> Result<Self, String> {
        Ok(Self {
            renderer,
            // 每 dense 通道一个音色库条目列表（dense = port×16+ch，最多 MAX_CHANNELS）
            port_key_maps: (0..MAX_CHANNELS).map(|_| Arc::new(Vec::new())).collect(),
            channel_port: [0; MAX_CHANNELS],
            sample_paths: Vec::new(),
            sample_offsets: HashMap::new(),
            voices: Vec::new(),
            voice_stage_buf: Vec::new(),
            channel_speed_cache: [0.0; MAX_CHANNELS],
            channel_mix: Vec::new(),
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            max_voices: crate::DEFAULT_MAX_VOICES,
            max_layers: Some(crate::DEFAULT_MAX_LAYERS),
            peak_voices: 0,
            sample_rate,
            interpolation: 0,
            events: Vec::new(),
            event_cursor: 0,
            cc_scratch: Vec::new(),
            seg_scratch: Vec::new(),
            sample_position: 0,
            render_position: 0,
            pending: std::collections::VecDeque::new(),
            states_buf: Vec::new(),
            ring: std::collections::VecDeque::new(),
            chase_base: 0,
        })
    }

    /// 批量加载排序好的事件列表（导出/Seek 用）。重置渲染位置到 0。
    pub fn load_events(&mut self, events: Vec<SynthEvent>) {
        debug_assert!(
            events.windows(2).all(|w| w[0].sample() <= w[1].sample()),
            "load_events 要求事件按 sample 有序（调用方负责排序）"
        );
        self.events = events;
        self.event_cursor = 0;
        self.voices.clear();
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        self.channel_speed_cache = [0.0; MAX_CHANNELS];
        self.drain_pending(false);
        self.ring.clear();
        self.sample_position = 0;
        self.render_position = 0;
        self.chase_base = 0;
    }

    /// 当前渲染位置
    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// 当前活跃 voice 数量（含 release 阶段，不含墓碑）。导出余韵循环用它早退。
    pub fn voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.state.env_stage < 6).count()
    }

    /// 设置全局 voice 上限（默认 8192）。超过时淘汰最老的 release 中 voice。
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max;
    }

    /// 设置采样插值方式（`Interpolation::code()`；须在加载音色库之前设置）。
    pub fn set_interpolation(&mut self, interp: u32) {
        self.interpolation = interp;
    }

    /// 每 key layer 上限（`SetLayerCount`；None = 不限制；默认 4，对齐 xsynth）。
    pub fn set_layer_count(&mut self, count: Option<usize>) {
        self.max_layers = count;
    }

    /// 渲染期间的峰值 voice 数（诊断用）
    pub fn peak_voices(&self) -> usize {
        self.peak_voices
    }

    /// Seek 到指定位置
    pub fn seek(&mut self, sample: u64) {
        // 丢弃在途块与 ring（seek 后不再输出旧内容）
        self.drain_pending(false);
        self.ring.clear();
        self.sample_position = sample;
        self.render_position = sample;
        self.event_cursor = self.events.partition_point(|e| e.sample() < sample);
        // 记录 seek 点，供 chase_skip 计算"seek 后已处理的控制事件区间"。
        self.chase_base = self.event_cursor;
        self.voices.clear();
        // 通道状态在 seek 时重置（chase 由 yinhe-audio 的 cc_events 重建保证）
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        // speed 缓存清空：下一块起点重新下发全部通道的 pitch_multiplier。
        self.channel_speed_cache = [0.0; MAX_CHANNELS];
    }
}
