pub mod grid;
pub mod layer;
pub mod note_buffer_key;
pub mod pipeline;
pub mod render_thread;
mod renderer;
mod util;
pub mod vertex;

pub use layer::{LayerSlot, layer_cache_key};
pub use note_buffer_key::{NoteBufferKey, hash_hidden};
pub use render_thread::{RenderJob, DecorLayerData, NoteLayerData, RenderThreadHandle};
pub use renderer::{InstanceRenderer, PrepareTimings};
pub use util::{hash_f64s, hash_f32s, hash_bools, hash_time_sigs, compute_scroll_frac};
pub use yinhe_theme::GpuTheme;
pub use vertex::{DrawInstance, NoteInstance, Uniforms, TrackColorsUniform, MAX_TRACKS, pack_props, pack_rgba};
