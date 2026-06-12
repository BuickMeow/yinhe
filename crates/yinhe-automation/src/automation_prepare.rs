use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::PianorollRenderer;
use crate::automation_instances;
use crate::AutomationPanelView;
use crate::Uniforms;

/// Prepare an automation panel for rendering.
///
/// Rebuilds every frame (the previous viewport-keyed cache was removed).
/// Returns `true` (kept for API stability) — GPU data is always updated now.
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    track_visible: &[bool],
    _force_rebuild: bool,
) -> bool {
    let uniforms = Uniforms {
        width: width as f32,
        height: height as f32,
        scroll_x: view.base.scroll_x,
        scroll_y: 0.0,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0,
        keyboard_width: view.base.left_panel_width,
        _pad: 0.0,
    };

    renderer
        .prepare_with_static_cache(uniforms, 0, |static_instances| {
            automation_instances::build_automation_instances(
                static_instances,
                width,
                height,
                view,
                lane,
                tpb,
                default_num,
                default_den,
                time_sig_events,
                track_visible,
            );
        })
        .dirty
}
