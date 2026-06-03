use eframe::egui;

use yinhe_pianoroll::arrangement_instances;
use yinhe_pianoroll::{ArrangementView, NoteInstance, NoteSource, Uniforms};

use super::render_context::RenderContext;

/// Display the arrangement view texture with zoom/pan interaction.
///
/// `instances` is a reusable scratch buffer — caller should retain it across frames.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    track_names: &[String],
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
    let texture_id = render_ctx.preview_texture_id();

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
            let margin = (right_edge - view.label_width) * 0.2;
            if cursor_x > right_edge - margin || cursor_x < view.label_width {
                view.scroll_x = (ct as f32 * view.pixels_per_tick)
                    - (right_edge - view.label_width) * 0.5;
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
        keyboard_width: view.label_width,
        _pad: 0.0,
    };

    // Only rebuild instances if view state or uniforms changed
    let need_rebuild = view.dirty || renderer.uniforms_changed(&uniforms);

    if need_rebuild {
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

    // ── Render to offscreen texture ──
    let mut encoder = render_ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("arrangement_frame"),
        });
    renderer.draw(&mut encoder, render_ctx.preview_view(), w, h);
    render_ctx.queue().submit(std::iter::once(encoder.finish()));

    // ── Display the texture in egui ──
    painter.image(
        texture_id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    // ── Draw track label overlays ──
    let (trk_first, trk_last) = view.visible_track_range(h as f32, num_tracks);
    for idx in trk_first..trk_last {
        if !track_visible.get(idx).copied().unwrap_or(true) {
            continue;
        }

        let name = track_names.get(idx).map(|s| s.as_str()).unwrap_or("");
        let y = view.lane_y(idx) + rect.min.y;
        let lh = view.lane_height;
        let lw = view.label_width;

        // Background strip for label area
        let label_bg = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, y),
            egui::vec2(lw, lh),
        );
        let bg_alpha = if idx % 2 == 0 { 0.08 } else { 0.04 };
        painter.rect_filled(
            label_bg,
            0.0,
            egui::Color32::BLACK.gamma_multiply(bg_alpha),
        );

        // Color indicator
        let color = track_colors.get(idx).copied().unwrap_or([0.5, 0.5, 0.5]);
        let color32 = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );
        let indicator_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 4.0, y + 4.0),
            egui::vec2(6.0, lh - 8.0),
        );
        painter.rect_filled(indicator_rect, 2.0, color32);

        // Track name
        painter.text(
            egui::pos2(rect.min.x + 14.0, y + lh * 0.5),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE.gamma_multiply(0.85),
        );
    }

    // ── Handle input (zoom/pan/cursor/drag/reset) ──
    crate::view_interaction::handle_input(ui, &resp, rect, view, cursor_tick, view.label_width);
}
