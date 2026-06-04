use eframe::egui;

use yinhe_types::TimeSigEvent;

use crate::quantize::QuantizePreset;

/// Height of the time ruler band at the top of the pianoroll view.
const RULER_H: f32 = 24.0;

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

    // Split: ruler band at top (24 px), wgpu content below
    let ruler_band_y = rect.min.y;
    let content_y = rect.min.y + RULER_H;
    let content_h = (rect.height() - RULER_H).max(0.0);
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, content_y),
        egui::pos2(rect.max.x, content_y + content_h),
    );
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;

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

    // Auto-follow: scroll so cursor stays visible.
    // Always active during playback; also triggers after playback stops
    // when cursor was snapped back to start (off-screen).
    if let Some(ct) = *cursor_tick {
        let cursor_x = view.tick_to_x(ct);
        let right_edge = w as f32;
        let kb_w = view.keyboard_width;
        let margin = (right_edge - kb_w) * 0.2;
        let cursor_off_screen = cursor_x < kb_w || cursor_x > right_edge;
        if is_playing || cursor_off_screen {
            if cursor_x > right_edge - margin || cursor_x < kb_w {
                view.scroll_x = (ct as f32 * view.pixels_per_tick) - (right_edge - kb_w) * 0.5;
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

    // Paint wgpu content into the content_rect (below the ruler)
    render_ctx.paint(
        pianoroll,
        w,
        h,
        "pianoroll_frame",
        &painter,
        content_rect,
        content_changed,
    );

    // ── Time ruler (top band, right of keyboard) ──
    if let Some(midi) = midi {
        if let Some(tpb) = midi.ticks_per_beat() {
            let ruler_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + view.keyboard_width, ruler_band_y),
                egui::pos2(rect.max.x, ruler_band_y + RULER_H),
            );
            let (def_num, def_den) = midi.time_sig_default();
            let sig_events = midi.time_sig_events();
            crate::time_ruler::paint(
                &painter, ruler_rect, view, tpb, def_num, def_den, sig_events,
            );
        }
    }

    // Handle input (zoom/pan/cursor/drag/reset) — uses content_rect so that
    // viewport_height for vertical zoom excludes the ruler band.
    crate::view_interaction::handle_input(
        ui,
        &resp,
        content_rect,
        view,
        cursor_tick,
        view.keyboard_width,
        Some((quantize, ppq)),
        bar_line_data,
    );
}
