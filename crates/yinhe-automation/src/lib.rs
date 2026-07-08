pub mod automation_view;
mod automation_prepare;
mod data_lines;
mod decor;
mod ghost;
mod tempo;
mod velocity_bars;

// Re-export from yinhe-wgpu (crate-local only, used by internal modules)
pub(crate) use yinhe_wgpu::{grid, Uniforms, InstanceRenderer};
pub use automation_prepare::{prepare as prepare_automation, AutomationGhost};
pub use automation_view::AutomationPanelView;
pub use ghost::build_lane_override;
pub use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, TimeSigEvent, TimelineViewBase};
