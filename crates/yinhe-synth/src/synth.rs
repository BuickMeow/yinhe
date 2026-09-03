//! GPU-accelerated audio renderer for offline export.
//!
//! Uses wgpu compute shaders with multi-chunk sample buffers to handle
//! soundfont data larger than the GPU's max buffer binding size.

pub mod buffers;
pub mod cpu_ref;
pub mod filter;
pub mod renderer;
pub mod types;
pub mod voice;

#[cfg(test)]
mod tests;

pub use cpu_ref::cpu_render_voices;
pub use filter::biquad_coeffs;
pub use renderer::GpuAudioRenderer;
pub use types::{CHANNEL_COUNT, CHUNK_SIZE, MAX_CHUNKS, WORKGROUP_SIZE};
pub use types::{ChState, EnvUpdateCmd, GpuVoiceState, ReleaseCmd, RenderParams, SegInfo};
pub use voice::advance_voices;
