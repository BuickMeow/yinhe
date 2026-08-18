pub(crate) mod data_lines;
pub(crate) mod ghost;
mod prepare;
pub(crate) mod velocity_bars;

pub use ghost::{
    build_lane_multi_copy, build_lane_multi_move, build_lane_override, build_lane_shape_override,
};
pub use prepare::{
    ArrAutomationLane, AutomationGhost, prepare as prepare_automation, prepare_arr_automation,
};
