//! Marquee / selection / eraser drag logic for the piano-roll view.

use eframe::egui;

use yinhe_types::{key_notes_in_range, TimeSigEvent};
use yinhe_editor_core::quantize::QuantizePreset;

use super::PianoViewEvent;

// ── Shared marquee drag state machine ──

/// Result of a completed marquee drag (distance >= 3px).
pub(crate) struct MarqueeDragResult {
    pub t_start: f64,
    pub t_end: f64,
    pub key_lo: u8,
    pub key_hi: u8,
    /// view-local pixel rect of the snapped marquee (for drawing).
    #[allow(dead_code)]
    pub snapped_view_rect: egui::Rect,
}

/// Shared marquee drag lifecycle: press → move (with auto-scroll) → release.
///
/// `on_press` is called once when the drag starts, allowing the caller to
/// clear or prepare state (e.g. clear selection for Select tool, no-op for Eraser).
/// Returns `Some(MarqueeDragResult)` on a valid drag release (>= 3px), `None` otherwise.
pub(crate) fn marquee_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    on_press: &mut dyn FnMut(),
    id_suffix: &'static str,
) -> Option<MarqueeDragResult> {
    let sel_id = ui.id().with(id_suffix);
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return None;
    }

    // Press → start drag
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let start_tick = view.x_to_tick(local.x);
        let start_content_y = local.y + view.base.scroll_y;
        drag = Some(((start_tick, start_content_y), local));
        on_press();
    }

    // Recompute start pixel from music coords each frame (immune to scroll/zoom)
    let start_pixel = drag.map(|((tick, content_y), _)| {
        egui::pos2(view.tick_to_x(tick), content_y - view.base.scroll_y)
    });

    // Move -> update with auto-scroll
    if let (Some(start_px), Some((start_music, _))) = (start_pixel, drag) {
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(music_rect.min, music_rect.max);
                let local = egui::pos2(
                    clamped.x - content_rect.min.x,
                    clamped.y - content_rect.min.y,
                );
                drag = Some((start_music, local));

                // ── Auto-scroll when dragging near the edge ──
                // No scroll compensation needed: start is in music coords, so it
                // automatically follows the content.
                crate::selection::drag::auto_scroll_on_drag(
                    ui,
                    &mut view.base,
                    music_rect,
                    pos,
                    |base, w, _h| {
                        base.clamp_scroll_x(w, total_ticks);
                        base.scroll_y = base.scroll_y.max(0.0);
                    },
                );
                view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);

                // ── Tooltip：显示 Δtick / Δkey ──
                let (s_tick, s_content_y) = start_music;
                let cur_tick = view.x_to_tick(local.x);
                let dt = cur_tick as i64 - s_tick as i64;
                let s_key = view.y_to_key(s_content_y - view.base.scroll_y);
                let cur_key = view.y_to_key(local.y);
                let dk = cur_key as i32 - s_key as i32;
                let lines = vec![
                    format!("±{} tick", dt),
                    format!("±{} key", dk),
                ];
                crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
            }
        }

        // Release → compute snapped bounds
        if pointer.primary_released() {
            let result = drag.and_then(|(_, end)| {
                if (end - start_px).length() >= 3.0 {
                    let (
                        sx, ex, sy, ey,
                        t_start, t_end, key_lo, key_hi,
                    ) = piano_snapped_bounds(start_px, end, view, quantize, ppq, bar_line_data);
                    let kb_w = music_rect.min.x - content_rect.min.x;
                    let snapped_view_rect = egui::Rect::from_min_max(
                        egui::pos2(sx.min(ex) - kb_w, sy.min(ey)),
                        egui::pos2(sx.max(ex) - kb_w, sy.max(ey)),
                    );
                    Some(MarqueeDragResult { t_start, t_end, key_lo, key_hi, snapped_view_rect })
                } else {
                    None
                }
            });
            ui.data_mut(|d| d.insert_persisted(sel_id, Option::<((f64, f32), egui::Pos2)>::None));
            view.base.dirty = true;
            return result;
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    None
}

