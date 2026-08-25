pub mod arrangement;
pub mod automation;
mod cull;
pub mod layer;
pub mod note_buffer_key;
pub mod pianoroll;
pub mod pipeline;
pub mod render_thread;
mod renderer;
pub mod resource;
mod util;
pub mod vertex;

pub use layer::{LayerSlot, layer_cache_key};
pub use note_buffer_key::{NoteBufferKey, hash_hidden};
pub use render_thread::{NoteLayerData, RenderJob, RenderThreadHandle};
pub use renderer::{InstanceRenderer, PrepareTimings};
pub use util::{hash_bools, hash_f32s, hash_f64s, hash_time_sigs};
pub use vertex::{
    DrawInstance, MAX_TRACKS, NoteInstance, Uniforms, VelocityBarInstance, pack_props, pack_rgba,
};
pub use yinhe_theme::GpuTheme;

// Re-export types that were previously provided by the separate crates
pub use arrangement::{build_ghost_notes, build_notes as build_arr_notes};
pub use automation::{
    ArrAutomationLane, AutomationGhost, build_lane_multi_copy, build_lane_multi_move,
    build_lane_override, build_lane_shape_override, prepare_arr_automation, prepare_automation,
};
pub use pianoroll::{
    PianorollRenderJob, build_all_notes, build_ghost_note, build_key_notes, build_notes,
    build_render_job,
};
