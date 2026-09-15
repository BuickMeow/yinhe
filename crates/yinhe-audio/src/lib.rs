mod audio_model;
mod audio_renderer;
mod audio_ring;
pub mod audio_source;
mod builtin_insert;
mod channel;
pub mod channel_layout;
mod channel_set;
pub mod clap_insert;
pub mod engine;
mod engine_mixer;
mod engine_render;
mod engine_state;
pub mod export;
mod instrument;
mod prepare_model;
mod preview_engine;
pub mod soundfont;
pub mod spawn;

// GPU 合成器从 yinhe-synth re-export
#[cfg(feature = "gpu")]
pub use yinhe_synth as synth;

pub use audio_model::effective_fades;
pub use audio_source::{
    AudioInfo, DecodedAudio, WavePeaks, decode_audio, encode_wav_bytes, probe_audio_info,
    resample_channel,
};
pub use builtin_insert::BuiltinInsert;
pub use clap_insert::ClapInsert;
pub use spawn::{
    AudioCommand, AudioHandle, CpalAudioHandle, InsertTarget, InstrumentPreviewNote,
    PreviewNoteParams, channels_for_model, discover_sample_rates, list_input_devices,
    list_output_devices, spawn_cpal_audio,
};