// ── Selection drag logic ──

/// Pre-computed info for each selected note during a selection drag.
/// Built once at drag start, reused every frame — eliminates O(N×M) midi lookups.
#[derive(Clone)]
pub(crate) struct SelDragNoteInfo {
    pub track: u16,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    midi: Option<&dyn yinhe_types::NoteSource>,
    selected: &mut yinhe_core::Selection,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    cursor_tick: &mut Option<f64>,
    note_drag_delta: &mut Option<(i64, i32)>,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    _track_colors: &[[f32; 3]],
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
) -> (Vec<(u32, u32, u8, u16)>, Vec<(u16, u32, u8)>) {
    let note_drag_id = ui.id().with("note_drag_origin");
    let mut note_drag_origin: Option<(f64, f64)> =
        ui.data_mut(|d| d.get_persisted(note_drag_id)).unwrap_or(None);

    // Pre-computed drag note info — built once at drag start, reused every frame.
    let drag_notes_id = ui.id().with("drag_notes");
    let mut drag_notes: Option<Vec<SelDragNoteInfo>> =
        ui.data_mut(|d| d.get_persisted(drag_notes_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

    // Clear stale note drag state
    if note_drag_origin.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        note_drag_origin = None;
        drag_notes = None;
        sel_rect.cancel_drag();
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (Vec::new(), Vec::new());
    }

    // Start drag (note drag only — marquee is handled by shared function below)
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let eff = sel_rect.effective();
        let on_bar = eff.is_some_and(|(t_start, t_end, key_lo, key_hi)| {
            let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
            );
            crate::widgets::selection_actions::compute_bar_rect(music_rect, pixel_rect)
                .is_some_and(|bar| bar.contains(pos))
        });

        if on_bar {
            // Don't start drag, don't clear anything — let the button handle it.
        } else {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let in_sel_rect = eff.is_some_and(|(t_start, t_end, key_lo, key_hi)| {
                let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                    &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
                );
                pixel_rect.contains(local)
            });
            if in_sel_rect {
                let raw_tick = view.x_to_tick(local.x);
                let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let key = view.y_to_key(local.y) as f64;
                note_drag_origin = Some((tick, key));
                sel_rect.start_drag();

                // Pre-compute all selected note info from rects + midi.
                // Use Selection.contains() for precise half-open tick filtering
                // (avoids off-by-one vs key_notes_in_range's broad range).
                // Add track_selected + track_visible filters to align ghost with
                // the note layer (build_notes).
                let sel = &*selected;
                drag_notes = Some(sel.rects.iter().flat_map(|&(ts, te, kl, kh, _tl, _th)| {
                    (kl..=kh).flat_map(move |key| {
                        midi.map(|m| {
                            key_notes_in_range(m.key_notes(key), ts, te).iter()
                                .filter(|n| sel.contains(n.track, n.start_tick, key))
                                .filter(|n| track_selected.is_empty() || track_selected.contains(&n.track))
                                .filter(|n| track_visible.get(n.track as usize).copied().unwrap_or(true))
                                .map(|n| SelDragNoteInfo {
                                    track: n.track,
                                    start_tick: n.start_tick,
                                    end_tick: n.end_tick,
                                    key,
                                })
                                .collect::<Vec<_>>()
                        }).unwrap_or_default()
                    })
                }).collect());
            }
        }
    }

    // Note drag: use pre-computed data for ghost/hidden, store delta only on release
    let mut ghost_notes: Vec<(u32, u32, u8, u16)> = Vec::new();
    let mut hidden_notes: Vec<(u16, u32, u8)> = Vec::new();
    if let Some((origin_tick, origin_key)) = note_drag_origin {
        if let Some(ref notes) = drag_notes {
            if pointer.primary_down() && !pointer.primary_pressed() {
                if let Some(pos) = pointer.hover_pos() {
                    // auto-scroll：拖拽音符能推出屏幕（pos 未 clamp）
                    crate::selection::drag::auto_scroll_on_drag(
                        ui,
                        &mut view.base,
                        music_rect,
                        pos,
                        |base, w, _h| {
                            base.clamp_scroll_x(w, total_ticks);
                            base.scroll_y = base.scroll_y.max(0.0);
                        },
                    );
                    view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);

                    // 位置 clamp 到 music_rect，避免鼠标飞出后产生异常值
                    let clamped = pos.clamp(music_rect.min, music_rect.max);
                    let local_x = clamped.x - content_rect.min.x;
                    let local_y = clamped.y - content_rect.min.y;
                    let raw_tick = view.x_to_tick(local_x);
                    let snapped_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                    let current_key = view.y_to_key(local_y) as f64;
                    let dt = (snapped_tick - origin_tick).round() as i64;
                    let dk = (current_key - origin_key).round() as i32;

                    // O(N) — just apply delta to pre-computed data, no midi lookup.
                    for info in notes {
                        let new_tick = (info.start_tick as i64 + dt).max(0) as u32;
                        let new_key = ((info.key as i32) + dk).clamp(0, 127) as u8;
                        let length = info.end_tick - info.start_tick;
                        ghost_notes.push((new_tick, new_tick + length, new_key, info.track));
                        hidden_notes.push((info.track, info.start_tick, info.key));
                    }

                    sel_rect.update_drag(dt, dk);
                    ui.ctx().request_repaint();
                }
            }
            if pointer.primary_released() {
                if let Some(pos) = pointer.hover_pos() {
                    let clamped = pos.clamp(music_rect.min, music_rect.max);
                    let local_x = clamped.x - content_rect.min.x;
                    let local_y = clamped.y - content_rect.min.y;
                    let raw_tick = view.x_to_tick(local_x);
                    let snapped_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                    let current_key = view.y_to_key(local_y) as f64;
                    let dt = (snapped_tick - origin_tick).round() as i64;
                    let dk = (current_key - origin_key).round() as i32;
                    *note_drag_delta = Some((dt, dk));
                    sel_rect.update_drag(dt, dk);

                    // Keep ghost/hidden alive on the release frame so the original
                    // notes don't flash back before the model is updated.
                    for info in notes {
                        let new_tick = (info.start_tick as i64 + dt).max(0) as u32;
                        let new_key = ((info.key as i32) + dk).clamp(0, 127) as u8;
                        let length = info.end_tick - info.start_tick;
                        ghost_notes.push((new_tick, new_tick + length, new_key, info.track));
                        hidden_notes.push((info.track, info.start_tick, info.key));
                    }
                }
                sel_rect.end_drag();
                note_drag_origin = None;
                drag_notes = None;
            }
        }
    }

    // ── Marquee selection (shared with Eraser tool) ──
    // Only start a marquee if no note drag is active (click was NOT inside selection).
    if note_drag_origin.is_some() {
        // Note drag active → clear any stale marquee state and skip marquee.
        let sel_id = ui.id().with("sel_drag");
        ui.data_mut(|d| d.insert_persisted(sel_id, Option::<((f64, f32), egui::Pos2)>::None));
    } else {
        let mut on_press = || {
            if !cmd {
                selected.clear();
            }
            sel_rect.rect = None;
        };
        if let Some(result) = marquee_drag_frame(
            ui, content_rect, music_rect, view, quantize, ppq, bar_line_data, total_ticks,
            &mut on_press, "sel_drag",
        ) {
            let track_lo = track_selected.iter().min().copied().unwrap_or(0);
            let track_hi = track_selected.iter().max().copied().unwrap_or(u16::MAX);
            selected.add_rect_track(
                result.t_start as u32, result.t_end as u32,
                result.key_lo, result.key_hi,
                track_lo, track_hi,
            );
            sel_rect.rect = Some((result.t_start, result.t_end, result.key_lo, result.key_hi));
        } else if ui.input(|i| i.pointer.primary_released()) {
            // Simple click (no marquee) - set cursor to click position for paste.
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                if music_rect.contains(pos) {
                    let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                    let tick = view.x_to_tick(local.x);
                    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
                    *cursor_tick = Some(snapped.max(0.0));
                }
            }
        }
    }

    ui.data_mut(|d| d.insert_persisted(note_drag_id, note_drag_origin));
    ui.data_mut(|d| d.insert_persisted(drag_notes_id, drag_notes));
    (ghost_notes, hidden_notes)
}

