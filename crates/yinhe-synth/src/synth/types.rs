//! 共享类型与常量：GpuVoiceState/RenderParams/SegInfo 等。

pub const MAX_CHUNKS: usize = 5;

/// 内部分段渲染的段长（帧）：外层块（4096）在 renderer 内部切成若干段，
/// 每段独立 pass1+pass2（partial 只需 voices × 段长），一次 submit/读回。
/// 512 帧下 partial 满容量（8192 voice）= 32MB，dispatch 数 ×8。
pub const RENDER_SEGMENT_FRAMES: u32 = 512;
pub const WORKGROUP_SIZE: u32 = 256;
/// pass2 每个 workgroup 处理的帧数：原实现"每帧一个 wg"（4096 帧 = 4096 个），
/// wg 启动/调度是固定开销大头（实测 4096→1 wg 时块耗时 112→37ms）。
pub const MIX_FRAMES_PER_WG: u32 = 8;
/// MIDI 通道数（与 shader pass2 的归约布局对齐；dense = port×16+ch，
/// 支持 2 端口）。与逻辑层的 `MAX_CHANNELS` 同值——单一来源，避免两个
/// 常量今后各自漂移。
pub const CHANNEL_COUNT: usize = crate::channel_state::MAX_CHANNELS;

/// Per-voice state that is uploaded to the GPU each block.
/// 布局必须与 WGSL 的 VoiceState 结构体严格对应。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVoiceState {
    // Sample playback
    pub sample_offset: u32,
    pub sample_length: u32,
    pub speed: f32,
    /// 音色库基础播放倍率（段边界按通道 pitch_multiplier 重算 speed = base × mult）
    pub base_speed: f32,
    pub base_gain: f32,
    pub time: f32,
    pub start_offset: u32, // 块内起始帧偏移
    /// MIDI 通道（0..31），pass2 按通道归约到 channel_mix。
    pub channel: u32,
    // Envelope state at start of block
    pub envelope: f32,       // 当前 envelope 值
    pub env_stage: u32,      // 0=Delay,1=Attack,2=Hold,3=Decay,4=Sustain,5=Release,6=Finished
    pub stage_progress: f32, // 当前阶段已用帧数
    // Envelope parameters
    pub env_level: f32,     // peak = gain
    pub sustain_level: f32, // 0..1
    pub env_start: f32,     // attack 起点 / release 起始值
    /// Decay 阶段起点 amp（正常 = peak；CC72/73 重走 Decay 时 = 当前 amp）
    pub decay_start: f32,
    // Stage durations (frames)
    pub delay_frames: f32,
    pub attack_frames: f32,
    pub hold_frames: f32,
    pub decay_frames: f32,
    pub release_frames: f32,
    // 声像：音色库基础声像（通道音量/声像已迁至 yinhe-dsp 效果器，见 spec-yinhe-dsp）
    pub base_pan_l: f32,
    pub base_pan_r: f32,
    // Loop
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_mode: u32, // 0=NoLoop, 1=LoopContinuous, 2=LoopSustain, 3=OneShot
    // 采样布局与插值（与 xsynth 默认对齐：interp=0 Nearest）
    pub is_stereo: u32, // 0=单声道样本, 1=交错立体声
    pub interp: u32,    // 0=Nearest, 1=Linear
    // per-voice biquad（cutoff > 0 启用）
    pub cutoff: f32,      // Hz
    pub resonance: f32,   // 线性 Q（保留字段，系数已由 CPU 预计算）
    pub filter_type: u32, // 0=LowPass, 1=HighPass, 2=BandPass, 3=SinglePoleLowPass
    pub flt_b0: f32,
    pub flt_b1: f32,
    pub flt_b2: f32,
    pub flt_a1: f32,
    pub flt_a2: f32,
    // DirectForm1 状态（左声道；跨 block 由 GPU 写回）
    pub flt_x1: f32,
    pub flt_x2: f32,
    pub flt_y1: f32,
    pub flt_y2: f32,
    // DirectForm1 状态（右声道，仅立体声样本使用）
    pub flt_x1r: f32,
    pub flt_x2r: f32,
    pub flt_y1r: f32,
    pub flt_y2r: f32,
    /// 完全重复 NoteOn 合批引用数（增益 ×dup；note_off 逐个递减，归 1 才 release）。
    /// 与 CpuSynth 的 `dup` 同语义（线性系统里 N 个同相位同参数 voice 之和 =
    /// 单个 ×N）：黑乐谱重复 NoteOn 常态下省 voice 且对齐 CPU 能量。
    pub dup: u32,
}

