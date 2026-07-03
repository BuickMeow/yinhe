pub mod instances;
pub mod keyboard;
mod pianoroll_prepare;
mod view;

// Re-export from yinhe-wgpu (crate-local only, used by internal modules)
pub(crate) use yinhe_wgpu::{grid, vertex, InstanceRenderer};
pub use pianoroll_prepare::{prepare};
pub use yinhe_wgpu::PrepareTimings;
pub use view::PianoRollView;
pub use yinhe_types::{Note, NoteSource, is_black_key};
