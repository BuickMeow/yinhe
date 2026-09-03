//! 共享类型与常量：GpuVoiceState/RenderParams/SegInfo 等。

pub const MAX_CHUNKS: usize = 5;
pub const CHUNK_SIZE: usize = 30_000_000; // 30M f32 = 120MB per chunk
pub const WORKGROUP_SIZE: u32 = 256;
/// MIDI 通道数（与 shader pass2 的 32 通道归约布局对齐；dense = port×16+ch，支持 2 端口）。
pub const CHANNEL_COUNT: usize = 32;

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
    // 声像：音色库基础声像（通道 pan 渐变在 shader 内逐帧计算，见 ch_pan）
    pub base_pan_l: f32,
    pub base_pan_r: f32,
    // 通道渐变状态（xsynth ValueLerp：CC7/10/11 10ms 线性渐变，shader 逐帧推进）
    pub ch_vol: f32,
    pub ch_vol_step: f32,
    pub ch_vol_frames: u32,
    pub ch_expr: f32,
    pub ch_expr_step: f32,
    pub ch_expr_frames: u32,
    pub ch_pan: f32,
    pub ch_pan_step: f32,
    pub ch_pan_frames: u32,
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
    pub seg_count: u32,        // 块内段数（段边界 = CC 事件位置）
    pub release_count: u32,    // release/kill 指令总数
    pub env_update_count: u32, // CC72/73/121 包络更新指令总数
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
    pub ch_vol: f32,
    pub ch_vol_step: f32,
    pub ch_vol_frames: u32,
    pub ch_expr: f32,
    pub ch_expr_step: f32,
    pub ch_expr_frames: u32,
    pub ch_pan: f32,
    pub ch_pan_step: f32,
    pub ch_pan_frames: u32,
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
