use std::collections::HashSet;

use eframe::egui;

use yinhe_types::{ArRow, ArRowLayout, ArrangementView};

use crate::piano_view::drag::{GhostNote, HiddenNote};

/// 吸附后的选框边界：view 局部坐标 + tick/track 范围。
pub(crate) struct ArrSnappedBounds {
    pub view_sx: f32,
    pub view_ex: f32,
    pub view_sy: f32,
    pub view_ey: f32,
    pub t_start: f64,
    pub t_end: f64,
    pub track_lo: usize,
    pub track_hi: usize,
}

/// Compute snapped selection bounds for arrangement.
/// 行号（均匀行空间）→ 音轨索引用 ArRowLayout 换算：AM 子行归到所属音轨。
pub(crate) fn arrange_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    vertical: bool,
) -> Option<ArrSnappedBounds> {
    let sx = start.x.min(end.x);
    let ex = start.x.max(end.x);
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
    if data.num_tracks == 0 {
        return None;
    }
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    let (track_lo, track_hi, view_sy, view_ey) = if vertical {
        let th = data.num_tracks - 1;
        (0, th, 0.0, data.num_tracks as f32 * lh - scroll_y)
    } else {
        let sy = start.y.min(end.y);
        let ey = start.y.max(end.y);
        let row_lo = ((scroll_y + sy) / lh).floor().max(0.0) as usize;
        let row_hi = ((scroll_y + ey) / lh).floor().max(0.0) as usize;
        if row_lo >= row_layout.total_rows() {
            return None;
        }
        let track_lo = row_layout.row_hit(row_lo).map(|h| h.track()).unwrap_or(0);
        let track_hi = row_layout
            .row_hit(row_hi.min(row_layout.total_rows().saturating_sub(1)))
            .map(|h| h.track())
            .unwrap_or(0);
        let view_sy = row_layout.track_y(track_lo, lh) - scroll_y;
        let view_ey =
            row_layout.track_y(track_hi, lh) + row_layout.track_height(track_hi, lh) - scroll_y;
        (track_lo, track_hi, view_sy, view_ey)
    };
    let view_sx = view.tick_to_x(t_start);
    let view_ex = view.tick_to_x(t_end);
    Some(ArrSnappedBounds {
        view_sx,
        view_ex,
        view_sy,
        view_ey,
        t_start,
        t_end,
        track_lo,
        track_hi,
    })
}

pub(crate) fn is_inside_sel_rect(
    arr_sel_rect: &[(f64, f64, usize, usize)],
    hover_pos: Option<egui::Pos2>,
    content_rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
) -> bool {
    arr_sel_rect
        .iter()
        .any(|&(t_start, t_end, track_lo, track_hi)| {
            hover_pos.is_some_and(|pos| {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                let lh = view.lane_height();
                let scroll_y = view.base.scroll_y;
                let sy = row_layout.track_y(track_lo, lh) - scroll_y;
                let ey = row_layout.track_y(track_hi, lh) + row_layout.track_height(track_hi, lh)
                    - scroll_y;
                let sx = view.tick_to_x(t_start);
                let ex = view.tick_to_x(t_end);
                let rect = egui::Rect::from_min_max(
                    egui::pos2(sx.min(ex), sy.min(ey)),
                    egui::pos2(sx.max(ex), sy.max(ey)),
                );
                rect.contains(local)
            })
        })
}

pub(crate) fn is_on_am_row(
    hover_pos: Option<egui::Pos2>,
    hit_rect: egui::Rect,
    content_rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    conductor_track_idx: Option<u16>,
) -> bool {
    hover_pos.is_some_and(|pos| {
        hit_rect.contains(pos)
            && match row_layout.hit_at_music_y(
                pos.y - content_rect.min.y + view.base.scroll_y,
                view.lane_height(),
            ) {
                Some(ArRow::Automation(..)) => true,
                Some(ArRow::Track(t)) => conductor_track_idx == Some(t as u16),
                None => false,
            }
    })
}

pub(crate) fn build_ghosts_for_move(
    dt: i64,
    dtr: i32,
    alt: bool,
    data: &crate::arrange::ArrangeData<'_>,
    selected: &yinhe_core::Selection,
) -> (Vec<GhostNote>, HashSet<HiddenNote>) {
    let mut ghosts = Vec::new();
    let mut hidden = HashSet::new();
    if dt == 0 && dtr == 0 {
        return (ghosts, hidden);
    }
    let max_track = (data.num_tracks as i32 - 1).max(0) as u16;
    let notes = crate::selection::drag::collect_selected_notes(
        selected,
        data.midi,
        data.track_visible,
        &HashSet::new(),
    );
    for note in notes {
        let new_tick = (note.start_tick as i64 + dt).max(0) as u32;
        let len = note.end_tick - note.start_tick;
        let new_track = (note.track as i32 + dtr).max(0).min(max_track as i32) as u16;
        ghosts.push((new_tick, new_tick + len, note.key, new_track));
        if !alt {
            hidden.insert((note.track, note.start_tick, note.key));
        }
    }
    (ghosts, hidden)
}

pub(crate) fn auto_scroll_arrange(
    ui: &mut egui::Ui,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    hit_rect: egui::Rect,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    total_ticks: f64,
) {
    let lh = view.lane_height();
    let full_w = content_rect.width();
    crate::selection::drag::auto_scroll_on_drag(ui, &mut view.base, hit_rect, pos, |base, _, h| {
        base.clamp_scroll_x(full_w, total_ticks);
        let max_scroll_y = (row_layout.total_rows() as f32 * lh - h).max(0.0);
        base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
    });
}

pub(crate) fn snapped_delta(
    origin_tick: f64,
    current_tick: f64,
    quantize: yinhe_editor_core::quantize::QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[yinhe_types::TimeSigEvent])>,
) -> i64 {
    let a = crate::view_interaction::snap_tick(origin_tick, quantize, ppq, bar_line_data);
    let b = crate::view_interaction::snap_tick(current_tick, quantize, ppq, bar_line_data);
    (b - a).round() as i64
}

pub(crate) fn delta_tracks(origin_tr: f32, current_tr: f32, vertical: bool) -> i32 {
    if vertical {
        0
    } else {
        (current_tr - origin_tr).round() as i32
    }
}
