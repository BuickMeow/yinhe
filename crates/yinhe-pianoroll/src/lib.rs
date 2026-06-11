pub mod instances;
pub mod keyboard;
mod pianoroll_prepare;
mod view;

// Re-export from yinhe-wgpu
pub use yinhe_wgpu::{
    NoteInstance, Uniforms, pack_props, pack_rgba,
    PianorollRenderer, grid, renderer, pipeline, vertex,
};
pub use pianoroll_prepare::{prepare, PrepareTimings};
pub use view::PianoRollView;
pub use yinhe_types::{Note, NoteSource, is_black_key};
