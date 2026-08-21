//! Selection-tool drag logic: move + edge-resize.
//!
//! 选框工具的 press → drag → release 状态机。marquee 框选逻辑在 `marquee.rs`。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::audio_settings::QuickDeleteMode;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::marquee::marquee_drag_frame;
use super::pencil::note_velocity;

mod hit;
mod interact;
mod state;
mod types;

pub(crate) use hit::{double_click_note, hit_test_note, rect_has_notes};
pub(crate) use interact::{
    clamped_local, cursor_tick_from_click, drag_scroll_and_clamp, on_action_bar,
};
pub(crate) use state::{SelDragFrameState, sel_drag_in_progress};
pub(crate) use types::*;

/// Press 帧分发：音符 hit-test → 单音符伸缩/移动；选框边缘 → 选区缩放；
/// 选框内 → 选区整体移动；选框外（非加选）→ 清空选框与选区。
/// marquee 的启动在共享的 `marquee_drag_frame`，不在此处。
#[allow(clippy::too_many_arguments)]
fn sel_press(
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
                        super::pencil::HitMode::ResizeLeft
                        | super::pencil::HitMode::ResizeRight => {
                            let side = match mode {
                                super::pencil::HitMode::ResizeLeft => ResizeSide::Left,
                                _ => ResizeSide::Right,
                            };
                            state.sel_note_resize = Some((side, track, start_tick, end_tick, key));
                        }
                        super::pencil::HitMode::Move => {
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
                    if let Some(vel) = note_velocity(midi, track, start_tick, key)
                        && vel > 1
                    {
                        state
                            .preview_reqs
                            .push(super::PreviewReq::Note(super::NotePreview {
                                track,
                                key,
                                velocity: Some(vel),
                                target_tick: start_tick,
                                duration_ticks: end_tick - start_tick,
                            }));
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
                                    super::PreviewReq::Note(super::NotePreview {
                                        track: info.track,
                                        key: info.key,
                                        velocity: Some(info.velocity),
                                        target_tick: info.start_tick,
                                        duration_ticks: info.end_tick - info.start_tick,
                                    })
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

/// 选区整体移动状态机（note_drag）：拖拽中更新 ghost/预览/选框，
/// release 提交 delta（普通移动与 Alt 复制共用 `note_drag_delta` 通道）。
#[allow(clippy::too_many_arguments)]
fn note_drag_frame(
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
                        super::PreviewReq::Note(super::NotePreview {
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
                    state.preview_reqs.push(super::PreviewReq::Stop);
                }
                sel_rect.end_drag();
            }
            state.note_drag_origin = None;
            state.drag_notes = None;
            state.note_drag_had_moved = false;
        }
    }
}

/// 选区边缘缩放状态机（sel_resize）：拖拽中更新 ghost/选框，
/// release 提交 `note_resize_delta`。
#[allow(clippy::too_many_arguments)]
fn sel_resize_frame(
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

            // 生成 ghost/hidden：每个音符独立 clamp（end > start + 1）
            for info in notes {
                match side {
                    ResizeSide::Right => {
                        let new_end =
                            (info.end_tick as i64 + dt).max(info.start_tick as i64 + 1) as u32;
                        state
                            .ghost_notes
                            .push((info.start_tick, new_end, info.key, info.track));
                        state
                            .hidden_notes
                            .push((info.track, info.start_tick, info.key));
                    }
                    ResizeSide::Left => {
                        let new_start = (info.start_tick as i64 + dt)
                            .max(0)
                            .min(info.end_tick as i64 - 1)
                            as u32;
                        state
                            .ghost_notes
                            .push((new_start, info.end_tick, info.key, info.track));
                        state
                            .hidden_notes
                            .push((info.track, info.start_tick, info.key));
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

                // Keep ghost/hidden alive on the release frame
                for info in notes {
                    match side {
                        ResizeSide::Right => {
                            let new_end =
                                (info.end_tick as i64 + dt).max(info.start_tick as i64 + 1) as u32;
                            state.ghost_notes.push((
                                info.start_tick,
                                new_end,
                                info.key,
                                info.track,
                            ));
                            state
                                .hidden_notes
                                .push((info.track, info.start_tick, info.key));
                        }
                        ResizeSide::Left => {
                            let new_start = (info.start_tick as i64 + dt)
                                .max(0)
                                .min(info.end_tick as i64 - 1)
                                as u32;
                            state.ghost_notes.push((
                                new_start,
                                info.end_tick,
                                info.key,
                                info.track,
                            ));
                            state
                                .hidden_notes
                                .push((info.track, info.start_tick, info.key));
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

/// 单音符边缘伸缩状态机（不用先选中，与铅笔一致）：
/// release 复用铅笔的 `PencilNoteDrag` 通道提交。
#[allow(clippy::too_many_arguments)]
fn single_note_resize_frame(
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
            state.preview_reqs.push(super::PreviewReq::Stop);
            state.sel_note_resize = None;
        }
    }
}

/// 单音符移动状态机（不用先选中，与铅笔一致）：
/// release 提交 `PencilNoteDrag::Move`，Alt 拖拽复制走 `note_drag_delta` 通道。
#[allow(clippy::too_many_arguments)]
fn single_note_move_frame(
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
                if let Some(vel) = note_velocity(midi, trk, orig_start, orig_key)
                    && vel > 1
                {
                    state
                        .preview_reqs
                        .push(super::PreviewReq::Note(super::NotePreview {
                            track: trk,
                            key: new_key,
                            velocity: Some(vel),
                            target_tick: new_start,
                            duration_ticks: orig_end - orig_start,
                        }));
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
            state.preview_reqs.push(super::PreviewReq::Stop);
            state.sel_note_move = None;
            state.single_note_had_moved = false;
        }
    }
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
    _track_colors: &[[f32; 4]],
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    write_track: Option<u16>,
    conductor_idx: Option<u16>,
    vertical: bool,
    quick_delete_mode: QuickDeleteMode,
) -> SelFrameOut {
    // 双击写音符的提交（note + track），由 show() 转成 PianoViewEvent::AddNote。
    let mut note_event: SelNoteEvent = None;
    // 单音符边缘伸缩的提交（复用铅笔的单音符伸缩通道）。
    let mut pencil_note_drag: Option<yinhe_types::PencilNoteDrag> = None;
    // 快速删除的提交（双击/右键删除音符）。
    let mut quick_delete: QuickDeleteEvent = None;

    // ── 帧内可变状态：从 egui 持久化加载（拖拽跨帧保持）──
    let mut state = SelDragFrameState::load(ui);

    let pointer = ui.input(|i| i.pointer.clone());
    // 加选修饰键：Windows 惯例 Ctrl+点击，macOS 惯例 Cmd+点击。
    // macOS 上 Ctrl+左键已被 raw_input_hook 改写为右键（系统惯例），不再承担加选。
    #[cfg(target_os = "macos")]
    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    #[cfg(not(target_os = "macos"))]
    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);

    // Clear stale note drag state
    if state.note_drag_origin.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        state.note_drag_origin = None;
        state.drag_notes = None;
        state.note_drag_had_moved = false;
        sel_rect.cancel_drag();
    }
    // Clear stale resize state
    if state.sel_resize_state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        state.sel_resize_state = None;
        state.drag_notes = None;
        sel_rect.cancel_resize();
    }
    // Clear stale single-note resize state
    if state.sel_note_resize.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        state.sel_note_resize = None;
    }
    // Clear stale single-note move state
    if state.sel_note_move.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        state.sel_note_move = None;
        state.single_note_had_moved = false;
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (Vec::new(), Vec::new(), Vec::new(), None, None, None);
    }

    // press 分支和 click 分支共用，整个函数作用域内有效。
    let eff_rects = sel_rect.effective_rects();
    // 按下时指针是否在选框浮动工具条上：在工具条上时不启动任何拖拽/框选
    // （曾复发两次：playhead 跳转 + 不按 ctrl 拉出第二个选框）。
    let press_on_bar = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pos| on_action_bar(pos, music_rect, view, &eff_rects));

    // 无编辑目标（未选中音轨 / 主音轨不可见 / 主音轨是 Conductor）时，
    // 禁止音符移动/缩放及一切 hit-test/预览（选框工具也不允许这些操作），
    // 但框选与点选的清空仍可用。提前计算供 sel_press 与后续状态机共用。
    let can_edit =
        super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx).is_some();

    // ── 右键快速删除（选择工具）──
    // 不依赖 can_edit：仅在已选轨道内删除（track_selected 空 = 全部可见轨道）
    if quick_delete.is_none()
        && quick_delete_mode.allows_right_click()
        && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !on_action_bar(pos, music_rect, view, &eff_rects)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((_, track, start_tick, _, key)) =
            hit_test_note(midi, view, local, track_visible, track_selected)
        {
            quick_delete = Some((track, start_tick, key));
        }
    }
    // ── 双击快速删除（选择工具）──
    // 提前于 sel_press 检测，避免第二击的 press 已设置 sel_note_move 导致判定被屏蔽
    if quick_delete.is_none()
        && quick_delete_mode.allows_double_click()
        && ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        })
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !on_action_bar(pos, music_rect, view, &eff_rects)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((_, track, start_tick, _, key)) =
            hit_test_note(midi, view, local, track_visible, track_selected)
        {
            quick_delete = Some((track, start_tick, key));
        }
    }

    // ── Press：音符/选框 hit-test 分发 ──
    sel_press(
        ui,
        &mut state,
        content_rect,
        music_rect,
        view,
        midi,
        selected,
        sel_rect,
        quantize,
        ppq,
        bar_line_data,
        &eff_rects,
        track_visible,
        track_selected,
        additive,
        press_on_bar,
        &pointer,
        can_edit,
    );

    // 选框整体移动/缩放即使无编辑目标也允许（仅移动选框本身，不涉及音符）
    note_drag_frame(
        ui,
        &mut state,
        view,
        content_rect,
        music_rect,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        vertical,
        sel_rect,
        note_drag_delta,
        &pointer,
    );
    sel_resize_frame(
        ui,
        &mut state,
        view,
        content_rect,
        music_rect,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        sel_rect,
        note_resize_delta,
        &pointer,
    );
    if can_edit {
        // 单音符操作仅在有编辑目标时允许
        single_note_resize_frame(
            ui,
            &mut state,
            view,
            content_rect,
            music_rect,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            &mut pencil_note_drag,
            &pointer,
        );
        single_note_move_frame(
            ui,
            &mut state,
            view,
            content_rect,
            music_rect,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            midi,
            selected,
            vertical,
            note_drag_delta,
            &mut pencil_note_drag,
            &pointer,
        );
    }

    // ── 双击写音符（第二击 release 帧触发）──
    // 已在 early 阶段处理过快速删除，此处仅处理“创建”分支（命中已有音符时 double_click_note 返回 None）
    if quick_delete.is_none()
        && ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        })
        && state.note_drag_origin.is_none()
        && state.sel_resize_state.is_none()
        && state.sel_note_resize.is_none()
        && state.sel_note_move.is_none()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !on_action_bar(pos, music_rect, view, &eff_rects)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((note, track)) = double_click_note(
            midi,
            write_track,
            track_visible,
            conductor_idx,
            view,
            local,
            quantize,
            ppq,
            bar_line_data,
        ) {
            note_event = Some((note, track));
            // 听觉预览：一次性播放（gate = 新建音符长度）。
            state
                .preview_reqs
                .push(super::PreviewReq::Note(super::NotePreview {
                    track,
                    key: note.key,
                    velocity: None,
                    target_tick: note.start_tick,
                    duration_ticks: note.end_tick - note.start_tick,
                }));
        }
    }

    // ── Marquee selection (shared with Eraser tool) ──
    // Only start a marquee if no note drag/resize is active (click was NOT inside selection).
    if state.note_drag_origin.is_some()
        || state.sel_resize_state.is_some()
        || state.sel_note_resize.is_some()
        || state.sel_note_move.is_some()
    {
        // Note drag/resize active → clear any stale marquee state and skip marquee.
        let sel_id = ui.id().with("sel_drag");
        ui.data_mut(|d| {
            d.insert_persisted(sel_id, Option::<((f64, f32), egui::Pos2, egui::Pos2)>::None)
        });
    } else {
        // release 帧 note_drag_origin / sel_resize_state / sel_note_resize 已被清 None，
        // 但本次 release 刚完成音符移动/缩放拖拽（delta 已写入）：不能再当简单点击处理，
        // 否则 cursor_tick 会跳到释放位置、演奏指示线错误跳转。
        let release_was_drag =
            note_drag_delta.is_some() || note_resize_delta.is_some() || pencil_note_drag.is_some();
        if let Some(result) = marquee_drag_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            "sel_drag",
            press_on_bar,
        ) {
            // 轨道作用域：track_selected（空 = 全部轨道）。
            let (track_lo, track_hi) = crate::selection::drag::pr_track_range(track_selected);
            // 垂直全选模式 key 固定 0..127；普通选框在框选区域无音符时
            // 也自动变成垂直选框（全 128 键）。
            // 自动切换的垂直选框打标记（拖动时锁定上下）；
            // 用户手动框选出的全键选框不打标记，仍可上下移动。
            let auto_vertical = !vertical
                && !rect_has_notes(
                    midi,
                    result.t_start as u32,
                    result.t_end as u32,
                    result.key_lo,
                    result.key_hi,
                    track_lo,
                    track_hi,
                );
            let (key_lo, key_hi) = if vertical || auto_vertical {
                (0, 127)
            } else {
                (result.key_lo, result.key_hi)
            };
            crate::selection::drag::add_pr_selection_rect(
                selected,
                result.t_start as u32,
                result.t_end as u32,
                key_lo,
                key_hi,
                track_selected,
            );
            sel_rect.push_rect(
                (result.t_start, result.t_end, key_lo, key_hi),
                auto_vertical,
            );
        } else if ui.input(|i| i.pointer.primary_released()) && !release_was_drag {
            // Simple click (no marquee) - set cursor to click position for paste.
            // 选框清空已在 press 时完成（非加选模式），此处仅设置 cursor。
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                // 仅当返回 Some 时才更新 cursor_tick：条件不满足时保持原值。
                if let Some(tick) = cursor_tick_from_click(
                    pos,
                    content_rect,
                    music_rect,
                    view,
                    &eff_rects,
                    quantize,
                    ppq,
                    bar_line_data,
                ) {
                    *cursor_tick = Some(tick);
                }
            }
        }
    }

    // ── 状态持久化（拖拽跨帧保持）──
    state.save(ui);
    (
        state.ghost_notes,
        state.hidden_notes,
        state.preview_reqs,
        note_event,
        pencil_note_drag,
        quick_delete,
    )
}

// 通用逻辑已抽取到 crate::selection::drag：
// - hit_test_sel_edge（边缘 hit-test）
// - collect_selected_notes（选中音符预计算）
// - compute_resize_dt（量化对齐 + 最小宽度约束）
pub(crate) use crate::selection::drag::{
    collect_selected_notes, compute_resize_dt, hit_test_sel_edge, main_cross_x_y,
    main_px_to_tick_dir, orient_rect, tick_to_main_px_dir,
};
#[cfg(test)]
#[path = "drag_tests.rs"]
mod tests;
