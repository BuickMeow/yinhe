use eframe::egui;

use yinhe_types::TimeSigEvent;

use crate::quantize::QuantizePreset;

/// Display the pianoroll texture with zoom/pan interaction.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &std::collections::HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    last_cursor_tick: &mut Option<f64>,
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

    // Clamp scroll — add some extra space beyond the last note
    let total_ticks = midi
        .and_then(|m| m.tick_length())
        .map(|tl| tl as f64 * 1.2)
        .unwrap_or(10000.0);
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Auto-follow: during playback, scroll so cursor stays visible
    if is_playing {
        if let Some(ct) = *cursor_tick {
            let cursor_x = view.tick_to_x(ct);
            let right_edge = w as f32;
            let margin = (right_edge - view.keyboard_width) * 0.2;
            if cursor_x > right_edge - margin || cursor_x < view.keyboard_width {
                view.scroll_x =
                    (ct as f32 * view.pixels_per_tick) - (right_edge - view.keyboard_width) * 0.5;
                view.clamp_scroll(w as f32, h as f32, total_ticks);
            }
        }
    }

    // ── Dirty detection ──
    // Mark dirty when cursor position changes (playback or click).
    // Must happen before force_rebuild capture so static instances
    // (including keyboard highlighting) are updated each frame.
    if *cursor_tick != *last_cursor_tick {
        view.dirty = true;
    }
    *last_cursor_tick = *cursor_tick;

    let force_rebuild = view.dirty;

    // Prepare GPU data. force_rebuild forces static instances to be rebuilt
    // (for data changes), but NOT during playback cursor movement.
    let gpu_dirty = yinhe_pianoroll::prepare(
        pianoroll,
        w,
        h,
        midi,
        view,
        selected,
        track_visible,
        *cursor_tick,
        force_rebuild,
    );

    // content_changed: true if data/playback changed this frame OR prepare
    // detected a GPU-side change. During playback, view.dirty is true (cursor
    // moved), so paint() will re-render even if prepare()'s static cache
    // was still valid.
    let content_changed = view.dirty || gpu_dirty;
    view.dirty = false;

    // Paint: RenderContext internally decides whether a full GPU render pass is
    // needed (fresh texture or content_changed) or just a display of the existing
    // texture — avoiding command-buffer back-pressure when nothing moved.
    render_ctx.paint(
        pianoroll,
        w,
        h,
        "pianoroll_frame",
        &painter,
        rect,
        content_changed,
    );

    // Handle input (zoom/pan/cursor/drag/reset)
    crate::view_interaction::handle_input(
        ui,
        &resp,
        rect,
        view,
        cursor_tick,
        view.keyboard_width,
        Some((quantize, ppq)),
        bar_line_data,
    );
}
