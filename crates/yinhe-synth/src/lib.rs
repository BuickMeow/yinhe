//! yinhe-synth: GPU-accelerated audio synthesizer.
//!
//! 独立的合成器 crate，包含：
//! - GPU compute shader 渲染器 (wgpu)
//! - SFZ 解析器（委托 xsynth-soundfonts）
//! - CPU 参考实现（用于测试/验证）
//! - Voice 状态管理（7 阶段 ADSR envelope）

pub mod sfz_parser;
pub mod synth;

pub use synth::{GpuVoiceState, GpuAudioRenderer, advance_voices, cpu_render_voices, RenderParams};
pub use wgpu;
