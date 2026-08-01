//! yinhe-synth: GPU-accelerated audio synthesizer.
//!
//! 独立的合成器 crate，包含：
//! - GpuSynth 高层封装（统一播放+导出接口，对等 xsynth ChannelGroup）
//! - GPU compute shader 渲染器 (wgpu)
//! - SFZ/SF2 解析器（委托 xsynth-soundfonts）
//! - Voice 状态管理（7 阶段 ADSR envelope）

pub mod gpu_synth;
pub mod sfz_parser;
pub mod synth;

pub use gpu_synth::{GpuSynth, SynthEvent};
pub use sfz_parser::{KeyInfo, LoopMode, build_key_map, load_wav_as_f32, select_key_info};
pub use synth::{GpuAudioRenderer, GpuVoiceState, RenderParams, advance_voices, cpu_render_voices};
pub use wgpu;
