//! 单音符边缘伸缩状态机（不用先选中，与铅笔一致）：release 复用铅笔的 `PencilNoteDrag` 通道提交。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::interact::{clamped_local, drag_scroll_and_clamp};
use super::state::SelDragFrameState;
use crate::selection::drag::{compute_resize_dt, main_cross_x_y, main_px_to_tick_dir};

/// 单音符边缘伸缩状态机（不用先选中，与铅笔一致）：
/// release 复用铅笔的 `PencilNoteDrag` 通道提交。
#[allow(clippy::too_many_arguments)]
pub(crate) fn single_note_resize_frame(
    ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    pencil_note_drag: &mut Option<yinhe_types::PencilNoteDrag>,
    pointer: &egui::PointerState,
) {
    // ── Single-note edge resize: 直接伸缩音符（不用先选中，与铅笔一致）──
    if let Some((side, trk, orig_start, orig_end, orig_key)) = state.sel_note_resize {
        let (boundary_tick, other_tick) = match side {
            ResizeSide::Right => (orig_end as f64, orig_start as f64),
            ResizeSide::Left => (orig_start as f64, orig_end as f64),
        };
        // Drag：实时显示 ghost + hidden
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            drag_scroll_and_clamp(ui, view, content_rect, music_rect, total_ticks, pos);

            let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
            let (main_px, _) = main_cross_x_y(view, (local_x, local_y));
            let raw_tick = main_px_to_tick_dir(view, main_px);
            let (new_boundary, _dt) = compute_resize_dt(
                raw_tick,
                side,
                boundary_tick,
                other_tick,
                quantize,
                ppq,
                bar_line_data,
            );

            // ghost = 新形状，hidden = 原音符
            match side {
                ResizeSide::Right => {
                    state
                        .ghost_notes
                        .push((orig_start, new_boundary as u32, orig_key, trk));
                }
                ResizeSide::Left => {
                    state
                        .ghost_notes
                        .push((new_boundary as u32, orig_end, orig_key, trk));
                }
            }
            state.hidden_notes.push((trk, orig_start, orig_key));

            // ── Tooltip：显示 ±gate ──
            let orig_gate = orig_end as i64 - orig_start as i64;
            let new_gate = match side {
                ResizeSide::Right => new_boundary as i64 - orig_start as i64,
                ResizeSide::Left => orig_end as i64 - new_boundary as i64,
            };
            let lines = vec![crate::view_interaction::format_signed(
                "gate",
                new_gate - orig_gate,
            )];
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
            ui.ctx().request_repaint();
        }
        // Release：提交单音符伸缩（复用铅笔的 PencilNoteDrag 通道）
        if pointer.primary_released() {
            if let Some(pos) = pointer.hover_pos() {
                let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
                let (main_px, _) = main_cross_x_y(view, (local_x, local_y));
                let raw_tick = main_px_to_tick_dir(view, main_px);
                let (new_boundary, _dt) = compute_resize_dt(
                    raw_tick,
                    side,
                    boundary_tick,
                    other_tick,
                    quantize,
                    ppq,
                    bar_line_data,
                );
                match side {
                    ResizeSide::Right => {
                        *pencil_note_drag = Some(yinhe_types::PencilNoteDrag::ResizeRight {
                            track: trk,
                            start_tick: orig_start,
                            key: orig_key,
                            new_end_tick: new_boundary as u32,
                        });
                        // Keep ghost/hidden alive on the release frame
                        state
                            .ghost_notes
                            .push((orig_start, new_boundary as u32, orig_key, trk));
                    }
                    ResizeSide::Left => {
                        *pencil_note_drag = Some(yinhe_types::PencilNoteDrag::ResizeLeft {
                            track: trk,
                            start_tick: orig_start,
                            key: orig_key,
                            new_start_tick: new_boundary as u32,
                        });
                        state
                            .ghost_notes
                            .push((new_boundary as u32, orig_end, orig_key, trk));
                    }
                }
                state.hidden_notes.push((trk, orig_start, orig_key));
            }
            state.preview_reqs.push(crate::piano_view::PreviewReq::Stop);
            state.sel_note_resize = None;
        }
    }
}
