pub mod automation_instances;
pub mod automation_view;
pub mod instances;
pub mod keyboard;
mod automation_prepare;
mod pianoroll_prepare;
mod view;

// Re-export from yinhe-wgpu
pub use yinhe_wgpu::{
    NoteInstance, Uniforms, pack_props, pack_rgba,
    PianorollRenderer, grid, renderer, pipeline, vertex,
};
pub use automation_prepare::prepare as prepare_automation;
pub use pianoroll_prepare::prepare;
pub use automation_view::AutomationPanelView;
pub use view::PianoRollView;
pub use yinhe_types::{Note, NoteSource, is_black_key, AutomationLane, AutomationTarget};
