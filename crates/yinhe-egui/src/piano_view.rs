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
    follow_mode: &mut super::view_interaction::FollowMode,
) {
    // Sense::hover() — no drag ownership. All drag is handled by dedicated
    // ui.interact calls below, each inside its own push_id scope.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;

    // Split: ruler band at top (24 px), wgpu content below, scrollbar at bottom
    let ruler_band_y = rect.min.y;
    let content_y = rect.min.y + RULER_H;
    let content_h = (rect.height() - RULER_H - super::scrollbar::SCROLLBAR_H).max(0.0);
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
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Auto-follow: scroll based on follow mode (playback only).
    // Never auto-follow when paused, so the user can freely scroll around.
    if let Some(ct) = *cursor_tick
        && is_playing && *follow_mode != super::view_interaction::FollowMode::None {
            let cursor_x = view.tick_to_x(ct);
            let right_edge = w as f32;
            let kb_w = view.keyboard_width();
            match *follow_mode {
                super::view_interaction::FollowMode::Page => {
                    let margin = (right_edge - kb_w) * 0.2;
                    if cursor_x > right_edge - margin || cursor_x < kb_w {
                        view.base.scroll_x =
                            (ct as f32 * view.base.pixels_per_tick) - (right_edge - kb_w) * 0.5;
                        view.clamp_scroll(w as f32, h as f32, total_ticks);
                    }
                }
                super::view_interaction::FollowMode::Continuous => {
                    // Cursor glued just inside the leftmost edge (1px inset
                    // avoids GPU clip-boundary flicker).  Use f32 arithmetic
                    // to match the GPU rendering path exactly.
                    let target = ct as f32 * view.base.pixels_per_tick;
                    view.base.scroll_x = target - 1.0;
                    view.clamp_scroll(w as f32, h as f32, total_ticks);
                }
                super::view_interaction::FollowMode::None => unreachable!(),
            }
        }

    // ── Content interaction (zoom/pan/cursor/drag/reset) ──
    // Created FIRST so that the keyboard handle (below) wins in the 4px
    // overlap zone where they intersect.
    crate::view_interaction::handle_input(
        ui,
        content_rect,
        view,
        cursor_tick,
        view.keyboard_width(),
        Some((quantize, ppq)),
        bar_line_data,
        None,
        is_playing,
        follow_mode,
    );

    // ── Keyboard resize handle ──
    // Created AFTER content interact so it wins the 4px overlap at the edge.
    // Covers ruler + content area, not the scrollbar below.
    ui.push_id("kb_handle", |ui| {
        let handle_x = rect.min.x + view.keyboard_width();
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(handle_x - 2.0, rect.min.y),
            egui::pos2(handle_x + 2.0, content_rect.max.y),
        );
        let handle_resp = ui.interact(handle_rect, ui.id(), egui::Sense::click_and_drag());
        if handle_resp.hovered() || handle_resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if handle_resp.dragged() {
            let delta = handle_resp.drag_delta().x;
            let old_kb = view.keyboard_width();
            let new_kb = (old_kb + delta).clamp(30.0, rect.width() * 0.4);

            // Keep scrollbar thumb visually in sync with the content area by
            // adjusting scroll_x so that the thumb's pixel offset within the
            // scrollbar track stays constant as the track width changes.
            let old_sb_w = w as f32 - old_kb;
            let new_sb_w = w as f32 - new_kb;
            if old_sb_w > 0.0 && new_sb_w > 0.0 {
                let start_tick = view.base.scroll_x / view.base.pixels_per_tick;
                let new_start_tick = start_tick * old_sb_w / new_sb_w;
                view.base.scroll_x = new_start_tick * view.base.pixels_per_tick;
            }

            view.base.left_panel_width = new_kb;
            view.base.dirty = true;
            ui.ctx().request_repaint();
        }
    });

    // ── Clamp scroll after all interactions ──
    // handle_input() and keyboard drag may have set scroll_x/scroll_y out of bounds.
    // Clamp before rendering to prevent 1-frame out-of-bounds visual.
    let total_ticks = midi.map(|m| m.tick_length().unwrap_or(0) as f64).unwrap_or(0.0);
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // ── Dirty detection ──
    // Run AFTER all interactions so handle_input/keyboard changes are caught.
    if *cursor_tick != *last_cursor_tick {
        view.base.dirty = true;
    }
    *last_cursor_tick = *cursor_tick;

    let force_rebuild = view.base.dirty;

    // Prepare GPU data — uses the latest view state (keyboard_width, scroll, etc.)
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

    let content_changed = view.base.dirty || gpu_dirty;
    view.base.dirty = false;

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
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat() {
            let ruler_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + view.keyboard_width(), ruler_band_y),
                egui::pos2(rect.max.x, ruler_band_y + RULER_H),
            );
            let (def_num, def_den) = midi.time_sig_default();
            let sig_events = midi.time_sig_events();
            crate::time_ruler::paint(
                &painter, ruler_rect, view, tpb, def_num, def_den, sig_events,
            );
        }

    // ── Horizontal scrollbar (right of keyboard, below content) ──
    if midi.is_some() {
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + view.keyboard_width(), content_rect.max.y),
            egui::pos2(
                rect.max.x,
                content_rect.max.y + super::scrollbar::SCROLLBAR_H,
            ),
        );
        ui.push_id("piano_scrollbar", |ui| {
            super::scrollbar::show(
                ui,
                sb_rect,
                w as f32 - view.keyboard_width(),
                &mut view.base.scroll_x,
                &mut view.base.pixels_per_tick,
                total_ticks,
                &mut view.base.dirty,
            );
        });
    }
}
