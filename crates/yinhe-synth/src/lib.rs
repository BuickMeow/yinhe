//! yinhe-synth: GPU-accelerated audio synthesizer.
//!
//! 独立的合成器 crate，包含：
//! - GpuSynth 高层封装（统一播放+导出接口，对等 xsynth ChannelGroup）
//! - GPU compute shader 渲染器 (wgpu)
//! - SFZ/SF2 解析器（委托 xsynth-soundfonts）
//! - Voice 状态管理（7 阶段 ADSR envelope + per-voice biquad 滤波器）
//! - 16 通道 MIDI 状态机（CC/pitch bend/RPN/damper）

pub mod gpu_synth;
pub mod limiter;
pub mod sfz_parser;
pub mod synth;

pub use gpu_synth::{ChaseSkip, ControlEvent, GpuSynth, MAX_CHANNELS, SynthEvent};
pub use sfz_parser::{
    KeyInfo, KeyMapEntry, LoopMode, build_key_maps, load_wav_as_f32, select_key_info,
    select_key_info_multi,
};
pub use synth::{
    GpuAudioRenderer, GpuVoiceState, RenderParams, advance_voices, biquad_coeffs, cpu_render_voices,
};
pub use wgpu;
