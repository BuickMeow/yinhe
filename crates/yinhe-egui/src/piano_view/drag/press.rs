//! 选择工具 press 分发：音符 hit-test → 单音符伸缩/移动；选框边缘 → 选区缩放；选框内 → 选区整体移动。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::hit::hit_test_note;
use super::state::SelDragFrameState;
use crate::selection::drag::{
    collect_selected_notes, hit_test_sel_edge, main_cross_x_y, main_px_to_tick_dir,
};

/// Press 帧分发：音符 hit-test → 单音符伸缩/移动；选框边缘 → 选区缩放；
/// 选框内 → 选区整体移动；选框外（非加选）→ 清空选框与选区。
/// marquee 的启动在共享的 `marquee_drag_frame`，不在此处。
#[allow(clippy::too_many_arguments)]
pub(crate) fn sel_press(
    ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    midi: Option<&dyn yinhe_types::NoteSource>,
    selected: &mut yinhe_core::Selection,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    eff_rects: &[(f64, f64, u8, u8)],
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    additive: bool,
    press_on_bar: bool,
    pointer: &egui::PointerState,
    can_edit: bool,
) {
    // Start drag (note drag only — marquee is handled by shared function below)
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let on_bar = press_on_bar;

        if on_bar {
            // Don't start drag, don't clear anything — let the button handle it.
        } else {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            // 点击位置是否在某个选框内（音符 hit-test 与选区移动共用）。
            let in_sel_rect = eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
                let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                    view, t_start, t_end, key_lo, key_hi,
                );
                pixel_rect.contains(local)
            });

            if !can_edit {
                // 无编辑目标：选框本身仍可拖动/缩放（仅移动选框，不涉及音符预览与音符 hit-test）
                let edge_hit = hit_test_sel_edge(eff_rects, view, local);
                if let Some((side, origin, other)) = edge_hit {
                    state.sel_resize_state = Some((side, origin, other));
                    sel_rect.start_resize(side);
                    state.drag_notes = Some(Vec::new());
                } else if in_sel_rect {
                    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
                    let raw_tick = main_px_to_tick_dir(view, main_px);
                    let tick =
                        crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                    let key = view.cross_px_to_key(cross_px) as f64;
                    let alt = ui.input(|i| i.modifiers.alt);
                    state.note_drag_origin = Some((tick, key, alt));
                    sel_rect.start_drag();
                    state.drag_notes = Some(Vec::new());
                    state.preview_last_dk = 0;
                    state.note_drag_had_moved = false;
                    ui.data_mut(|d| {
                        d.insert_persisted(
                            ui.id().with("note_drag_preview_dk"),
                            state.preview_last_dk,
                        )
                    });
                } else if !additive {
                    selected.clear();
                    sel_rect.clear();
                }
            } else {
                // ── 音符 hit-test（不用先选中，与铅笔一致）──
                // 轨道作用域 = track_selected（空 = 全部）∩ track_visible。
                // 边缘 → 单音符伸缩；中部（未选中）→ 单音符移动。
                // Bug 修复：存在选框时禁用单音符移动/缩放，选框整体操作优先。
                let has_selection = !sel_rect.is_empty();
                if !has_selection
                    && let Some((mode, track, start_tick, end_tick, key)) =
                        hit_test_note(midi, view, local, track_visible, track_selected)
                {
                    match mode {
                        crate::piano_view::pencil::HitMode::ResizeLeft
                        | crate::piano_view::pencil::HitMode::ResizeRight => {
                            let side = match mode {
                                crate::piano_view::pencil::HitMode::ResizeLeft => ResizeSide::Left,
                                _ => ResizeSide::Right,
                            };
                            state.sel_note_resize = Some((side, track, start_tick, end_tick, key));
                        }
                        crate::piano_view::pencil::HitMode::Move => {
                            // 音符中部：未选中时直接移动该音符；已选中交给选区移动。
                            if !in_sel_rect {
                                let (main_px, _) = main_cross_x_y(view, (local.x, local.y));
                                let raw_tick = main_px_to_tick_dir(view, main_px);
                                let tick = crate::view_interaction::snap_tick(
                                    raw_tick,
                                    quantize,
                                    ppq,
                                    bar_line_data,
                                );
                                // press 时锁定 alt（复制模式），拖拽中切换不影响本次操作。
                                let alt = ui.input(|i| i.modifiers.alt);
                                state.sel_note_move =
                                    Some((track, start_tick, key, end_tick, tick, 0, alt));
                                state.single_note_had_moved = false;
                            }
                        }
                    }
                    // 点击音符出声（gate 长度，原力度）。vel <= 1 隐藏音符不响。
                    if let Some(vel) =
                        crate::piano_view::pencil::note_velocity(midi, track, start_tick, key)
                        && vel > 1
                    {
                        state.preview_reqs.push(crate::piano_view::PreviewReq::Note(
                            crate::piano_view::NotePreview {
                                track,
                                key,
                                velocity: Some(vel),
                                target_tick: start_tick,
                                duration_ticks: end_tick - start_tick,
                            },
                        ));
                    }
                }

                // ── 选框边缘 hit-test：优先级大于拖动移动 ──
                // 已命中音符（伸缩/移动）时跳过——单音符操作优先于选框整体操作。
                let edge_hit = if state.sel_note_resize.is_some() || state.sel_note_move.is_some() {
                    None
                } else {
                    hit_test_sel_edge(eff_rects, view, local)
                };

                if let Some((side, origin_boundary_tick, other_boundary_tick)) = edge_hit {
                    // 启动 resize：记录原边缘 tick + 另一边缘 + 预计算选中音符
                    state.sel_resize_state =
                        Some((side, origin_boundary_tick, other_boundary_tick));
                    sel_rect.start_resize(side);
                    state.drag_notes = Some(collect_selected_notes(
                        selected,
                        midi,
                        track_visible,
                        track_selected,
                    ));
                } else if state.sel_note_resize.is_none() && state.sel_note_move.is_none() {
                    if in_sel_rect {
                        let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
                        let raw_tick = main_px_to_tick_dir(view, main_px);
                        let tick = crate::view_interaction::snap_tick(
                            raw_tick,
                            quantize,
                            ppq,
                            bar_line_data,
                        );
                        let key = view.cross_px_to_key(cross_px) as f64;
                        // Alt（Option）按下时进入复制模式：原音符保留，拖出副本。
                        // press 时锁定 alt 状态，拖拽中切换不影响本次操作。
                        let alt = ui.input(|i| i.modifiers.alt);
                        state.note_drag_origin = Some((tick, key, alt));
                        sel_rect.start_drag();
                        state.drag_notes = Some(collect_selected_notes(
                            selected,
                            midi,
                            track_visible,
                            track_selected,
                        ));
                        state.preview_last_dk = 0;
                        state.note_drag_had_moved = false;
                        ui.data_mut(|d| {
                            d.insert_persisted(
                                ui.id().with("note_drag_preview_dk"),
                                state.preview_last_dk,
                            )
                        });
                        // 点击选中音符出声：立即预览整组（dk=0，与移动时同组预览一致）。
                        // vel <= 1 的音符（黑乐谱隐藏音符）不响，与播放筛除一致。
                        if let Some(notes) = state.drag_notes.as_ref() {
                            state.preview_reqs = notes
                                .iter()
                                .filter(|info| info.velocity > 1)
                                .map(|info| {
                                    crate::piano_view::PreviewReq::Note(
                                        crate::piano_view::NotePreview {
                                            track: info.track,
                                            key: info.key,
                                            velocity: Some(info.velocity),
                                            target_tick: info.start_tick,
                                            duration_ticks: info.end_tick - info.start_tick,
                                        },
                                    )
                                })
                                .collect();
                        }
                    } else if !additive {
                        // 单击选框外（非加选模式）→ 立即清空选框与选区。
                        // 比 on_press 回调更早触发，覆盖 click（< 3px）的场景。
                        selected.clear();
                        sel_rect.clear();
                    }
                }
            }
        }
    }
}