/// Draw the active marquee box on top of GPU content.
/// Must be called AFTER `render_ctx.paint` so the box is not covered by the texture.
/// `id_suffix` — persisted drag state key (e.g. "sel_drag" or "eraser_drag").
/// `fill_color` / `stroke_color` — base colors for the marquee.
pub(crate) fn draw_marquee_box(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    id_suffix: &'static str,
    fill_color: egui::Color32,
    stroke_color: egui::Color32,
) {
    let drag_id = ui.id().with(id_suffix);
    let drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);

    if let Some((start_music, end)) = drag {
        let start = egui::pos2(view.tick_to_x(start_music.0), start_music.1 - view.base.scroll_y);
        if (end - start).length() < 3.0 {
            return;
        }
        let (vx, vy, vw, vh, _, _, _, _) =
            piano_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);
        let kb_w = music_rect.min.x - content_rect.min.x;
        let snapped = egui::Rect::from_min_max(
            egui::pos2(vx.min(vy) - kb_w, vw.min(vh)),
            egui::pos2(vx.max(vy) - kb_w, vw.max(vh)),
        );
        crate::selection::draw::draw(&ui.painter(), music_rect, snapped, fill_color, stroke_color);
    }
}

// ── Eraser tool ──

/// Eraser-tool input: uses the shared marquee drag, then returns an
/// `EraserDelete` event on release. No selection persistence.
#[allow(clippy::too_many_arguments)]
pub(crate) fn eraser_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    track_selected: &std::collections::HashSet<u16>,
) -> Option<PianoViewEvent> {
    let result = marquee_drag_frame(
        ui, content_rect, music_rect, view, quantize, ppq, bar_line_data, total_ticks,
        &mut || {}, // no-op on press for eraser
        "eraser_drag",
    )?;
    let track_lo = track_selected.iter().min().copied().unwrap_or(0);
    let track_hi = track_selected.iter().max().copied().unwrap_or(u16::MAX);
    Some(PianoViewEvent::EraserDelete {
        t_start: result.t_start as u32,
        t_end: result.t_end as u32,
        key_lo: result.key_lo,
        key_hi: result.key_hi,
        track_lo,
        track_hi,
    })
}

