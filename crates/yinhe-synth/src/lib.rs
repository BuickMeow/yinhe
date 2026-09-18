//! yinhe-synth: GPU-accelerated audio synthesizer.
//!
//! 独立的合成器 crate，**只负责音源层**：采样播放、voice 包络（ADSR）、
//! 音高（pitch bend/RPN 调音）、延音踏板、音色选择。
//! 通道级 DSP（音量/声像/滤波/效果/输出限幅）全部由 yinhe-dsp 与混音台负责
//! （见 `docs/spec-yinhe-dsp.md`）。
//!
//! - GpuSynth 高层封装（统一播放+导出接口，对等 xsynth ChannelGroup）
//! - GPU compute shader 渲染器 (wgpu)
//! - SFZ/SF2 解析器（委托 xsynth-soundfonts）
//! - Voice 状态管理（7 阶段 ADSR envelope + per-voice biquad 滤波器 = 音色自带 filter）
//! - 32 通道 MIDI 状态机（仅音源层：bank/program、pitch bend/RPN、damper、ADSR CC）

pub mod channel_state;
pub mod cpu_synth;
pub mod denormals;
pub mod gpu_synth;
pub(crate) mod sf_cache;
pub mod sf_parser;
pub mod synth;
mod voice_params;

pub use channel_state::{ChaseSkip, MAX_CHANNELS};

/// 默认全局 voice 上限（CPU/GPU 共用；淘汰策略各自实现：CPU 块末淡出，
/// GPU 指令 kill）。**这是性能目标**（GPU 时间 ∝ 活跃数，实测每千 voice
/// ≈2.8ms；14336 ≈ 40ms，给低端设备与 UI 争抢留余量），与**容量**
/// （`MAX_VOICE_SLOTS`，只决定内存和 note_on 可容纳量）解耦：
/// 容量远大于本值时，`note_on` 不会因墓碑占位而拒绝新音（丢音）。
pub(crate) const DEFAULT_MAX_VOICES: usize = 14336;
/// 默认每 key layer 上限（对齐 xsynth `VoiceChannelParams.layers`；共用）。
pub(crate) const DEFAULT_MAX_LAYERS: usize = 4;
pub use cpu_synth::CpuSynth;
pub use gpu_synth::{ControlEvent, GpuSynth, SynthEvent, prefetch_key_maps};
pub use sf_parser::{
    KeyInfo, KeyMapEntry, LoopMode, build_key_maps, load_wav_as_f32, select_key_info,
    select_key_info_multi,
};
pub use synth::{
    GpuAudioRenderer, GpuVoiceState, RENDER_SEGMENT_FRAMES, RenderParams, RenderSegment,
    biquad_coeffs,
};
pub use wgpu;
