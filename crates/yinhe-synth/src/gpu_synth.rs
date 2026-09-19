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
// 被淘汰 voice 的力度分桶（诊断：确认过载时牺牲的是小力度而非大力度）。
// 桶：0..=31 / 32..=63 / 64..=127。
pub static EVICT_VEL_LO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static EVICT_VEL_MID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static EVICT_VEL_HI: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `note_on` 因槽位满被拒绝（不发声）的计数——与淘汰不同，这是**丢音**。
pub static NOTE_ON_REJECTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 完全重复 NoteOn 合批命中计数（诊断；合批省下的 voice 创建数）。
pub static BATCH_HITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
// 被拒绝音符的力度分桶（诊断：确认丢的是小力度还是大力度）
pub static REJECT_VEL_LO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static REJECT_VEL_MID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static REJECT_VEL_HI: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 反事实淘汰探针开关（诊断；不影响实际淘汰选择，仅在超限时额外统计）。
pub static PROBE_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 探针累计的淘汰事件数（每次超限一段算一次）。
pub static PROBE_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 探针"结束最久优先"策略选中 voice 距 NoteOff 的 age 分桶。
/// 0=结束还早(>2s) 1=2~0.5s 2=0.5~0.1s 3=0.1~0s 4=已结束0~0.1s 5=0.1~0.5s 6=0.5~2s 7=>2s
pub static PROBE_END_AGE: [std::sync::atomic::AtomicU64; 8] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 8];
/// 探针"结束最久优先"选中者中已结束（now >= end_sample）的数量。
pub static PROBE_END_RELEASED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 合批失配原因探针：bucket 内存在同 (vel,end_sample) 候选但某项失配的次数。
/// 0=kill_pending 1=release_pending 2=held 3=env>=6 4=sample_offset 5=length
/// 6=speed 7=base_speed 8=start_offset 9=不应发生 10=无同 vel/end_sample 候选
pub static PROBE_BATCH_MISS: [std::sync::atomic::AtomicU64; 11] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 11];
/// voice 状态上传的 write_buffer 调用次数 / 覆盖槽位数 / 累计微秒（诊断）。
pub static FLUSH_WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static FLUSH_SLOTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static FLUSH_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 探针"力度优先"策略选中 voice 的力度 16 档直方图（vel/8）。
pub static PROBE_VEL16: [std::sync::atomic::AtomicU64; 16] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 16];

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
    /// 空闲槽位（voice 结束、harvest 读回后即时回收）——`note_on` 优先复用，
    /// 避免墓碑累积顶到容量上限、也避免周期性 compact 排空流水线（实测
    /// 每块 compact 会等两个在途 GPU 块，60-90ms 级）。
    ///
    /// **FIFO（最低索引优先）**：harvest 按索引升序回收，front 即最小空闲槽位。
    /// 若用 LIFO，新音会填回最近释放的高索引，活跃 voice 全部沉在高位，
    /// 尾部截断失效、`voices.len()` 高水位不降（实测低潮段 alive=214 时
    /// pass1 仍按 1.7 万槽位渲染，harvest 固定 ~95ms）。
    free_slots: std::collections::VecDeque<u32>,
    /// 各槽位是否已回收进 `free_slots`（防重复入列；与 `voices` 等长）
    freed_flags: Vec<bool>,
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
    /// 每 (dense 通道 × 128 key) 的活跃 voice 计数（per-key layer 限制用）。
    /// 增量维护：note_on +1，kill / GPU 确认结束 -1——替代每音一次 O(V) 全扫。
    layer_counts: Box<[u32]>,
    /// 峰值 voice 数统计（诊断用）
    peak_voices: usize,
    /// 上一块耗时分解（ms）：[collect, submit(含 collect), harvest, ring, out, compact]
    pub diag_ms: [f64; 8],
    /// 上一块提交的 GPU 块数
    pub diag_blocks: u32,
    /// 上一块结束时的活跃 voice 数
    pub diag_alive: u32,
    /// 上一块输出时 ring 不足、静音补齐的帧数（>0 即输出被截断）
    pub diag_ring_short: u32,
    /// 最近一次 harvest 读回 GPU channel_mix 的峰值（0 = pass 产出就是静音）
    pub diag_gpu_mix_peak: f32,
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
    /// voice 槽位复用代号计数器（harvest 身份校验，见 `Voice::slot_gen`）。
    next_gen: u32,
    /// 本块内创建的 voice 按 (dense 通道 × 128 key) 分桶（合批候选扫描用）。
    /// 跨块 voice 已渲染（参数/包络状态已推进）不可合批，故只需本块新建者；
    /// 每块 collect_block 开头清空。桶内索引 = 槽位索引。
    batch_buckets: Vec<Vec<u32>>,
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
    /// 提交时各槽位的 voice 代号快照：收割时逐槽校验，槽位已被复用（代号
    /// 不同）则丢弃该槽位的读回状态（属于旧 voice）。
    voice_gens: Vec<u32>,
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
            free_slots: std::collections::VecDeque::new(),
            freed_flags: Vec::new(),
            channel_speed_cache: [0.0; MAX_CHANNELS],
            channel_mix: Vec::new(),
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            max_voices: crate::DEFAULT_MAX_VOICES,
            max_layers: Some(crate::DEFAULT_MAX_LAYERS),
            layer_counts: vec![0u32; MAX_CHANNELS * 128].into_boxed_slice(),
            peak_voices: 0,
            diag_ms: [0.0; 8],
            diag_blocks: 0,
            diag_alive: 0,
            diag_ring_short: 0,
            diag_gpu_mix_peak: 0.0,
            sample_rate,
            interpolation: 0,
            events: Vec::new(),
            event_cursor: 0,
            cc_scratch: Vec::new(),
            next_gen: 1,
            batch_buckets: (0..MAX_CHANNELS * 128).map(|_| Vec::new()).collect(),
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
        self.free_slots.clear();
        self.freed_flags.clear();
        self.layer_counts.fill(0);
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

    /// 诊断：长寿 voice 详情（channel, key, env_stage, release_pending,
    /// held_by_damper, loop_mode），最多 8 个——判断是踏板按住还是泄漏。
    pub fn long_lived_detail(
        &self,
        current_sample: u64,
        threshold: u64,
    ) -> Vec<(u8, u8, u32, bool, bool, u32)> {
        self.voices
            .iter()
            .filter(|v| {
                v.state.env_stage < 6 && current_sample.saturating_sub(v.start_sample) > threshold
            })
            .take(8)
            .map(|v| {
                (
                    v.channel,
                    v.key,
                    v.state.env_stage,
                    v.release_pending,
                    v.held_by_damper,
                    v.state.loop_mode,
                )
            })
            .collect()
    }

    /// 诊断：各 dense 通道当前 (bank, program)。
    pub fn debug_channel_banks(&self) -> Vec<(u8, u8)> {
        self.channels.iter().map(|c| (c.bank, c.program)).collect()
    }

    /// 诊断：CPU 侧某采样区间的峰值（判断采样数据是否本身为静音）。
    pub fn debug_sample_peak(&self, start: usize, len: usize) -> f32 {
        let d = &self.renderer.sample_data;
        let end = (start + len).min(d.len());
        d.get(start..end)
            .map(|s| s.iter().fold(0.0f32, |m, &v| m.max(v.abs())))
            .unwrap_or(0.0)
    }

    /// 诊断：CPU 侧拼接采样的原始值 + 总长度。
    pub fn debug_sample_value(&self, idx: usize) -> (usize, f32) {
        (
            self.renderer.sample_data.len(),
            self.renderer
                .sample_data
                .get(idx)
                .copied()
                .unwrap_or(f32::NAN),
        )
    }

    /// 诊断：voice 关键字段 (offset, length, gain, time, envelope, stage)。
    pub fn debug_voice_tuple(&self, idx: usize) -> Option<(u32, u32, f32, f32, f32, u32)> {
        self.voices.get(idx).map(|v| {
            (
                v.state.sample_offset,
                v.state.sample_length,
                v.state.base_gain,
                v.state.time,
                v.state.envelope,
                v.state.env_stage,
            )
        })
    }

    /// 诊断：某 port 的音色库条目摘要：总条目数 + 鼓组（bank 128）条目。
    pub fn port_entries(&self, port: usize) -> (usize, Vec<(u8, u8, usize)>) {
        self.port_key_maps
            .get(port)
            .map(|m| {
                (
                    m.len(),
                    m.iter()
                        .filter(|e| e.bank == 128)
                        .map(|e| {
                            (
                                e.bank,
                                e.preset,
                                e.map.iter().filter(|l| !l.is_empty()).count(),
                            )
                        })
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    /// 诊断：按 dense 通道统计活跃 voice 数（判断各通道是否真的在发声）。
    pub fn voice_count_by_channel(&self) -> [u32; MAX_CHANNELS] {
        let mut out = [0u32; MAX_CHANNELS];
        for v in self.voices.iter() {
            if v.state.env_stage < 6 && (v.channel as usize) < MAX_CHANNELS {
                out[v.channel as usize] += 1;
            }
        }
        out
    }

    /// 设置全局 voice 上限（默认 8192）。超过时淘汰最老的 release 中 voice。
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max;
    }

    /// 活跃 voice 的力度直方图（8 桶，从高到低：127-112、111-96、…、15-0）。
    /// 诊断用：确认高力度音符在实时的存活数量。
    pub fn velocity_histogram(&self) -> [u32; 8] {
        let mut h = [0u32; 8];
        for v in self.voices.iter() {
            if v.state.env_stage >= 6 {
                continue;
            }
            let bucket = 7usize.saturating_sub((v.velocity as usize) / 16).min(7);
            h[bucket] += 1;
        }
        h
    }

    /// 长寿 voice 数（活跃且创建时间早于 `current - threshold` 样本）。
    /// 诊断 voice 泄漏：正常音符寿命 = gate + release（秒级），远超则没结束。
    pub fn long_lived_voices(&self, current_sample: u64, threshold: u64) -> u32 {
        self.voices
            .iter()
            .filter(|v| {
                v.state.env_stage < 6 && current_sample.saturating_sub(v.start_sample) > threshold
            })
            .count() as u32
    }

    /// 状态统计：(未开始 start_offset>0, release 中 stage>=5, 正常发声 stage<5)。
    /// 诊断"有 voice 但输出为 0"：若大量 voice 未开始或早早 release，则整块静音。
    pub fn state_stats(&self) -> (u32, u32, u32) {
        let (mut not_started, mut releasing, mut sounding) = (0u32, 0u32, 0u32);
        for v in self.voices.iter() {
            if v.state.env_stage >= 6 {
                continue;
            }
            if v.state.start_offset > 0 {
                not_started += 1;
            } else if v.state.env_stage >= 5 {
                releasing += 1;
            } else {
                sounding += 1;
            }
        }
        (not_started, releasing, sounding)
    }

    /// 活跃 voice 按 key 计数，返回 Top N 最多的 key（找异常堆积）。
    pub fn top_keys(&self, n: usize) -> Vec<(u8, u32)> {
        let mut counts = [0u32; 128];
        for v in self.voices.iter() {
            if v.state.env_stage < 6 {
                counts[v.key as usize] += 1;
            }
        }
        let mut v: Vec<(u8, u32)> = counts
            .iter()
            .enumerate()
            .filter(|&(_, &c)| c > 0)
            .map(|(k, &c)| (k as u8, c))
            .collect();
        v.sort_unstable_by_key(|x| std::cmp::Reverse(x.1));
        v.truncate(n);
        v
    }

    /// 诊断：输出游标 / 提交游标 / 事件游标 / 事件总数。
    pub fn diag_cursors(&self) -> (u64, u64, usize, usize) {
        (
            self.sample_position,
            self.render_position,
            self.event_cursor,
            self.events.len(),
        )
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
        self.free_slots.clear();
        self.freed_flags.clear();
        self.layer_counts.fill(0);
        // 通道状态在 seek 时重置（chase 由 yinhe-audio 的 cc_events 重建保证）
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        // speed 缓存清空：下一块起点重新下发全部通道的 pitch_multiplier。
        self.channel_speed_cache = [0.0; MAX_CHANNELS];
    }
}
