pub(crate) mod instances;
pub use instances::build_notes;
pub use instances::build_all_notes;
mod pianoroll_prepare;

// Re-export from yinhe-wgpu (crate-local only, used by internal modules)
pub(crate) use yinhe_wgpu::{grid, vertex, InstanceRenderer};
pub use pianoroll_prepare::{prepare, build_render_job, PianorollRenderJob};
pub use yinhe_wgpu::PrepareTimings;
pub use yinhe_types::{Note, NoteSource, PianoRollView, is_black_key};
