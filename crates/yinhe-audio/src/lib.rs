mod audio_model;
mod audio_renderer;
mod audio_ring;
mod channel;
pub mod channel_layout;
pub mod engine;
mod engine_render;
mod engine_state;
pub mod export;
mod prepare_model;
mod preview_engine;
pub mod soundfont;
pub mod spawn;

// GPU 合成器从 yinhe-synth re-export
#[cfg(feature = "gpu")]
pub use yinhe_synth as synth;

pub use spawn::{
    AudioCommand, AudioHandle, CpalAudioHandle, PreviewNoteParams, channels_for_model,
    list_output_devices, spawn_cpal_audio,
};
