pub mod automation_instances;
pub mod automation_view;
mod automation_prepare;

// Re-export from yinhe-wgpu (crate-local only, used by internal modules)
pub(crate) use yinhe_wgpu::{grid, Uniforms, InstanceRenderer};
pub use automation_prepare::prepare as prepare_automation;
pub use automation_view::AutomationPanelView;
pub use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, TimeSigEvent, TimelineViewBase};
