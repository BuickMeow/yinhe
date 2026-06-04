use eframe::egui;

use yinhe_arrangement::instances as arrangement_instances;
use yinhe_arrangement::{ArrangementView, NoteInstance, NoteSource, PianorollRenderer, Uniforms};

use super::render_context::RenderContext;

/// Display the arrangement view texture with zoom/pan interaction.
///
/// `instances` is a reusable scratch buffer — caller should retain it across frames.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut PianorollRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    _track_names: &[String],
    instances: &mut Vec<NoteInstance>,
) {
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = resp.rect;
    let w = rect.width() as u32;
    let h = rect.height() as u32;

    if w == 0 || h == 0 {
        return;
    }

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(w, h);

    // Clamp scroll
    let total_ticks = midi
        .and_then(|m| m.tick_length())
        .map(|tl| tl as f64 * 1.2)
        .unwrap_or(10000.0);
    let num_tracks = track_visible.len();
    view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);

    // Auto-follow: during playback, scroll so cursor stays visible
    if is_playing {
        if let Some(ct) = *cursor_tick {
            let cursor_x = view.tick_to_x(ct);
            let right_edge = w as f32;
            let margin = right_edge * 0.2;
            if cursor_x > right_edge - margin || cursor_x < 0.0 {
                view.scroll_x = (ct as f32 * view.pixels_per_tick) - right_edge * 0.5;
                view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);
            }
        }
    }

    // Mark dirty during playback — cursor line needs to move each frame
    if is_playing && cursor_tick.is_some() {
        view.dirty = true;
    }

    // ── Compute uniforms ──
    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: view.scroll_x,
        scroll_y: view.scroll_y,
        pixels_per_tick: view.pixels_per_tick,
        key_height: view.lane_height,
        keyboard_width: 0.0,
        _pad: 0.0,
    };

    // Only rebuild instances if view state or uniforms changed
    let gpu_dirty = view.dirty || renderer.uniforms_changed(&uniforms);

    if gpu_dirty {
        // ── Build instances using scratch buffer ──
        let mut scratch = std::mem::take(instances);
        scratch.clear();

        arrangement_instances::build_arrangement_instances(
            &mut scratch,
            w,
            h,
            midi,
            view,
            track_visible,
            track_colors,
            *cursor_tick,
        );

        // ── Upload to GPU ──
        renderer.prepare_from_parts(uniforms, &scratch);

        // Return scratch buffer to caller
        scratch.clear();
        *instances = scratch;
    }
    view.dirty = false;

    // Paint: RenderContext internally decides whether a full GPU render pass is
    // needed (fresh texture or content_changed) or just a display of the existing
    // texture — avoiding command-buffer back-pressure when nothing moved.
    render_ctx.paint(
        renderer,
        w,
        h,
        "arrangement_frame",
        &painter,
        rect,
        gpu_dirty,
    );

    // ── Handle input (zoom/pan/cursor/drag/reset) ──
    crate::view_interaction::handle_input(ui, &resp, rect, view, cursor_tick, 0.0);
}
