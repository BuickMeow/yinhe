//! 单音符移动状态机（不用先选中，与铅笔一致）：release 提交 `PencilNoteDrag::Move`，Alt 复制走 `note_drag_delta` 通道。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::interact::{clamped_local, drag_scroll_and_clamp};
use super::state::SelDragFrameState;
use crate::selection::drag::{main_cross_x_y, main_px_to_tick_dir};

/// 单音符移动状态机（不用先选中，与铅笔一致）：
/// release 提交 `PencilNoteDrag::Move`，Alt 拖拽复制走 `note_drag_delta` 通道。
#[allow(clippy::too_many_arguments)]
pub(crate) fn single_note_move_frame(
    ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    midi: Option<&dyn yinhe_types::NoteSource>,
    selected: &mut yinhe_core::Selection,
    vertical: bool,
    note_drag_delta: &mut Option<(i64, i32, bool)>,
    pencil_note_drag: &mut Option<yinhe_types::PencilNoteDrag>,
    pointer: &egui::PointerState,
) {
    // ── Single-note move: 直接拖动未选中音符（不用先选中，与铅笔一致）──
    if let Some((trk, orig_start, orig_key, orig_end, press_tick, last_dk, alt)) =
        state.sel_note_move
    {
        // Drag：实时显示 ghost + hidden + tooltip
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            drag_scroll_and_clamp(ui, view, content_rect, music_rect, total_ticks, pos);

            let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
            let (main_px, cross_px) = main_cross_x_y(view, (local_x, local_y));
            let raw_tick = main_px_to_tick_dir(view, main_px);
            let snapped_tick =
                crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
            let dt = (snapped_tick - press_tick).round() as i64;
            // 垂直选框工具：只能水平移动，dk 强制为 0
            let dk = if vertical {
                0
            } else {
                view.cross_px_to_key(cross_px) as i32 - orig_key as i32
            };

            if dt != 0 || dk != 0 {
                state.single_note_had_moved = true;
            }
            let new_start = (orig_start as i64 + dt).max(0) as u32;
            let new_key = (orig_key as i32 + dk).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
            state
                .ghost_notes
                .push((new_start, new_start + (orig_end - orig_start), new_key, trk));
            // Alt（复制模式）：原音符保留可见，不 push hidden_notes。
            if !alt {
                state.hidden_notes.push((trk, orig_start, orig_key));
            }

            // 音符预览：每变化 1 key 触发一次（gate 长度，原力度）。
            // vel <= 1 的音符（黑乐谱隐藏音符）不预览，与播放筛除一致。
            if dk != last_dk {
                state.sel_note_move =
                    Some((trk, orig_start, orig_key, orig_end, press_tick, dk, alt));
                if let Some(vel) =
                    crate::piano_view::pencil::note_velocity(midi, trk, orig_start, orig_key)
                    && vel > 1
                {
                    state.preview_reqs.push(crate::piano_view::PreviewReq::Note(
                        crate::piano_view::NotePreview {
                            track: trk,
                            key: new_key,
                            velocity: Some(vel),
                            target_tick: new_start,
                            duration_ticks: orig_end - orig_start,
                        },
                    ));
                }
            }

            // ── Tooltip：显示 ±tick / ±key（已按量化 snap）──
            let lines = vec![
                crate::view_interaction::format_signed("tick", dt),
                crate::view_interaction::format_signed("key", dk as i64),
            ];
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
            ui.ctx().request_repaint();
        }
        // Release：提交单音符移动（复用铅笔的 PencilNoteDrag 通道）
        if pointer.primary_released() {
            let mut dt = 0i64;
            let mut dk = 0i32;
            if let Some(pos) = pointer.hover_pos() {
                let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
                let (main_px, cross_px) = main_cross_x_y(view, (local_x, local_y));
                let raw_tick = main_px_to_tick_dir(view, main_px);
                let snapped_tick =
                    crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                dt = (snapped_tick - press_tick).round() as i64;
                dk = if vertical {
                    0
                } else {
                    view.cross_px_to_key(cross_px) as i32 - orig_key as i32
                };
                if dt != 0 || dk != 0 {
                    state.single_note_had_moved = true;
                }
            }
            let had_moved = state.single_note_had_moved;
            if alt {
                if had_moved {
                    // Alt = 复制：先把该音符置为唯一选中，再走选区复制通道
                    // （duplicate_selected_to 复制后选区跟随副本，便于连续 Alt+拖动）。
                    selected.clear();
                    selected.add_rect_track(orig_start, orig_end, orig_key, orig_key, trk, trk);
                    *note_drag_delta = Some((dt, dk, true));
                    // Keep ghost alive on the release frame
                    let new_start = (orig_start as i64 + dt).max(0) as u32;
                    let new_key =
                        (orig_key as i32 + dk).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                    state.ghost_notes.push((
                        new_start,
                        new_start + (orig_end - orig_start),
                        new_key,
                        trk,
                    ));
                }
            } else {
                *pencil_note_drag = Some(yinhe_types::PencilNoteDrag::Move {
                    track: trk,
                    start_tick: orig_start,
                    key: orig_key,
                    delta_ticks: dt,
                    delta_keys: dk,
                });
                // Keep ghost/hidden alive on the release frame
                let new_start = (orig_start as i64 + dt).max(0) as u32;
                let new_key = (orig_key as i32 + dk).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                state.ghost_notes.push((
                    new_start,
                    new_start + (orig_end - orig_start),
                    new_key,
                    trk,
                ));
                state.hidden_notes.push((trk, orig_start, orig_key));
            }
            state.preview_reqs.push(crate::piano_view::PreviewReq::Stop);
            state.sel_note_move = None;
            state.single_note_had_moved = false;
        }
    }
}
