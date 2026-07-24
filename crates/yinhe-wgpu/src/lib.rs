pub mod arrangement;
pub mod automation;
mod cull;
pub mod layer;
pub mod note_buffer_key;
pub mod pianoroll;
pub mod pipeline;
pub mod render_thread;
mod renderer;
mod util;
pub mod vertex;

pub use layer::{LayerSlot, layer_cache_key};
pub use note_buffer_key::{NoteBufferKey, hash_hidden};
pub use render_thread::{RenderJob, NoteLayerData, RenderThreadHandle};
pub use renderer::{InstanceRenderer, PrepareTimings};
pub use util::{hash_f64s, hash_f32s, hash_bools, hash_time_sigs, compute_scroll_frac};
pub use yinhe_theme::GpuTheme;
pub use vertex::{DrawInstance, NoteInstance, VelocityBarInstance, Uniforms, MAX_TRACKS, pack_props, pack_rgba};

// Re-export types that were previously provided by the separate crates
pub use pianoroll::{build_render_job, PianorollRenderJob, build_notes, build_all_notes, build_key_notes, build_ghost_note};
pub use automation::{prepare_automation, AutomationGhost, build_lane_override, build_lane_shape_override};
pub use arrangement::{build_ghost_notes, build_notes as build_arr_notes};
