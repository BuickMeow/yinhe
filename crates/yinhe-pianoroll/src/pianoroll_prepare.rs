use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::instances;
use crate::vertex::Uniforms;
use crate::view::PianoRollView;
use crate::PianorollRenderer;

/// Prepare the pianoroll for rendering.
///
/// Convenience wrapper around [`PianorollRenderer::prepare_with_builder`]
/// that builds note/grid instances from pianoroll-specific view state.
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

    renderer.prepare_with_builder(uniforms, view.dirty, |instances| {
        instances::build_instances(
            instances,
            width,
            height,
            midi,
            view,
            selected,
            track_visible,
            cursor_tick,
        );
    });
}
