//! Selection-tool drag logic: move + edge-resize.
//!
//! 选框工具的 press → drag → release 状态机。marquee 框选逻辑在 `marquee.rs`。

use eframe::egui;

use yinhe_types::TimeSigEvent;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_editor_core::ResizeSide;

use crate::selection::drag::CollectedNote;
use super::marquee::marquee_drag_frame;

/// Pre-computed info for each selected note during a selection drag.
/// Built once at drag start, reused every frame — eliminates O(N×M) midi lookups.
pub(crate) type SelDragNoteInfo = CollectedNote;

/// 指针是否在选框浮动工具条（selection_actions bar）上。
fn on_action_bar(
    pos: egui::Pos2,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
) -> bool {
    eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
        let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
            &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
        );
        crate::widgets::selection_actions::compute_bar_rect(music_rect, pixel_rect)
            .is_some_and(|bar| bar.contains(pos))
    })
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
    note_drag_delta: &mut Option<(i64, i32, bool)>,
    note_resize_delta: &mut Option<(ResizeSide, i64)>,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    _track_colors: &[[f32; 3]],
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    vertical: bool,
) -> (Vec<(u32, u32, u8, u16)>, Vec<(u16, u32, u8)>) {
    let note_drag_id = ui.id().with("note_drag_origin");
    let mut note_drag_origin: Option<(f64, f64, bool)> =
        ui.data_mut(|d| d.get_persisted(note_drag_id)).unwrap_or(None);

    // Pre-computed drag note info — built once at drag start, reused every frame.
    let drag_notes_id = ui.id().with("drag_notes");
    let mut drag_notes: Option<Vec<SelDragNoteInfo>> =
        ui.data_mut(|d| d.get_persisted(drag_notes_id)).unwrap_or(None);

    // Resize state: (side, origin_boundary_tick, other_boundary_tick)。
    // origin_boundary_tick 是被拖动边缘的原 tick；other_boundary_tick 是另一个边缘。
    let resize_id = ui.id().with("sel_resize_state");
    let mut sel_resize_state: Option<(ResizeSide, f64, f64)> =
        ui.data_mut(|d| d.get_persisted(resize_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);

    // Clear stale note drag state
    if note_drag_origin.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        note_drag_origin = None;
        drag_notes = None;
        sel_rect.cancel_drag();
    }
    // Clear stale resize state
    if sel_resize_state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        sel_resize_state = None;
        drag_notes = None;
        sel_rect.cancel_resize();
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (Vec::new(), Vec::new());
    }

    // press 分支和 click 分支共用，整个函数作用域内有效。
    let eff_rects = sel_rect.effective_rects();

    // Start drag (note drag only — marquee is handled by shared function below)
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let on_bar = on_action_bar(pos, music_rect, view, &eff_rects);

        if on_bar {
            // Don't start drag, don't clear anything — let the button handle it.
        } else {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);

            // ── 边缘 hit-test：优先级大于拖动移动 ──
            // 检查鼠标是否在某个选框的左右边缘 EDGE_THRESHOLD_PX 内。
            let edge_hit = hit_test_sel_edge(&eff_rects, &view.base, view.key_height, local);

            if let Some((side, origin_boundary_tick, other_boundary_tick)) = edge_hit {
                // 启动 resize：记录原边缘 tick + 另一边缘 + 预计算选中音符
                sel_resize_state = Some((side, origin_boundary_tick, other_boundary_tick));
                sel_rect.start_resize(side);
                drag_notes = Some(collect_selected_notes(selected, midi, track_visible, track_selected));
            } else {
                let in_sel_rect = eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
                    let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                        &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
                    );
                    pixel_rect.contains(local)
                });
                if in_sel_rect {
                    let raw_tick = view.x_to_tick(local.x);
                    let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                    let key = view.y_to_key(local.y) as f64;
                    // Alt（Option）按下时进入复制模式：原音符保留，拖出副本。
                    // press 时锁定 alt 状态，拖拽中切换不影响本次操作。
                    let alt = ui.input(|i| i.modifiers.alt);
                    note_drag_origin = Some((tick, key, alt));
                    sel_rect.start_drag();
                    drag_notes = Some(collect_selected_notes(selected, midi, track_visible, track_selected));
                } else if !additive {
                    // 单击选框外（非加选模式）→ 立即清空选框与选区。
                    // 比 on_press 回调更早触发，覆盖 click（< 3px）的场景。
                    selected.clear();
                    sel_rect.clear();
                }
            }
        }
    }

    // Note drag: use pre-computed data for ghost/hidden, store delta only on release
    let mut ghost_notes: Vec<(u32, u32, u8, u16)> = Vec::new();
    let mut hidden_notes: Vec<(u16, u32, u8)> = Vec::new();
    if let Some((origin_tick, origin_key, alt)) = note_drag_origin {
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
                    // 垂直选框工具：只能水平移动，dk 强制为 0
                    let dk = if vertical { 0 } else { (current_key - origin_key).round() as i32 };

                    // O(N) — just apply delta to pre-computed data, no midi lookup.
                    // Alt（复制模式）：原音符保留可见，不 push hidden_notes。
                    for info in notes {
                        let new_tick = (info.start_tick as i64 + dt).max(0) as u32;
                        let new_key = ((info.key as i32) + dk).clamp(0, 127) as u8;
                        let length = info.end_tick - info.start_tick;
                        ghost_notes.push((new_tick, new_tick + length, new_key, info.track));
                        if !alt {
                            hidden_notes.push((info.track, info.start_tick, info.key));
                        }
                    }

                    sel_rect.update_drag(dt, dk);

                    // ── Tooltip：显示 ±tick / ±key（已按量化 snap）──
                    let lines = vec![
                        crate::view_interaction::format_signed("tick", dt),
                        crate::view_interaction::format_signed("key", dk as i64),
                    ];
                    crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
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
                    // 垂直选框工具：只能水平移动，dk 强制为 0
                    let dk = if vertical { 0 } else { (current_key - origin_key).round() as i32 };
                    *note_drag_delta = Some((dt, dk, alt));
                    sel_rect.update_drag(dt, dk);

                    // Keep ghost/hidden alive on the release frame so the original
                    // notes don't flash back before the model is updated.
                    for info in notes {
                        let new_tick = (info.start_tick as i64 + dt).max(0) as u32;
                        let new_key = ((info.key as i32) + dk).clamp(0, 127) as u8;
                        let length = info.end_tick - info.start_tick;
                        ghost_notes.push((new_tick, new_tick + length, new_key, info.track));
                        if !alt {
                            hidden_notes.push((info.track, info.start_tick, info.key));
                        }
                    }
                }
                sel_rect.end_drag();
                note_drag_origin = None;
                drag_notes = None;
            }
        }
    }

    // ── Resize drag: 边缘拖动伸缩选中音符 ──
    if let Some((side, origin_boundary_tick, other_boundary_tick)) = sel_resize_state {
        if let Some(ref notes) = drag_notes {
            // Drag：实时显示 ghost + 更新 sel_rect
            if pointer.primary_down() && !pointer.primary_pressed() {
                if let Some(pos) = pointer.hover_pos() {
                    // auto-scroll：边缘拖动能推出屏幕
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

                    let clamped = pos.clamp(music_rect.min, music_rect.max);
                    let local_x = clamped.x - content_rect.min.x;
                    let raw_tick = view.x_to_tick(local_x);
                    let (_new_boundary, dt) = compute_resize_dt(
                        raw_tick, side, origin_boundary_tick, other_boundary_tick,
                        quantize, ppq, bar_line_data,
                    );

                    // 生成 ghost/hidden：每个音符独立 clamp（end > start + 1）
                    for info in notes {
                        match side {
                            ResizeSide::Right => {
                                let new_end = (info.end_tick as i64 + dt)
                                    .max(info.start_tick as i64 + 1) as u32;
                                ghost_notes.push((info.start_tick, new_end, info.key, info.track));
                                hidden_notes.push((info.track, info.start_tick, info.key));
                            }
                            ResizeSide::Left => {
                                let new_start = (info.start_tick as i64 + dt)
                                    .max(0)
                                    .min(info.end_tick as i64 - 1) as u32;
                                ghost_notes.push((new_start, info.end_tick, info.key, info.track));
                                hidden_notes.push((info.track, info.start_tick, info.key));
                            }
                        }
                    }

                    sel_rect.update_resize(dt);

                    // ── Tooltip：显示 ±gate（gate 变化量：Left 时 start 偏移 dt，gate 变化 = -dt）──
                    let gate_delta = match side {
                        ResizeSide::Left => -dt,
                        ResizeSide::Right => dt,
                    };
                    let lines = vec![
                        crate::view_interaction::format_signed("gate", gate_delta),
                    ];
                    crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
                    ui.ctx().request_repaint();
                }
            }
            // Release：提交 dt
            if pointer.primary_released() {
                if let Some(pos) = pointer.hover_pos() {
                    let clamped = pos.clamp(music_rect.min, music_rect.max);
                    let local_x = clamped.x - content_rect.min.x;
                    let raw_tick = view.x_to_tick(local_x);
                    let (_new_boundary, dt) = compute_resize_dt(
                        raw_tick, side, origin_boundary_tick, other_boundary_tick,
                        quantize, ppq, bar_line_data,
                    );
                    *note_resize_delta = Some((side, dt));
                    sel_rect.update_resize(dt);

                    // Keep ghost/hidden alive on the release frame
                    for info in notes {
                        match side {
                            ResizeSide::Right => {
                                let new_end = (info.end_tick as i64 + dt)
                                    .max(info.start_tick as i64 + 1) as u32;
                                ghost_notes.push((info.start_tick, new_end, info.key, info.track));
                                hidden_notes.push((info.track, info.start_tick, info.key));
                            }
                            ResizeSide::Left => {
                                let new_start = (info.start_tick as i64 + dt)
                                    .max(0)
                                    .min(info.end_tick as i64 - 1) as u32;
                                ghost_notes.push((new_start, info.end_tick, info.key, info.track));
                                hidden_notes.push((info.track, info.start_tick, info.key));
                            }
                        }
                    }
                }
                sel_rect.end_resize();
                sel_resize_state = None;
                drag_notes = None;
            }
        }
    }

    // ── Marquee selection (shared with Eraser tool) ──
    // Only start a marquee if no note drag/resize is active (click was NOT inside selection).
    if note_drag_origin.is_some() || sel_resize_state.is_some() {
        // Note drag/resize active → clear any stale marquee state and skip marquee.
        let sel_id = ui.id().with("sel_drag");
        ui.data_mut(|d| d.insert_persisted(sel_id, Option::<((f64, f32), egui::Pos2, egui::Pos2)>::None));
    } else {
        if let Some(result) = marquee_drag_frame(
            ui, content_rect, music_rect, view, quantize, ppq, bar_line_data, total_ticks,
            "sel_drag",
        ) {
            let track_lo = track_selected.iter().min().copied().unwrap_or(0);
            let track_hi = track_selected.iter().max().copied().unwrap_or(u16::MAX);
            // 垂直全选模式：key 范围固定 0..127，忽略鼠标 y
            let (key_lo, key_hi) = if vertical { (0, 127) } else { (result.key_lo, result.key_hi) };
            selected.add_rect_track(
                result.t_start as u32, result.t_end as u32,
                key_lo, key_hi,
                track_lo, track_hi,
            );
            sel_rect.rects.push((result.t_start, result.t_end, key_lo, key_hi));
        } else if ui.input(|i| i.pointer.primary_released()) {
            // Simple click (no marquee) - set cursor to click position for paste.
            // 选框清空已在 press 时完成（非加选模式），此处仅设置 cursor。
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                if music_rect.contains(pos) && !on_action_bar(pos, music_rect, view, &eff_rects) {
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
    ui.data_mut(|d| d.insert_persisted(resize_id, sel_resize_state));
    (ghost_notes, hidden_notes)
}

// 通用逻辑已抽取到 crate::selection::drag：
// - hit_test_sel_edge（边缘 hit-test）
// - collect_selected_notes（选中音符预计算）
// - compute_resize_dt（量化对齐 + 最小宽度约束）
pub(crate) use crate::selection::drag::{collect_selected_notes, compute_resize_dt, hit_test_sel_edge};
