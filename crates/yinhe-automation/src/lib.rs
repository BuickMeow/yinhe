pub mod automation_instances;
pub mod automation_view;
mod automation_prepare;

pub use yinhe_wgpu::{
    NoteInstance, Uniforms, pack_props, pack_rgba,
    PianorollRenderer, grid,
};
pub use automation_prepare::prepare as prepare_automation;
pub use automation_view::AutomationPanelView;
pub use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, TimeSigEvent, TimelineViewBase};
