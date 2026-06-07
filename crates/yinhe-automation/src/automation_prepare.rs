use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::PianorollRenderer;
use crate::automation_instances;
use crate::AutomationPanelView;
use crate::Uniforms;

/// Hash viewport properties that affect automation panel instances.
fn viewport_hash(width: u32, height: u32, view: &AutomationPanelView, target_id: u64) -> u64 {
    let mut h: u64 = 0;
    h ^= width as u64;
    h = h.wrapping_mul(31).wrapping_add(height as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.scroll_x.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.pixels_per_tick.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.left_panel_width.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.panel_height.to_bits() as u64);
    h = h.wrapping_mul(31).wrapping_add(target_id);
    h
}

/// Hash a `AutomationTarget` into a u64 for viewport hashing.
fn target_hash(target: &yinhe_types::AutomationTarget) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    target.hash(&mut hasher);
    hasher.finish()
}

/// Prepare an automation panel for rendering.
///
/// Uses two-phase caching: instances are only rebuilt when the view or
/// target changes. During playback, only scroll position changes trigger
/// a rebuild (via viewport_hash).
///
/// Returns `true` if GPU data was actually updated.
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
    force_rebuild: bool,
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

    let target_id = target_hash(&view.selected_target);
    let mut vhash = viewport_hash(width, height, view, target_id);

    if force_rebuild {
        vhash = !vhash;
    }

    // Automation panels don't have dynamic content, so the cursor phase is a no-op.
    renderer.prepare_with_static_cache(
        uniforms,
        vhash,
        |static_instances| {
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
            );
        },
        |_cursor_instances| {
            // No dynamic content for automation panels (cursor is drawn in pianoroll).
        },
    )
}
