use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::PianorollRenderer;
use crate::instances;
use crate::vertex::Uniforms;
use crate::view::PianoRollView;

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
/// Uses two-phase caching: static instances (background, grid, notes, keyboard)
/// are only rebuilt when the view changes or `force_rebuild` is set. During
/// playback, only the cursor line is updated each frame (O(1) work).
///
/// Returns `true` if GPU data (uniforms or instances) was actually updated,
/// `false` if everything was already up-to-date and a re-render can be skipped.
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: Option<f64>,
    force_rebuild: bool,
) -> bool {
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

    renderer.prepare_with_static_cache(
        uniforms,
        vhash,
        |static_instances| {
            instances::build_static_instances(
                static_instances,
                width,
                height,
                midi,
                view,
                selected,
                track_visible,
                cursor_tick,
            );
        },
        |cursor_instances| {
            instances::build_cursor_instance(cursor_instances, cursor_tick, view, width, height);
        },
    )
}
