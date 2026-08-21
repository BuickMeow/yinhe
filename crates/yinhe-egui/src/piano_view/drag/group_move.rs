//! 选区整体移动状态机（note_drag）：拖拽中更新 ghost/预览/选框，release 提交 delta。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::interact::{clamped_local, drag_scroll_and_clamp};
use super::state::SelDragFrameState;
use crate::selection::drag::{main_cross_x_y, main_px_to_tick_dir};

/// 选区整体移动状态机（note_drag）：拖拽中更新 ghost/预览/选框，
/// release 提交 delta（普通移动与 Alt 复制共用 `note_drag_delta` 通道）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn note_drag_frame(
    ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    vertical: bool,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    note_drag_delta: &mut Option<(i64, i32, bool)>,
    pointer: &egui::PointerState,
) {
    // Note drag: use pre-computed data for ghost/hidden, store delta only on release
    if let Some((origin_tick, origin_key, alt)) = state.note_drag_origin
        && let Some(ref notes) = state.drag_notes
    {
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
            let current_key = view.cross_px_to_key(cross_px) as f64;
            let dt = (snapped_tick - origin_tick).round() as i64;
            // 垂直选框（垂直工具或空区域框选自动生成的全键选框）：只能水平移动，dk 强制为 0
            let dk = if vertical || sel_rect.has_auto_vertical() {
                0
            } else {
                (current_key - origin_key).round() as i32
            };

            if dt != 0 || dk != 0 {
                state.note_drag_had_moved = true;
            }

            // O(N) — just apply delta to pre-computed data, no midi lookup.
            // Alt（复制模式）：原音符保留可见，不 push hidden_notes。
            for info in notes {
                let new_tick = (info.start_tick as i64 + dt).max(0) as u32;
                let new_key = ((info.key as i32) + dk).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                let length = info.end_tick - info.start_tick;
                state
                    .ghost_notes
                    .push((new_tick, new_tick + length, new_key, info.track));
                if !alt {
                    state
                        .hidden_notes
                        .push((info.track, info.start_tick, info.key));
                }
            }

            sel_rect.update_drag(dt, dk);

            // 音符听觉预览：每变化 1 key，播放一次整组选中音符（各自通道/力度，
            // 长度 = 音符 gate，时长换算用目标位置 Tempo）。
            if dk != state.preview_last_dk {
                state.preview_last_dk = dk;
                // vel <= 1 的音符（黑乐谱隐藏音符）不预览，与播放筛除一致。
                state.preview_reqs = notes
                    .iter()
                    .filter(|info| info.velocity > 1)
                    .map(|info| {
                        crate::piano_view::PreviewReq::Note(crate::piano_view::NotePreview {
                            track: info.track,
                            key: ((info.key as i32) + dk).clamp(0, yinhe_types::MAX_KEY as i32)
                                as u8,
                            velocity: Some(info.velocity),
                            target_tick: (info.start_tick as i64 + dt).max(0) as u32,
                            duration_ticks: info.end_tick - info.start_tick,
                        })
                    })
                    .collect();
                ui.data_mut(|d| {
                    d.insert_persisted(ui.id().with("note_drag_preview_dk"), state.preview_last_dk)
                });
            }

            // ── Tooltip：显示 ±tick / ±key（已按量化 snap）──
            let lines = vec![
                crate::view_interaction::format_signed("tick", dt),
                crate::view_interaction::format_signed("key", dk as i64),
            ];
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
            ui.ctx().request_repaint();
        }
        if pointer.primary_released() {
            let mut dt = 0i64;
            let mut dk = 0i32;
            if let Some(pos) = pointer.hover_pos() {
                let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
                let (main_px, cross_px) = main_cross_x_y(view, (local_x, local_y));
                let raw_tick = main_px_to_tick_dir(view, main_px);
                let snapped_tick =
                    crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let current_key = view.cross_px_to_key(cross_px) as f64;
                dt = (snapped_tick - origin_tick).round() as i64;
                dk = if vertical || sel_rect.has_auto_vertical() {
                    0
                } else {
                    (current_key - origin_key).round() as i32
                };
                if dt != 0 || dk != 0 {
                    state.note_drag_had_moved = true;
                }
            }
            let had_moved = state.note_drag_had_moved;
            if alt && !had_moved {
                // 纯点击不复制
                sel_rect.cancel_drag();
            } else {
                *note_drag_delta = Some((dt, dk, alt));
                sel_rect.update_drag(dt, dk);
                let has_notes = !notes.is_empty();
                if has_notes {
                    for info in notes {
                        let new_tick = (info.start_tick as i64 + dt).max(0) as u32;
                        let new_key =
                            ((info.key as i32) + dk).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                        let length = info.end_tick - info.start_tick;
                        state
                            .ghost_notes
                            .push((new_tick, new_tick + length, new_key, info.track));
                        if !alt {
                            state
                                .hidden_notes
                                .push((info.track, info.start_tick, info.key));
                        }
                    }
                    state.preview_reqs.push(crate::piano_view::PreviewReq::Stop);
                }
                sel_rect.end_drag();
            }
            state.note_drag_origin = None;
            state.drag_notes = None;
            state.note_drag_had_moved = false;
        }
    }
}
