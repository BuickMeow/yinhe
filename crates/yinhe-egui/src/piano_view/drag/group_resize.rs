//! 选区边缘缩放状态机（sel_resize）：拖拽中更新 ghost/选框，release 提交 `note_resize_delta`。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::interact::{clamped_local, drag_scroll_and_clamp};
use super::state::SelDragFrameState;
use crate::selection::drag::{compute_resize_dt, main_cross_x_y, main_px_to_tick_dir};

/// 选区边缘缩放状态机（sel_resize）：拖拽中更新 ghost/选框，
/// release 提交 `note_resize_delta`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_resize_frame(
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
    note_resize_delta: &mut Option<(ResizeSide, i64)>,
    pointer: &egui::PointerState,
) {
    // ── Resize drag: 边缘拖动伸缩选中音符 ──
    if let Some((side, origin_boundary_tick, other_boundary_tick)) = state.sel_resize_state
        && let Some(ref notes) = state.drag_notes
    {
        // Drag：实时显示 ghost + 更新 sel_rect
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            drag_scroll_and_clamp(ui, view, content_rect, music_rect, total_ticks, pos);

            let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
            let (main_px, _) = main_cross_x_y(view, (local_x, local_y));
            let raw_tick = main_px_to_tick_dir(view, main_px);
            let (_new_boundary, dt) = compute_resize_dt(
                raw_tick,
                side,
                origin_boundary_tick,
                other_boundary_tick,
                quantize,
                ppq,
                bar_line_data,
            );

            // 生成 ghost/hidden：每个音符独立 clamp（end > start + 1）。
            // 视口裁剪：视口外的 ghost/hidden 不影响渲染（每帧重建）。
            let main_len = view.main_axis_len(content_rect.width(), content_rect.height());
            let (tick_lo, tick_hi) = view.visible_main_range(main_len);
            let cross_len = view.cross_axis_len(content_rect.width(), content_rect.height());
            let (key_lo, key_hi) = view.visible_cross_range(cross_len);
            for info in notes.iter() {
                if info.key < key_lo || info.key > key_hi {
                    continue;
                }
                match side {
                    ResizeSide::Right => {
                        let new_end =
                            (info.end_tick as i64 + dt).max(info.start_tick as i64 + 1) as u32;
                        if (new_end as f64) > tick_lo && (info.start_tick as f64) < tick_hi {
                            state.ghost_notes.push((
                                info.start_tick,
                                new_end,
                                info.key,
                                info.track,
                            ));
                        }
                        if (info.start_tick as f64) < tick_hi && (info.end_tick as f64) > tick_lo {
                            state
                                .hidden_notes
                                .push((info.track, info.start_tick, info.key));
                        }
                    }
                    ResizeSide::Left => {
                        let new_start = (info.start_tick as i64 + dt)
                            .max(0)
                            .min(info.end_tick as i64 - 1)
                            as u32;
                        if (info.end_tick as f64) > tick_lo && (new_start as f64) < tick_hi {
                            state.ghost_notes.push((
                                new_start,
                                info.end_tick,
                                info.key,
                                info.track,
                            ));
                        }
                        if (info.start_tick as f64) < tick_hi && (info.end_tick as f64) > tick_lo {
                            state
                                .hidden_notes
                                .push((info.track, info.start_tick, info.key));
                        }
                    }
                }
            }

            sel_rect.update_resize(dt);

            // ── Tooltip：显示 ±gate（gate 变化量：Left 时 start 偏移 dt，gate 变化 = -dt）──
            let gate_delta = match side {
                ResizeSide::Left => -dt,
                ResizeSide::Right => dt,
            };
            let lines = vec![crate::view_interaction::format_signed("gate", gate_delta)];
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
            ui.ctx().request_repaint();
        }
        // Release：提交 dt
        if pointer.primary_released() {
            if let Some(pos) = pointer.hover_pos() {
                let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
                let (main_px, _) = main_cross_x_y(view, (local_x, local_y));
                let raw_tick = main_px_to_tick_dir(view, main_px);
                let (_new_boundary, dt) = compute_resize_dt(
                    raw_tick,
                    side,
                    origin_boundary_tick,
                    other_boundary_tick,
                    quantize,
                    ppq,
                    bar_line_data,
                );
                *note_resize_delta = Some((side, dt));
                sel_rect.update_resize(dt);

                // Keep ghost/hidden alive on the release frame（同样视口裁剪）
                let main_len = view.main_axis_len(content_rect.width(), content_rect.height());
                let (tick_lo, tick_hi) = view.visible_main_range(main_len);
                let cross_len = view.cross_axis_len(content_rect.width(), content_rect.height());
                let (key_lo, key_hi) = view.visible_cross_range(cross_len);
                for info in notes.iter() {
                    if info.key < key_lo || info.key > key_hi {
                        continue;
                    }
                    match side {
                        ResizeSide::Right => {
                            let new_end =
                                (info.end_tick as i64 + dt).max(info.start_tick as i64 + 1) as u32;
                            if (new_end as f64) > tick_lo && (info.start_tick as f64) < tick_hi {
                                state.ghost_notes.push((
                                    info.start_tick,
                                    new_end,
                                    info.key,
                                    info.track,
                                ));
                            }
                            if (info.start_tick as f64) < tick_hi
                                && (info.end_tick as f64) > tick_lo
                            {
                                state
                                    .hidden_notes
                                    .push((info.track, info.start_tick, info.key));
                            }
                        }
                        ResizeSide::Left => {
                            let new_start = (info.start_tick as i64 + dt)
                                .max(0)
                                .min(info.end_tick as i64 - 1)
                                as u32;
                            if (info.end_tick as f64) > tick_lo && (new_start as f64) < tick_hi {
                                state.ghost_notes.push((
                                    new_start,
                                    info.end_tick,
                                    info.key,
                                    info.track,
                                ));
                            }
                            if (info.start_tick as f64) < tick_hi
                                && (info.end_tick as f64) > tick_lo
                            {
                                state
                                    .hidden_notes
                                    .push((info.track, info.start_tick, info.key));
                            }
                        }
                    }
                }
            }
            sel_rect.end_resize();
            state.sel_resize_state = None;
            state.drag_notes = None;
        }
    }
}