/// Uniform buffer for render parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderParams {
    pub frame_count: u32,
    pub voice_count: u32,
    pub sample_rate: u32,
    pub sample_chunk_count: u32,
    pub voice_wg_count: u32,   // pass1 workgroup 数 = ceil(voice_count / 256)
    pub seg_count: u32,        // 段内段数（段边界 = CC 事件位置）
    pub release_count: u32,    // release/kill 指令总数
    pub env_update_count: u32, // CC72/73/121 包络更新指令总数
    /// partial 缓冲的每 voice 帧 stride（= 段长上界 RENDER_SEGMENT_FRAMES；
    /// 末日段短于该值时仍按此 stride 索引，保证各段不串位）
    pub partial_stride: u32,
    /// 整块 channel_mix 的帧数（pass2 写入 stride；= 外层块的帧数）
    pub channel_mix_frames: u32,
    /// 本渲染段的帧在整块 channel_mix 中的起始偏移（pass2 写入位置）
    pub mix_offset: u32,
    /// 活跃 voice 数（pass1 dispatch 与 pass2 扫描上限；active_buf 前段）
    pub active_count: u32,
    /// active_buf 中每通道 (off,count) 区间的起始 u32 索引
    pub ranges_off: u32,
    /// scatter 拷贝项数（每项 2 个 u32：staging 元素索引、目标槽位）
    pub scatter_count: u32,
    /// scatter 项数组在 scatter_items_buf 中的 u32 起始索引（轮转区域）
    pub scatter_items_base: u32,
    /// 对齐填充（uniform struct 16 字节对齐）
    pub _pad: u32,
}

/// 一个渲染段：外层块内的帧区间 + 该段的事件结构。
/// 段内所有帧索引（SegInfo.start_frame / ReleaseCmd.frame / EnvUpdateCmd.frame /
/// voice 的 start_offset）均为**段内相对**（0..frame_length）。
pub struct RenderSegment<'a> {
    /// 段在块内的起始帧（pass2 写 channel_mix 的偏移）
    pub frame_start: u32,
    /// 段帧数（<= RENDER_SEGMENT_FRAMES）
    pub frame_length: u32,
    pub segs: &'a [SegInfo],
    pub ch_updates: &'a [ChState],
    pub releases: &'a [ReleaseCmd],
    pub env_cmds: &'a [EnvUpdateCmd],
    /// 活跃 voice 数（pass1 dispatch 与 pass2 扫描上限）
    pub active_count: u32,
    /// 活跃槽位列表（按通道分桶）
    pub active_data: &'a [u32],
    /// 每通道 `[off, count]`
    pub active_ranges: &'a [u32],
}

/// 段信息：块内段边界（与 WGSL `SegInfo` 对应）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SegInfo {
    pub start_frame: u32,
    pub ch_off: u32,
    pub ch_count: u32,
    pub _pad: u32,
}

/// 段边界处某通道的新状态（CC 事件后；与 WGSL `ChState` 对应）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChState {
    pub ch: u32,
    pub speed_mult: f32,
}

/// release/kill 指令（与 WGSL `ReleaseCmd` 对应；mode 5=release，6=kill）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ReleaseCmd {
    pub frame: u32,
    pub vid: u32,
    pub mode: u32,
    pub _pad: u32,
}

/// CC72/73/121 包络更新指令（与 WGSL `EnvUpdateCmd` 对应）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EnvUpdateCmd {
    pub frame: u32,
    pub vid: u32,
    pub attack_frames: f32,
    pub release_frames: f32,
}
