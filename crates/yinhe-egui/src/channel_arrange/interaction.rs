//! 通道 AR 的选框 / 拖动 / 橡皮擦交互（行空间）。
//!
//! 与轨道 AR（`arrange::view_ui::interaction`）的差异：
//! - 选框不是 `(track_lo, track_hi)` 连续范围，而是「行区间 → 轨道集合」；
//! - 垂直拖动 delta 按落点行的「锚定轨道」之差计算（行高不等，行号差无意义）；
//! - 拖动预览的 ghost 需按「轨道 → 可见行」映射展开到行 id。

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;

use yinhe_editor_core::edit_state::ChanSelRect;
use yinhe_types::ArrangementView;

use crate::arrange::{ArrangeData, ArrangeEdit};

use super::groups::CaGroups;
use super::rows::{CaRowLayout, row_tracks, rows_range_tracks, track_runs};
use super::types::{CaGhostNote, anchor_track, is_am_row, point_in_sel_rects, row_at_local_y};

/// 一帧选框/拖动交互产物。
pub(crate) struct CaDragOut {
    pub ghost_notes: Vec<CaGhostNote>,
    pub hidden_notes: HashSet<(u16, u32, u8)>,
    pub drag_rect: Option<egui::Rect>,
}

/// 选框吸附后的行/tick 边界（视口局部坐标 + 行范围）。
struct SnappedBounds {
    view_sx: f32,
    view_ex: f32,
    view_sy: f32,
    view_ey: f32,
    t_start: f64,
    t_end: f64,
    row_lo: usize,
    row_hi: usize,
}

/// 垂直拖动 delta：起点/落点行的锚定轨道之差。
fn track_delta(
    origin_music_y: f32,
    current_music_y: f32,
    layout: &CaRowLayout,
    groups: &CaGroups,
) -> i32 {
    let anchor = |y: f32| {
        layout
            .hit_row(y)
            .and_then(|r| anchor_track(layout, groups, r))
    };
    match (anchor(origin_music_y), anchor(current_music_y)) {
        (Some(a), Some(b)) => b as i32 - a as i32,
        _ => 0,
    }
}

/// 选框吸附边界：tick 取吸附网格；行范围取起止音乐 y 命中的行。
fn snapped_bounds(
    start_pixel: egui::Pos2,
    end: egui::Pos2,
    view: &ArrangementView,
    layout: &CaRowLayout,
    data: &ArrangeData<'_>,
) -> Option<SnappedBounds> {
    let sx = start_pixel.x.min(end.x);
    let ex = start_pixel.x.max(end.x);
    let tick_s = view.x_to_tick(sx);
    let tick_e = view.x_to_tick(ex);
    let snapped_s =
        crate::view_interaction::snap_tick(tick_s, data.quantize, data.ppq, data.bar_line_data);
    let snapped_e =
        crate::view_interaction::snap_tick(tick_e, data.quantize, data.ppq, data.bar_line_data);
    let t_start = snapped_s.min(snapped_e);
    let mut t_end = snapped_s.max(snapped_e);
    let interval = data.quantize.tick_interval(data.ppq) as f64;
    if t_end <= t_start {
        t_end = t_start + interval.max(1.0);
    }
    if layout.is_empty() {
        return None;
    }
    let scroll_y = view.base.scroll_y;
    let my0 = start_pixel.y + scroll_y;
    let my1 = end.y + scroll_y;
    let row_lo = layout
        .hit_row(my0.min(my1).max(0.0))
        .unwrap_or(0)
        .min(layout.total_rows() - 1);
    let row_hi = layout
        .hit_row(my0.max(my1).max(0.0))
        .unwrap_or(layout.total_rows() - 1)
        .min(layout.total_rows() - 1);
    let (row_lo, row_hi) = (row_lo.min(row_hi), row_lo.max(row_hi));
    let view_sy = layout.y_top(row_lo) - scroll_y;
    let view_ey = layout.y_top(row_hi) + layout.height(row_hi) - scroll_y;
    let view_sx = view.tick_to_x(t_start);
    let view_ex = view.tick_to_x(t_end);
    Some(SnappedBounds {
        view_sx,
        view_ex,
        view_sy,
        view_ey,
        t_start,
        t_end,
        row_lo,
        row_hi,
    })
}

