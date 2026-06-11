use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::PianorollRenderer;
use crate::instances;
use crate::vertex::Uniforms;
use crate::view::PianoRollView;

pub use yinhe_wgpu::PrepareTimings;

/// Prepare the pianoroll for rendering.
///
/// Rebuilds static instances every frame. The playback cursor line is drawn
/// separately by egui on top of the rendered texture and is NOT part of the
/// instance buffer.
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

    renderer.prepare_with_static_cache(uniforms, 0, |static_instances| {
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
