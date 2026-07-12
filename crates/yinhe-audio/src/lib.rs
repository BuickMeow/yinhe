mod audio_model;
mod audio_renderer;
mod audio_ring;
mod channel;
pub mod engine;
mod engine_render;
mod engine_state;
pub mod export;
#[cfg(feature = "gpu")]
pub mod gpu_export;
mod prepare_model;
pub mod soundfont;
pub mod spawn;

// GPU 合成器从 yinhe-synth re-export
#[cfg(feature = "gpu")]
pub use yinhe_synth as synth;

pub use spawn::{AudioCommand, AudioHandle, CpalAudioHandle, channels_for_model, spawn_cpal_audio};
