use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::PianorollRenderer;
use crate::instances;
use crate::vertex::Uniforms;
use crate::view::PianoRollView;

pub use yinhe_wgpu::PrepareTimings;

/// Hash viewport properties that affect static instances.
fn viewport_hash(width: u32, height: u32, view: &PianoRollView) -> u64 {
    let mut h: u64 = 0;
    h ^= width as u64;
    h = h.wrapping_mul(31).wrapping_add(height as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.scroll_x.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.scroll_y.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.pixels_per_tick.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.key_height.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.left_panel_width.to_bits() as u64);
    h
}

/// Prepare the pianoroll for rendering.
///
/// Static instances (background, grid, notes, keyboard) are rebuilt only
/// when the viewport changes or `force_rebuild` is set. The playback
/// cursor line is drawn separately by egui on top of the rendered texture,
/// so it does NOT participate in caching or instance upload.
///
/// Returns `PrepareTimings` with per-phase wall-clock breakdown.
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    force_rebuild: bool,
) -> PrepareTimings {
    let uniforms = Uniforms {
        width: width as f32,
        height: height as f32,
        scroll_x: view.base.scroll_x,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: view.key_height,
        keyboard_width: view.base.left_panel_width,
        _pad: 0.0,
    };

    let mut vhash = viewport_hash(width, height, view);

    // force_rebuild: for data changes that don't affect the hash (selection,
    // track visibility), still flip to guarantee a mismatch.
    if force_rebuild {
        vhash = !vhash;
    }

    renderer.prepare_with_static_cache(uniforms, vhash, |static_instances| {
        instances::build_static_instances(
            static_instances,
            width,
            height,
            midi,
            view,
            selected,
            track_visible,
        );
    })
}