/// Compute snapped selection bounds for piano roll.
#[allow(clippy::too_many_arguments)]
pub(crate) fn piano_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f32, f32, f32, f32, f64, f64, u8, u8) {
    let sx = start.x.min(end.x);
    let ex = start.x.max(end.x);
    let sy = start.y.min(end.y);
    let ey = start.y.max(end.y);

    let tick_s = view.x_to_tick(sx);
    let tick_e = view.x_to_tick(ex);
    let snapped_s = crate::view_interaction::snap_tick(tick_s, quantize, ppq, bar_line_data);
    let snapped_e = crate::view_interaction::snap_tick(tick_e, quantize, ppq, bar_line_data);
    let t_start = snapped_s.min(snapped_e);
    let mut t_end = snapped_s.max(snapped_e);

    // Ensure minimum width of one quantise grid interval
    let interval = quantize.tick_interval(ppq) as f64;
    if t_end <= t_start {
        t_end = t_start + interval.max(1.0);
    }

    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;

    let key_lo = (127.0 - ((scroll_y + ey) / kh)).ceil().max(0.0).min(127.0) as u8;
    let key_hi = (127.0 - ((scroll_y + sy) / kh)).ceil().max(0.0).min(127.0) as u8;
    let screen_sy = (127.0 - key_hi as f32) * kh - scroll_y;
    let screen_ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;

    let screen_sx = view.tick_to_x(t_start);
    let screen_ex = view.tick_to_x(t_end);

    (
        screen_sx, screen_ex, screen_sy, screen_ey, t_start, t_end, key_lo, key_hi,
    )
}