/// 拖动自动滚动（行方向按内容总高 clamp）。
#[allow(clippy::too_many_arguments)]
fn auto_scroll(
    ui: &mut egui::Ui,
    view: &mut ArrangementView,
    layout: &CaRowLayout,
    hit_rect: egui::Rect,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    total_ticks: f64,
) {
    let full_w = content_rect.width();
    let total_h = layout.total_height();
    crate::selection::drag::auto_scroll_on_drag(ui, &mut view.base, hit_rect, pos, |base, _, h| {
        base.clamp_scroll_x(full_w, total_ticks);
        let max_scroll_y = (total_h - h).max(0.0);
        base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
    });
}

/// 选框/拖动一帧。返回 ghost/hidden（供音符层构建）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    hit_rect: egui::Rect,
    view: &mut ArrangementView,
    layout: &CaRowLayout,
    groups: &CaGroups,
    track_rows: &[Vec<u16>],
    data: &ArrangeData<'_>,
    edit: &mut ArrangeEdit<'_>,
    chan_ar_sel: &mut Vec<ChanSelRect>,
) -> CaDragOut {
    let mut out = CaDragOut {
        ghost_notes: Vec::new(),
        hidden_notes: HashSet::new(),
        drag_rect: None,
    };

    let num_tracks = data.tracks.len() as u16;
    let sel_id = ui.id().with("ca_sel_drag");
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);
    type CaMoveDrag = ((f64, f32), (f64, f32), bool);
    let move_drag_id = ui.id().with("ca_move_drag");
    let mut move_drag: Option<CaMoveDrag> = ui
        .data_mut(|d| d.get_persisted(move_drag_id))
        .unwrap_or(None);
    let move_orig_id = ui.id().with("ca_move_orig_sel");
    let mut move_orig_sel: Vec<ChanSelRect> = ui
        .data_mut(|d| d.get_persisted(move_orig_id))
        .unwrap_or_default();
    let move_had_moved_id = ui.id().with("ca_move_had_moved");
    let mut move_had_moved: bool = ui
        .data_mut(|d| d.get_persisted(move_had_moved_id))
        .unwrap_or(false);
    type CaMoveNotes = Arc<Vec<crate::selection::drag::CollectedNote>>;
    let move_notes_id = ui.id().with("ca_move_notes");
    let mut move_notes: Option<CaMoveNotes> = ui
        .data_mut(|d| d.get_persisted(move_notes_id))
        .unwrap_or(None);
    let move_min_tick_id = ui.id().with("ca_move_min_tick");
    let mut move_min_tick: f64 = ui
        .data_mut(|d| d.get_persisted(move_min_tick_id))
        .unwrap_or(f64::INFINITY);

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
        move_notes = None;
        move_min_tick = f64::INFINITY;
    }
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(sel_id, drag));
        ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
        ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
        ui.data_mut(|d| d.insert_persisted(move_had_moved_id, move_had_moved));
        ui.data_mut(|d| d.insert_persisted(move_notes_id, move_notes));
        ui.data_mut(|d| d.insert_persisted(move_min_tick_id, move_min_tick));
        return out;
    }

    let scroll_y = view.base.scroll_y;
    let inside_sel_rect = pointer.hover_pos().is_some_and(|pos| {
        hit_rect.contains(pos) && {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            point_in_sel_rects(chan_ar_sel, layout, view, local)
        }
    });
    if inside_sel_rect && move_drag.is_none() && drag.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }
    let on_am_row = pointer.hover_pos().is_some_and(|pos| {
        hit_rect.contains(pos)
            && row_at_local_y(layout, scroll_y, pos.y - content_rect.min.y)
                .map(|r| is_am_row(layout, r))
                .unwrap_or(false)
    });

    if pointer.primary_pressed()
        && !on_am_row
        && let Some(pos) = pointer.hover_pos()
        && hit_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let click_tick = view.x_to_tick(local.x);
        let click_music_y = local.y + scroll_y;
        if inside_sel_rect && !additive {
            // 选区整体拖动：先按当前选区收集一次（下一行会清空选框）。
            move_notes = Some(Arc::new(crate::selection::drag::collect_selected_notes(
                edit.selected,
                data.midi,
                data.track_visible,
                &HashSet::new(),
                &[],
            )));
            move_orig_sel = chan_ar_sel.clone();
            chan_ar_sel.clear();
            let rect_min = move_orig_sel
                .iter()
                .map(|r| r.t_start)
                .fold(f64::INFINITY, f64::min);
            let notes_min = move_notes
                .as_deref()
                .map(|ns| {
                    ns.iter()
                        .map(|n| n.start_tick as f64)
                        .fold(f64::INFINITY, f64::min)
                })
                .unwrap_or(f64::INFINITY);
            move_min_tick = rect_min.min(notes_min);
            let origin = (click_tick, click_music_y);
            let alt = ui.input(|i| i.modifiers.alt);
            move_drag = Some((origin, origin, alt));
            drag = None;
        } else {
            drag = Some(((click_tick, click_music_y), local));
            if !additive {
                chan_ar_sel.clear();
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
        let current_music_y = local.y + scroll_y;
        move_drag = Some((origin, (current_tick, current_music_y), alt));
        auto_scroll(
            ui,
            view,
            layout,
            hit_rect,
            content_rect,
            pos,
            data.total_ticks,
        );
    }
    if let Some(((origin_t, origin_y), (current_t, current_y), alt)) = move_drag
        && !move_orig_sel.is_empty()
    {
        let dt = crate::arrange::view_ui::types::snapped_delta(
            origin_t,
            current_t,
            data.quantize,
            data.ppq,
            data.bar_line_data,
        );
        let dt = crate::arrange::view_ui::types::clamp_delta_to_zero(dt, move_min_tick);
        let dtr = track_delta(origin_y, current_y, layout, groups);
        *chan_ar_sel = move_orig_sel
            .iter()
            .map(|s| ChanSelRect {
                t_start: s.t_start + dt as f64,
                t_end: s.t_end + dt as f64,
                tracks: s
                    .tracks
                    .iter()
                    .map(|&t| (t as i32 + dtr).clamp(0, num_tracks.saturating_sub(1) as i32) as u16)
                    .collect(),
            })
            .collect();
        if dt != 0 || dtr != 0 {
            move_had_moved = true;
        }
        {
            let notes = move_notes.as_deref().map(Vec::as_slice).unwrap_or(&[]);
            let tick_range = view.visible_tick_range(content_rect.width());
            let (g, h) = crate::arrange::view_ui::types::build_ghosts_for_move(
                dt, dtr, alt, notes, tick_range, num_tracks,
            );
            for (s, e, k, t) in g {
                if let Some(rows) = track_rows.get(t as usize) {
                    for &row in rows {
                        out.ghost_notes.push((s, e, k, row));
                    }
                }
            }
            out.hidden_notes.extend(h);
        }
    }
    if move_drag.is_some() && pointer.primary_released() {
        if let Some(((origin_t, origin_y), (current_t, current_y), alt)) = move_drag {
            let delta_ticks = crate::arrange::view_ui::types::snapped_delta(
                origin_t,
                current_t,
                data.quantize,
                data.ppq,
                data.bar_line_data,
            );
            let delta_ticks =
                crate::arrange::view_ui::types::clamp_delta_to_zero(delta_ticks, move_min_tick);
            let delta_tracks = track_delta(origin_y, current_y, layout, groups);
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
                *chan_ar_sel = move_orig_sel
                    .iter()
                    .map(|s| ChanSelRect {
                        t_start: s.t_start + delta_ticks as f64,
                        t_end: s.t_end + delta_ticks as f64,
                        tracks: s
                            .tracks
                            .iter()
                            .map(|&t| {
                                (t as i32 + delta_tracks)
                                    .clamp(0, num_tracks.saturating_sub(1) as i32)
                                    as u16
                            })
                            .collect(),
                    })
                    .collect();
                view.base.dirty = true;
            } else {
                *chan_ar_sel = move_orig_sel.clone();
            }
        }
        move_drag = None;
        move_orig_sel.clear();
        move_had_moved = false;
        move_notes = None;
        move_min_tick = f64::INFINITY;
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
            auto_scroll(
                ui,
                view,
                layout,
                hit_rect,
                content_rect,
                pos,
                data.total_ticks,
            );
        }
        let start_pixel = egui::pos2(view.tick_to_x(start_music.0), start_music.1 - scroll_y);
        if let Some((_, end)) = drag
            && (end - start_pixel).length() >= 3.0
            && let Some(b) = snapped_bounds(start_pixel, end, view, layout, data)
        {
            out.drag_rect = Some(egui::Rect::from_min_max(
                egui::pos2(b.view_sx.min(b.view_ex), b.view_sy.min(b.view_ey)),
                egui::pos2(b.view_sx.max(b.view_ex), b.view_sy.max(b.view_ey)),
            ));
        }
        if pointer.primary_released() {
            let release_end = pointer.hover_pos().map(|pos| {
                let clamped = pos.clamp(hit_rect.min, hit_rect.max);
                egui::pos2(
                    clamped.x - content_rect.min.x,
                    clamped.y - content_rect.min.y,
                )
            });
            if let Some((_, end)) = drag {
                let end = release_end.unwrap_or(end);
                let drag_dist = (end - start_pixel).length();
                if drag_dist < 3.0 {
                    // 单击：移动播放光标 + 选中行所属轨道（主音轨跟随点选）。
                    let tick = view.x_to_tick(start_pixel.x);
                    let snapped = crate::view_interaction::snap_tick(
                        tick,
                        data.quantize,
                        data.ppq,
                        data.bar_line_data,
                    );
                    edit.selected.clear();
                    chan_ar_sel.clear();
                    *edit.cursor_tick = Some(snapped.max(0.0));
                    let row_tracks = layout
                        .hit_row(start_music.1)
                        .and_then(|r| layout.row(r))
                        .map(|row| row_tracks(row, groups))
                        .unwrap_or_default();
                    if !row_tracks.is_empty() {
                        edit.track_selected.clear();
                        for &t in &row_tracks {
                            edit.track_selected.insert(t);
                        }
                        *edit.selection_anchor = Some(row_tracks[0]);
                        *edit.info_content = Some(crate::right_panel::InfoContent::Track);
                    }
                } else if let Some(b) = snapped_bounds(start_pixel, end, view, layout, data) {
                    let tracks = rows_range_tracks(layout, b.row_lo..b.row_hi + 1, groups);
                    if !tracks.is_empty() {
                        if !additive {
                            edit.selected.clear();
                            chan_ar_sel.clear();
                        }
                        for (lo, hi) in track_runs(&tracks) {
                            edit.selected.add_rect_track(
                                b.t_start as u32,
                                b.t_end as u32,
                                0,
                                yinhe_types::MAX_KEY,
                                lo,
                                hi,
                            );
                        }
                        if let Some(midi) = data.midi {
                            edit.selected.materialize_pending(midi);
                        }
                        edit.selected.materialize_automation_pending(data.tracks);
                        chan_ar_sel.push(ChanSelRect {
                            t_start: b.t_start,
                            t_end: b.t_end,
                            tracks,
                        });
                    } else if !additive {
                        edit.selected.clear();
                        chan_ar_sel.clear();
                    }
                    view.base.dirty = true;
                } else if !additive {
                    edit.selected.clear();
                    chan_ar_sel.clear();
                }
            }
            drag = None;
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
    ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
    ui.data_mut(|d| d.insert_persisted(move_had_moved_id, move_had_moved));
    ui.data_mut(|d| d.insert_persisted(move_notes_id, move_notes));
    ui.data_mut(|d| d.insert_persisted(move_min_tick_id, move_min_tick));
    out
}
