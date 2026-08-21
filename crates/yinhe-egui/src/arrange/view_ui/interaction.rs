use std::collections::HashSet;

use eframe::egui;

use yinhe_types::{ArRowLayout, ArrangementView};

use crate::piano_view::drag::{GhostNote, HiddenNote};

use super::types::{
    arrange_snapped_bounds, auto_scroll_arrange, build_ghosts_for_move, is_inside_sel_rect,
    is_on_am_row,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    hit_rect: egui::Rect,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    edit: &mut crate::arrange::ArrangeEdit<'_>,
    vertical: bool,
) -> (Vec<GhostNote>, HashSet<HiddenNote>, Option<egui::Rect>) {
    let mut ghost_notes: Vec<GhostNote> = Vec::new();
    let mut hidden_notes: HashSet<HiddenNote> = HashSet::new();
    let mut drag_rect: Option<egui::Rect> = None;
    let sel_id = ui.id().with("sel_drag_arr");
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);
    type ArrMoveDrag = ((f64, f32), (f64, f32), bool);
    let move_drag_id = ui.id().with("arr_move_drag");
    let mut move_drag: Option<ArrMoveDrag> = ui
        .data_mut(|d| d.get_persisted(move_drag_id))
        .unwrap_or(None);
    let move_orig_id = ui.id().with("arr_move_orig_sel");
    let mut move_orig_sel: Vec<(f64, f64, usize, usize)> = ui
        .data_mut(|d| d.get_persisted(move_orig_id))
        .unwrap_or_default();
    let move_had_moved_id = ui.id().with("arr_move_had_moved");
    let mut move_had_moved: bool = ui
        .data_mut(|d| d.get_persisted(move_had_moved_id))
        .unwrap_or(false);
    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    let additive = cmd || ui.input(|i| i.modifiers.shift);
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }
    if move_drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        move_drag = None;
        move_orig_sel.clear();
        move_had_moved = false;
    }
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(sel_id, drag));
        ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
        ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
        ui.data_mut(|d| d.insert_persisted(move_had_moved_id, move_had_moved));
        return (ghost_notes, hidden_notes, drag_rect);
    }
    let inside_sel_rect = is_inside_sel_rect(
        edit.arr_sel_rect,
        pointer.hover_pos(),
        content_rect,
        view,
        row_layout,
    );
    if inside_sel_rect && move_drag.is_none() && drag.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }
    let on_am_row = is_on_am_row(
        pointer.hover_pos(),
        hit_rect,
        content_rect,
        view,
        row_layout,
        data.conductor_track_idx,
    );
    if pointer.primary_pressed()
        && !on_am_row
        && let Some(pos) = pointer.hover_pos()
        && hit_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let click_tick = view.x_to_tick(local.x);
        let click_track_f = row_layout
            .hit_at_music_y(local.y + view.base.scroll_y, view.lane_height())
            .map(|h| h.track() as f32)
            .unwrap_or(0.0);
        if inside_sel_rect && !additive {
            move_orig_sel = edit.arr_sel_rect.clone();
            edit.arr_sel_rect.clear();
            let origin = (click_tick, click_track_f);
            let alt = ui.input(|i| i.modifiers.alt);
            move_drag = Some((origin, origin, alt));
            drag = None;
        } else {
            let start_track_y = (local.y + view.base.scroll_y) / view.lane_height();
            drag = Some(((click_tick, start_track_y), local));
            if !additive {
                edit.arr_sel_rect.clear();
                edit.selected.clear();
            }
        }
    }
    if let Some((origin, _, alt)) = move_drag
        && pointer.primary_down()
        && !pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let current_tick = view.x_to_tick(local.x);
        let current_track_f = row_layout
            .hit_at_music_y(local.y + view.base.scroll_y, view.lane_height())
            .map(|h| h.track() as f32)
            .unwrap_or(0.0);
        move_drag = Some((origin, (current_tick, current_track_f), alt));
        {
            let dt = super::types::snapped_delta(
                origin.0,
                current_tick,
                data.quantize,
                data.ppq,
                data.bar_line_data,
            );
            let dtr = super::types::delta_tracks(origin.1, current_track_f, vertical);
            if dt != 0 || dtr != 0 {
                move_had_moved = true;
            }
        }
        auto_scroll_arrange(
            ui,
            view,
            row_layout,
            hit_rect,
            content_rect,
            pos,
            data.total_ticks,
        );
    }
    if let Some(((origin_t, origin_tr), (current_t, current_tr), alt)) = move_drag
        && !move_orig_sel.is_empty()
    {
        let dt = super::types::snapped_delta(
            origin_t,
            current_t,
            data.quantize,
            data.ppq,
            data.bar_line_data,
        );
        let dtr = super::types::delta_tracks(origin_tr, current_tr, vertical);
        *edit.arr_sel_rect = move_orig_sel
            .iter()
            .map(|&(t_start, t_end, track_lo, track_hi)| {
                (
                    t_start + dt as f64,
                    t_end + dt as f64,
                    track_lo.saturating_add_signed(dtr as isize),
                    track_hi.saturating_add_signed(dtr as isize),
                )
            })
            .collect();
        if dt != 0 || dtr != 0 {
            move_had_moved = true;
        }
        {
            let (g, h) = build_ghosts_for_move(dt, dtr, alt, data, edit.selected);
            ghost_notes.extend(g);
            hidden_notes.extend(h);
        }
    }
    if move_drag.is_some() && pointer.primary_released() {
        if let Some(((origin_t, origin_tr), (current_t, current_tr), alt)) = move_drag {
            let delta_ticks = super::types::snapped_delta(
                origin_t,
                current_t,
                data.quantize,
                data.ppq,
                data.bar_line_data,
            );
            let delta_tracks = super::types::delta_tracks(origin_tr, current_tr, vertical);
            if delta_ticks != 0 || delta_tracks != 0 {
                move_had_moved = true;
            }
            let should_trigger = if alt {
                move_had_moved
            } else {
                delta_ticks != 0 || delta_tracks != 0
            };
            if should_trigger {
                *edit.arr_drag_delta = Some((delta_ticks, delta_tracks, alt));
                *edit.arr_sel_rect = move_orig_sel
                    .iter()
                    .map(|&(t_start, t_end, track_lo, track_hi)| {
                        (
                            t_start + delta_ticks as f64,
                            t_end + delta_ticks as f64,
                            track_lo.saturating_add_signed(delta_tracks as isize),
                            track_hi.saturating_add_signed(delta_tracks as isize),
                        )
                    })
                    .collect();
                view.base.dirty = true;
            } else {
                *edit.arr_sel_rect = move_orig_sel.clone();
            }
        }
        move_drag = None;
        move_orig_sel.clear();
        move_had_moved = false;
        drag_rect = None;
    }
    if let Some((start_music, _)) = drag {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            let clamped = pos.clamp(hit_rect.min, hit_rect.max);
            let local = egui::pos2(
                clamped.x - content_rect.min.x,
                clamped.y - content_rect.min.y,
            );
            drag = Some((start_music, local));
            auto_scroll_arrange(
                ui,
                view,
                row_layout,
                hit_rect,
                content_rect,
                pos,
                data.total_ticks,
            );
        }
        let start_pixel = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 * view.lane_height() - view.base.scroll_y,
        );
        if let Some((_, end)) = drag
            && (end - start_pixel).length() >= 3.0
            && let Some(b) =
                arrange_snapped_bounds(start_pixel, end, view, row_layout, data, vertical)
        {
            drag_rect = Some(egui::Rect::from_min_max(
                egui::pos2(b.view_sx.min(b.view_ex), b.view_sy.min(b.view_ey)),
                egui::pos2(b.view_sx.max(b.view_ex), b.view_sy.max(b.view_ey)),
            ));
        }
        if pointer.primary_released() {
            if let (Some(_midi_ref), Some((_, end))) = (data.midi, drag) {
                let drag_dist = (end - start_pixel).length();
                if drag_dist < 3.0 {
                    let tick = view.x_to_tick(start_pixel.x);
                    let snapped = crate::view_interaction::snap_tick(
                        tick,
                        data.quantize,
                        data.ppq,
                        data.bar_line_data,
                    );
                    edit.selected.clear();
                    edit.arr_sel_rect.clear();
                    *edit.cursor_tick = Some(snapped.max(0.0));
                    let track_arr_idx = start_music.1.floor() as usize;
                    if track_arr_idx < data.num_tracks {
                        let track_idx = data.track_info[track_arr_idx].index;
                        edit.track_selected.clear();
                        edit.track_selected.insert(track_idx);
                        *edit.selection_anchor = Some(track_idx);
                        *edit.info_content = Some(crate::right_panel::InfoContent::Track);
                    }
                } else if let Some(b) =
                    arrange_snapped_bounds(start_pixel, end, view, row_layout, data, vertical)
                {
                    if !additive {
                        edit.selected.clear();
                        edit.arr_sel_rect.clear();
                    }
                    edit.selected.add_rect_track(
                        b.t_start as u32,
                        b.t_end as u32,
                        0,
                        yinhe_types::MAX_KEY,
                        b.track_lo as u16,
                        b.track_hi as u16,
                    );
                    edit.arr_sel_rect
                        .push((b.t_start, b.t_end, b.track_lo, b.track_hi));
                } else if !additive {
                    edit.selected.clear();
                    edit.arr_sel_rect.clear();
                }
                view.base.dirty = true;
            }
            drag = None;
            drag_rect = None;
        }
    }
    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
    ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
    ui.data_mut(|d| d.insert_persisted(move_had_moved_id, move_had_moved));
    (ghost_notes, hidden_notes, drag_rect)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn eraser_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    hit_rect: egui::Rect,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    edit: &mut crate::arrange::ArrangeEdit<'_>,
) {
    let drag_id = ui.id().with("eraser_drag_arr");
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);
    let pointer = ui.input(|i| i.pointer.clone());
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(drag_id, drag));
        return;
    }
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && hit_rect.contains(pos)
        && !is_on_am_row(
            Some(pos),
            hit_rect,
            content_rect,
            view,
            row_layout,
            data.conductor_track_idx,
        )
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let start_tick = view.x_to_tick(local.x);
        let start_track_f = (local.y + view.base.scroll_y) / view.lane_height();
        drag = Some(((start_tick, start_track_f), local));
        *edit.arr_eraser_rect = None;
    }
    if let Some((start_music, _)) = drag {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            let clamped = pos.clamp(hit_rect.min, hit_rect.max);
            let local = egui::pos2(
                clamped.x - content_rect.min.x,
                clamped.y - content_rect.min.y,
            );
            drag = Some((start_music, local));
            auto_scroll_arrange(
                ui,
                view,
                row_layout,
                hit_rect,
                content_rect,
                pos,
                data.total_ticks,
            );
        }
        let start_pixel = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 * view.lane_height() - view.base.scroll_y,
        );
        if pointer.primary_released() {
            if let Some((_, end)) = drag {
                if (end - start_pixel).length() >= 3.0
                    && let Some(b) =
                        arrange_snapped_bounds(start_pixel, end, view, row_layout, data, false)
                {
                    *edit.arr_eraser_rect = Some((b.t_start, b.t_end, b.track_lo, b.track_hi));
                }
                view.base.dirty = true;
            }
            drag = None;
        }
    }
    ui.data_mut(|d| d.insert_persisted(drag_id, drag));
}
