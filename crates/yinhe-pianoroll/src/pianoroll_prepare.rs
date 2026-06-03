use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::instances;
use crate::vertex::Uniforms;
use crate::view::PianoRollView;
use crate::PianorollRenderer;

/// Hash viewport properties that affect static instances.
/// Changes only when scroll, zoom, or window size actually change — NOT on every
/// playback frame.  This avoids expensive static rebuilds during steady playback.
fn viewport_hash(width: u32, height: u32, view: &PianoRollView) -> u64 {
    let mut h: u64 = 0;
    h ^= width as u64;
    h = h.wrapping_mul(31).wrapping_add(height as u64);
    h = h.wrapping_mul(31).wrapping_add(view.scroll_x.to_bits() as u64);
    h = h.wrapping_mul(31).wrapping_add(view.scroll_y.to_bits() as u64);
    h = h.wrapping_mul(31).wrapping_add(view.pixels_per_tick.to_bits() as u64);
    h = h.wrapping_mul(31).wrapping_add(view.key_height.to_bits() as u64);
    h = h.wrapping_mul(31).wrapping_add(view.keyboard_width.to_bits() as u64);
    h
}

/// Prepare the pianoroll for rendering.
///
/// Uses two-phase caching: static instances (background, grid, notes, keyboard)
/// are only rebuilt when the view changes. During playback, only the cursor line
/// is updated each frame (O(1) work).
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: Option<f64>,
) {
    let uniforms = Uniforms {
        width: width as f32,
        height: height as f32,
        scroll_x: view.scroll_x,
        scroll_y: view.scroll_y,
        pixels_per_tick: view.pixels_per_tick,
        key_height: view.key_height,
        keyboard_width: view.keyboard_width,
        _pad: 0.0,
    };

    let vhash = viewport_hash(width, height, view);

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
            instances::build_cursor_instance(
                cursor_instances,
                cursor_tick,
                view,
                width,
                height,
            );
        },
    );
}
