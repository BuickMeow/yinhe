//! 选区右缘整体变速 / 右下角 Riff 重复状态机。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::interact::{clamped_local, drag_scroll_and_clamp};
use super::state::SelDragFrameState;
use crate::selection::drag::{main_cross_x_y, main_px_to_tick_dir};

fn snapped_main_tick(
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f64, f32) {
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let raw = main_px_to_tick_dir(view, main_px);
    let tick = crate::view_interaction::snap_tick(raw, quantize, ppq, bar_line_data);
    (tick, cross_px)
}

/// 选区右缘整体变速：左边界固定，右边界按量化跟随指针。
#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_scale_frame(
    ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    selection_scale_delta: &mut Option<u64>,
    pointer: &egui::PointerState,
) {
    let Some((t0, t1)) = state.sel_scale_state else {
        return;
    };
    let interval = quantize.tick_interval(ppq).max(1) as f64;
    let old_span = (t1 - t0).max(1.0);
    let min_span = old_span.min(interval).max(1.0);

    if pointer.primary_down()
        && !pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
    {
        drag_scroll_and_clamp(ui, view, content_rect, music_rect, total_ticks, pos);
        let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
        let local = egui::pos2(local_x, local_y);
        let (tick, _) = snapped_main_tick(view, local, quantize, ppq, bar_line_data);
        let new_span = (tick - t0).max(min_span);
        let dt = (new_span - old_span).round() as i64;
        sel_rect.update_resize(dt);

        // ghost：选中音符相对 t0 等比缩放，原位置隐藏。
        state.ghost_notes.clear();
        state.hidden_notes.clear();
        if let Some(notes) = state.drag_notes.as_ref() {
            let factor = new_span / old_span;
            let (tick_lo, tick_hi) = view.visible_tick_range(music_rect.width());
            let (key_lo, key_hi) = view.visible_cross_range(
                view.cross_axis_len(content_rect.width(), content_rect.height()),
            );
            let scale = |v: f64| (t0 + (v - t0) * factor).round().max(t0);
            for info in notes.iter() {
                if info.key < key_lo || info.key > key_hi {
                    continue;
                }
                let new_start = scale(f64::from(info.start_tick)) as u32;
                let new_end =
                    (scale(f64::from(info.end_tick)) as u32).max(new_start.saturating_add(1));
                if f64::from(new_start) >= tick_hi || f64::from(new_end) <= tick_lo {
                    continue;
                }
                state
                    .ghost_notes
                    .push((new_start, new_end, info.key, info.track));
                if f64::from(info.start_tick) < tick_hi && f64::from(info.end_tick) > tick_lo {
                    state
                        .hidden_notes
                        .push((info.track, info.start_tick, info.key));
                }
            }
        }
        crate::view_interaction::draw_hover_tooltip(
            ui.ctx(),
            &[format!("span {}", new_span.round() as i64)],
            pos.x,
            pos.y,
        );
        ui.ctx().request_repaint();
    }

    if pointer.primary_released() {
        if let Some(pos) = pointer.hover_pos() {
            let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
            let local = egui::pos2(local_x, local_y);
            let (tick, _) = snapped_main_tick(view, local, quantize, ppq, bar_line_data);
            let new_span = (tick - t0).max(min_span);
            *selection_scale_delta = Some(new_span.round().max(1.0) as u64);
        }
        sel_rect.cancel_resize();
        state.sel_scale_state = None;
        state.drag_notes = None;
    }
}

/// 右下角 Riff：按选区宽度/高度计算重复步数，松手交给 App 提交一次 undo。
#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_riff_frame(
    _ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    _music_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    riff_repeat: &mut Option<(i64, i64)>,
    pointer: &egui::PointerState,
) {
    let Some((t0, t1, k0, k1)) = state.sel_riff_state else {
        return;
    };
    if pointer.primary_released() {
        if let Some(pos) = pointer.hover_pos() {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let (tick, cross_px) = snapped_main_tick(view, local, quantize, ppq, bar_line_data);
            let width = (t1 - t0).max(1.0);
            let sx = if tick > t1 {
                ((tick - t1) / width).ceil() as i64
            } else if tick < t0 {
                -((t0 - tick) / width).ceil() as i64
            } else {
                0
            };
            let span = (i64::from(k1) - i64::from(k0) + 1).max(1);
            let key = i64::from(view.cross_px_to_key(cross_px));
            let sy = if key > i64::from(k1) {
                ((key - i64::from(k1)) as f64 / span as f64).ceil() as i64
            } else if key < i64::from(k0) {
                -((i64::from(k0) - key) as f64 / span as f64).ceil() as i64
            } else {
                0
            };
            if sx != 0 || sy != 0 {
                *riff_repeat = Some((sx, sy));
            }
        }
        state.sel_riff_state = None;
    }
}
