//! Selection-tool drag logic: move + edge-resize.
//!
//! 选框工具的 press → drag → release 状态机。marquee 框选逻辑在 `marquee.rs`。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::marquee::marquee_drag_frame;
use super::pencil::note_velocity;
use crate::selection::drag::CollectedNote;

/// Pre-computed info for each selected note during a selection drag.
/// Built once at drag start, reused every frame — eliminates O(N×M) midi lookups.
pub(crate) type SelDragNoteInfo = CollectedNote;

/// 拖拽预览的幽灵音符：(start_tick, end_tick, key, track)。
pub(crate) type GhostNote = (u32, u32, u8, u16);
/// 拖拽时隐藏的原音符：(track, start_tick, key)。
pub(crate) type HiddenNote = (u16, u32, u8);

/// 双击写音符的提交：(note, track)。
pub(crate) type SelNoteEvent = Option<(yinhe_core::NoteEvent, u16)>;

/// 选择工具单音符边缘伸缩：(side, track, start_tick, end_tick, key)。
/// 与选框整体伸缩（sel_resize_state）互斥，音符边缘优先。
pub(crate) type SelNoteResize = (ResizeSide, u16, u32, u32, u8);

/// sel_drag_frame 的帧输出。
pub(crate) type SelFrameOut = (
    Vec<GhostNote>,
    Vec<HiddenNote>,
    Vec<super::PreviewReq>,
    SelNoteEvent,
    Option<yinhe_types::PencilNoteDrag>,
);

/// 指针是否在选框浮动工具条（selection_actions bar）上。
fn on_action_bar(
    pos: egui::Pos2,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
) -> bool {
    eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
        let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
            &view.base,
            view.key_height,
            t_start,
            t_end,
            key_lo,
            key_hi,
        );
        crate::widgets::selection_actions::compute_bar_rect(music_rect, pixel_rect)
            .is_some_and(|bar| bar.contains(pos))
    })
}

/// 简单点击（无 marquee）时的播放指示器定位。
///
/// 点在浮动工具条（selection_actions bar）上或 music_rect 外时返回 `None`——
/// 这是防穿透的关键：点击工具条按钮不能让 playhead 跳转（曾复发两次）。
#[allow(clippy::too_many_arguments)]
fn cursor_tick_from_click(
    pos: egui::Pos2,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    eff_rects: &[(f64, f64, u8, u8)],
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> Option<f64> {
    if !music_rect.contains(pos) || on_action_bar(pos, music_rect, view, eff_rects) {
        return None;
    }
    let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
    let tick = view.x_to_tick(local.x);
    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
    Some(snapped.max(0.0))
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
    editing_track: Option<u16>,
    conductor_idx: Option<u16>,
    vertical: bool,
) -> SelFrameOut {
    // 双击写音符的提交（note + track），由 show() 转成 PianoViewEvent::AddNote。
    let mut note_event: SelNoteEvent = None;
    // 单音符边缘伸缩的提交（复用铅笔的单音符伸缩通道）。
    let mut pencil_note_drag: Option<yinhe_types::PencilNoteDrag> = None;
    let note_drag_id = ui.id().with("note_drag_origin");
    let mut note_drag_origin: Option<(f64, f64, bool)> = ui
        .data_mut(|d| d.get_persisted(note_drag_id))
        .unwrap_or(None);

    // 拖拽中已触发预览的 key delta（每变化 1 key 触发一次整组预览）。
    let preview_dk_id = ui.id().with("note_drag_preview_dk");
    let mut preview_last_dk: i32 = ui.data_mut(|d| d.get_persisted(preview_dk_id)).unwrap_or(0);
    // 音符听觉预览请求（key 变化时整组触发）。
    let mut preview_reqs: Vec<super::PreviewReq> = Vec::new();

    // Pre-computed drag note info — built once at drag start, reused every frame.
    let drag_notes_id = ui.id().with("drag_notes");
    let mut drag_notes: Option<Vec<SelDragNoteInfo>> = ui
        .data_mut(|d| d.get_persisted(drag_notes_id))
        .unwrap_or(None);

    // Resize state: (side, origin_boundary_tick, other_boundary_tick)。
    // origin_boundary_tick 是被拖动边缘的原 tick；other_boundary_tick 是另一个边缘。
    let resize_id = ui.id().with("sel_resize_state");
    let mut sel_resize_state: Option<(ResizeSide, f64, f64)> =
        ui.data_mut(|d| d.get_persisted(resize_id)).unwrap_or(None);

    // 单音符边缘伸缩状态（不需要先选中，见 hit_test_note）。
    let note_resize_id = ui.id().with("sel_note_resize_state");
    let mut sel_note_resize: Option<SelNoteResize> = ui
        .data_mut(|d| d.get_persisted(note_resize_id))
        .unwrap_or(None);

    // 单音符移动状态：(track, orig_start, orig_key, orig_end, press_snapped_tick, last_dk)。
    // 不需要先选中：press 音符中部（未选中）直接移动该音符，与铅笔一致。
    let note_move_id = ui.id().with("sel_note_move_state");
    let mut sel_note_move: Option<(u16, u32, u8, u32, f64, i32)> = ui
        .data_mut(|d| d.get_persisted(note_move_id))
        .unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    // 加选修饰键：Windows 惯例 Ctrl+点击，macOS 惯例 Cmd+点击。
    // macOS 上 Ctrl+左键已被 raw_input_hook 改写为右键（系统惯例），不再承担加选。
    #[cfg(target_os = "macos")]
    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    #[cfg(not(target_os = "macos"))]
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
    // Clear stale single-note resize state
    if sel_note_resize.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        sel_note_resize = None;
    }
    // Clear stale single-note move state
    if sel_note_move.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        sel_note_move = None;
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (Vec::new(), Vec::new(), Vec::new(), None, None);
    }

    // press 分支和 click 分支共用，整个函数作用域内有效。
    let eff_rects = sel_rect.effective_rects();
    // 按下时指针是否在选框浮动工具条上：在工具条上时不启动任何拖拽/框选
    // （曾复发两次：playhead 跳转 + 不按 ctrl 拉出第二个选框）。
    let press_on_bar = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pos| on_action_bar(pos, music_rect, view, &eff_rects));

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
                    &view.base,
                    view.key_height,
                    t_start,
                    t_end,
                    key_lo,
                    key_hi,
                );
                pixel_rect.contains(local)
            });

            // ── 音符 hit-test（不用先选中，与铅笔一致）──
            // 轨道作用域 = editing_track（存在时）∪ track_selected ∩ track_visible。
            // 边缘 → 单音符伸缩；中部（未选中）→ 单音符移动。
            if let Some((mode, track, start_tick, end_tick, key)) = hit_test_note(
                midi,
                view,
                local,
                track_visible,
                track_selected,
                editing_track,
            ) {
                match mode {
                    super::pencil::HitMode::ResizeLeft | super::pencil::HitMode::ResizeRight => {
                        let side = match mode {
                            super::pencil::HitMode::ResizeLeft => ResizeSide::Left,
                            _ => ResizeSide::Right,
                        };
                        sel_note_resize = Some((side, track, start_tick, end_tick, key));
                    }
                    super::pencil::HitMode::Move => {
                        // 音符中部：未选中时直接移动该音符；已选中交给选区移动。
                        if !in_sel_rect {
                            let raw_tick = view.x_to_tick(local.x);
                            let tick = crate::view_interaction::snap_tick(
                                raw_tick,
                                quantize,
                                ppq,
                                bar_line_data,
                            );
                            sel_note_move = Some((track, start_tick, key, end_tick, tick, 0));
                        }
                    }
                }
                // 点击音符出声（gate 长度，原力度）。vel <= 1 隐藏音符不响。
                if let Some(vel) = note_velocity(midi, track, start_tick, key)
                    && vel > 1
                {
                    preview_reqs.push(super::PreviewReq::Note(super::NotePreview {
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
            let edge_hit = if sel_note_resize.is_some() || sel_note_move.is_some() {
                None
            } else {
                hit_test_sel_edge(&eff_rects, &view.base, view.key_height, local)
            };

            if let Some((side, origin_boundary_tick, other_boundary_tick)) = edge_hit {
                // 启动 resize：记录原边缘 tick + 另一边缘 + 预计算选中音符
                sel_resize_state = Some((side, origin_boundary_tick, other_boundary_tick));
                sel_rect.start_resize(side);
                drag_notes = Some(collect_selected_notes(
                    selected,
                    midi,
                    track_visible,
                    track_selected,
                    editing_track,
                ));
            } else if sel_note_resize.is_none() && sel_note_move.is_none() {
                if in_sel_rect {
                    let raw_tick = view.x_to_tick(local.x);
                    let tick =
                        crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                    let key = view.y_to_key(local.y) as f64;
                    // Alt（Option）按下时进入复制模式：原音符保留，拖出副本。
                    // press 时锁定 alt 状态，拖拽中切换不影响本次操作。
                    let alt = ui.input(|i| i.modifiers.alt);
                    note_drag_origin = Some((tick, key, alt));
                    sel_rect.start_drag();
                    drag_notes = Some(collect_selected_notes(
                        selected,
                        midi,
                        track_visible,
                        track_selected,
                        editing_track,
                    ));
                    preview_last_dk = 0;
                    ui.data_mut(|d| d.insert_persisted(preview_dk_id, 0));
                    // 点击选中音符出声：立即预览整组（dk=0，与移动时同组预览一致）。
                    // vel <= 1 的音符（黑乐谱隐藏音符）不响，与播放筛除一致。
                    if let Some(notes) = drag_notes.as_ref() {
                        preview_reqs = notes
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

    // Note drag: use pre-computed data for ghost/hidden, store delta only on release
    let mut ghost_notes: Vec<GhostNote> = Vec::new();
    let mut hidden_notes: Vec<HiddenNote> = Vec::new();
    if let Some((origin_tick, origin_key, alt)) = note_drag_origin
        && let Some(ref notes) = drag_notes
    {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
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
            let snapped_tick =
                crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
            let current_key = view.y_to_key(local_y) as f64;
            let dt = (snapped_tick - origin_tick).round() as i64;
            // 垂直选框工具：只能水平移动，dk 强制为 0
            let dk = if vertical {
                0
            } else {
                (current_key - origin_key).round() as i32
            };

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

            // 音符听觉预览：每变化 1 key，播放一次整组选中音符（各自通道/力度，
            // 长度 = 音符 gate，时长换算用目标位置 Tempo）。
            if dk != preview_last_dk {
                preview_last_dk = dk;
                // vel <= 1 的音符（黑乐谱隐藏音符）不预览，与播放筛除一致。
                preview_reqs = notes
                    .iter()
                    .filter(|info| info.velocity > 1)
                    .map(|info| {
                        super::PreviewReq::Note(super::NotePreview {
                            track: info.track,
                            key: ((info.key as i32) + dk).clamp(0, 127) as u8,
                            velocity: Some(info.velocity),
                            target_tick: (info.start_tick as i64 + dt).max(0) as u32,
                            duration_ticks: info.end_tick - info.start_tick,
                        })
                    })
                    .collect();
                ui.data_mut(|d| d.insert_persisted(preview_dk_id, preview_last_dk));
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
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(music_rect.min, music_rect.max);
                let local_x = clamped.x - content_rect.min.x;
                let local_y = clamped.y - content_rect.min.y;
                let raw_tick = view.x_to_tick(local_x);
                let snapped_tick =
                    crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let current_key = view.y_to_key(local_y) as f64;
                let dt = (snapped_tick - origin_tick).round() as i64;
                // 垂直选框工具：只能水平移动，dk 强制为 0
                let dk = if vertical {
                    0
                } else {
                    (current_key - origin_key).round() as i32
                };
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
            preview_reqs.push(super::PreviewReq::Stop);
            sel_rect.end_drag();
            note_drag_origin = None;
            drag_notes = None;
        }
    }

    // ── Resize drag: 边缘拖动伸缩选中音符 ──
    if let Some((side, origin_boundary_tick, other_boundary_tick)) = sel_resize_state
        && let Some(ref notes) = drag_notes
    {
        // Drag：实时显示 ghost + 更新 sel_rect
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
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
                        ghost_notes.push((info.start_tick, new_end, info.key, info.track));
                        hidden_notes.push((info.track, info.start_tick, info.key));
                    }
                    ResizeSide::Left => {
                        let new_start = (info.start_tick as i64 + dt)
                            .max(0)
                            .min(info.end_tick as i64 - 1)
                            as u32;
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
            let lines = vec![crate::view_interaction::format_signed("gate", gate_delta)];
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
            ui.ctx().request_repaint();
        }
        // Release：提交 dt
        if pointer.primary_released() {
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(music_rect.min, music_rect.max);
                let local_x = clamped.x - content_rect.min.x;
                let raw_tick = view.x_to_tick(local_x);
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
                            ghost_notes.push((info.start_tick, new_end, info.key, info.track));
                            hidden_notes.push((info.track, info.start_tick, info.key));
                        }
                        ResizeSide::Left => {
                            let new_start = (info.start_tick as i64 + dt)
                                .max(0)
                                .min(info.end_tick as i64 - 1)
                                as u32;
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

    // ── Single-note edge resize: 直接伸缩音符（不用先选中，与铅笔一致）──
    if let Some((side, trk, orig_start, orig_end, orig_key)) = sel_note_resize {
        let (boundary_tick, other_tick) = match side {
            ResizeSide::Right => (orig_end as f64, orig_start as f64),
            ResizeSide::Left => (orig_start as f64, orig_end as f64),
        };
        // Drag：实时显示 ghost + hidden
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
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
                    ghost_notes.push((orig_start, new_boundary as u32, orig_key, trk));
                }
                ResizeSide::Left => {
                    ghost_notes.push((new_boundary as u32, orig_end, orig_key, trk));
                }
            }
            hidden_notes.push((trk, orig_start, orig_key));

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
                let clamped = pos.clamp(music_rect.min, music_rect.max);
                let local_x = clamped.x - content_rect.min.x;
                let raw_tick = view.x_to_tick(local_x);
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
                        pencil_note_drag = Some(yinhe_types::PencilNoteDrag::ResizeRight {
                            track: trk,
                            start_tick: orig_start,
                            key: orig_key,
                            new_end_tick: new_boundary as u32,
                        });
                        // Keep ghost/hidden alive on the release frame
                        ghost_notes.push((orig_start, new_boundary as u32, orig_key, trk));
                    }
                    ResizeSide::Left => {
                        pencil_note_drag = Some(yinhe_types::PencilNoteDrag::ResizeLeft {
                            track: trk,
                            start_tick: orig_start,
                            key: orig_key,
                            new_start_tick: new_boundary as u32,
                        });
                        ghost_notes.push((new_boundary as u32, orig_end, orig_key, trk));
                    }
                }
                hidden_notes.push((trk, orig_start, orig_key));
            }
            preview_reqs.push(super::PreviewReq::Stop);
            sel_note_resize = None;
        }
    }

    // ── Single-note move: 直接拖动未选中音符（不用先选中，与铅笔一致）──
    if let Some((trk, orig_start, orig_key, orig_end, press_tick, last_dk)) = sel_note_move {
        // Drag：实时显示 ghost + hidden + tooltip
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            // auto-scroll：音符能拖出屏幕
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
            let local_y = clamped.y - content_rect.min.y;
            let raw_tick = view.x_to_tick(local_x);
            let snapped_tick =
                crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
            let dt = (snapped_tick - press_tick).round() as i64;
            // 垂直选框工具：只能水平移动，dk 强制为 0
            let dk = if vertical {
                0
            } else {
                view.y_to_key(local_y) as i32 - orig_key as i32
            };

            let new_start = (orig_start as i64 + dt).max(0) as u32;
            let new_key = (orig_key as i32 + dk).clamp(0, 127) as u8;
            ghost_notes.push((new_start, new_start + (orig_end - orig_start), new_key, trk));
            hidden_notes.push((trk, orig_start, orig_key));

            // 音符预览：每变化 1 key 触发一次（gate 长度，原力度）。
            // vel <= 1 的音符（黑乐谱隐藏音符）不预览，与播放筛除一致。
            if dk != last_dk {
                sel_note_move = Some((trk, orig_start, orig_key, orig_end, press_tick, dk));
                if let Some(vel) = note_velocity(midi, trk, orig_start, orig_key)
                    && vel > 1
                {
                    preview_reqs.push(super::PreviewReq::Note(super::NotePreview {
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
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(music_rect.min, music_rect.max);
                let local_x = clamped.x - content_rect.min.x;
                let local_y = clamped.y - content_rect.min.y;
                let raw_tick = view.x_to_tick(local_x);
                let snapped_tick =
                    crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let dt = (snapped_tick - press_tick).round() as i64;
                let dk = if vertical {
                    0
                } else {
                    view.y_to_key(local_y) as i32 - orig_key as i32
                };
                pencil_note_drag = Some(yinhe_types::PencilNoteDrag::Move {
                    track: trk,
                    start_tick: orig_start,
                    key: orig_key,
                    delta_ticks: dt,
                    delta_keys: dk,
                });
                // Keep ghost/hidden alive on the release frame
                let new_start = (orig_start as i64 + dt).max(0) as u32;
                let new_key = (orig_key as i32 + dk).clamp(0, 127) as u8;
                ghost_notes.push((new_start, new_start + (orig_end - orig_start), new_key, trk));
                hidden_notes.push((trk, orig_start, orig_key));
            }
            preview_reqs.push(super::PreviewReq::Stop);
            sel_note_move = None;
        }
    }

    // ── 双击写音符（第二击 release 帧触发）──
    // egui 在第二击 release 时判定 double-click。条件：
    // - 无 note drag / resize 进行中（排除双击选框内音符/边缘的情况）
    // - 不在浮动工具条上（防事件穿透）
    // - editing_track 有效且点击位置无音符 → 创建，长度 = 一个量化间隔。
    // 双击命中音符时 double_click_note 返回 None，保持选中/拖拽行为。
    if ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    }) && note_drag_origin.is_none()
        && sel_resize_state.is_none()
        && sel_note_resize.is_none()
        && sel_note_move.is_none()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !on_action_bar(pos, music_rect, view, &eff_rects)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((note, track)) = double_click_note(
            midi,
            editing_track,
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
            preview_reqs.push(super::PreviewReq::Note(super::NotePreview {
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
    if note_drag_origin.is_some()
        || sel_resize_state.is_some()
        || sel_note_resize.is_some()
        || sel_note_move.is_some()
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
            // 轨道作用域：editing_track 存在时框选只作用于编辑音轨；
            // 否则 track_selected（空 = 全部轨道）。
            let (track_lo, track_hi) =
                crate::selection::drag::pr_track_range(editing_track, track_selected);
            // 垂直全选模式 key 固定 0..127；普通选框在框选区域无音符时
            // 也自动变成垂直选框（全 128 键）。
            let (key_lo, key_hi) = if vertical
                || !rect_has_notes(
                    midi,
                    result.t_start as u32,
                    result.t_end as u32,
                    result.key_lo,
                    result.key_hi,
                    track_lo,
                    track_hi,
                ) {
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
                editing_track,
                track_selected,
            );
            sel_rect
                .rects
                .push((result.t_start, result.t_end, key_lo, key_hi));
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

    ui.data_mut(|d| d.insert_persisted(note_drag_id, note_drag_origin));
    ui.data_mut(|d| d.insert_persisted(drag_notes_id, drag_notes));
    ui.data_mut(|d| d.insert_persisted(resize_id, sel_resize_state));
    ui.data_mut(|d| d.insert_persisted(note_resize_id, sel_note_resize));
    ui.data_mut(|d| d.insert_persisted(note_move_id, sel_note_move));
    (
        ghost_notes,
        hidden_notes,
        preview_reqs,
        note_event,
        pencil_note_drag,
    )
}

// 通用逻辑已抽取到 crate::selection::drag：
// - hit_test_sel_edge（边缘 hit-test）
// - collect_selected_notes（选中音符预计算）
// - compute_resize_dt（量化对齐 + 最小宽度约束）
pub(crate) use crate::selection::drag::{
    collect_selected_notes, compute_resize_dt, hit_test_sel_edge,
};

/// 双击写音符：editing_track 有效且点击位置无音符时创建新音符。
///
/// 音符长度 = 一个量化间隔（与铅笔点击一致）。返回 `(note, track)`。
/// 命中已有音符（editing_track 上）时返回 `None`——双击保持选中/拖拽行为。
#[allow(clippy::too_many_arguments)]
fn double_click_note(
    midi: Option<&dyn yinhe_types::NoteSource>,
    editing_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> Option<(yinhe_core::NoteEvent, u16)> {
    let track = super::pencil::valid_pencil_track(editing_track, track_visible, conductor_idx)?;
    let raw_tick = view.x_to_tick(local.x);
    let key = view.y_to_key(local.y);
    // 点击位置已有音符（editing_track 上）→ 不创建。
    // key_notes_in_range 左边界保守（tick - max_note_len），右边界精确，
    // 任何覆盖该像素点的音符都会被包含；像素判定过滤跨边界长音符。
    if let Some(midi) = midi {
        let hit = midi
            .key_notes_in_range(key, raw_tick as u32, (raw_tick + 1.0) as u32)
            .any(|n| {
                n.track == track
                    && view.tick_to_x(n.start_tick as f64) <= local.x
                    && local.x <= view.tick_to_x(n.end_tick as f64)
            });
        if hit {
            return None;
        }
    }
    let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data).max(0.0);
    let interval = quantize.tick_interval(ppq) as f64;
    Some((
        yinhe_core::NoteEvent {
            id: 0, // 由 Document::add_note 分配
            start_tick: tick as u32,
            end_tick: (tick + interval) as u32,
            key,
            velocity: 100, // App 层替换为 default_velocity
        },
        track,
    ))
}

/// 音符 hit-test：返回 `(mode, track, start_tick, end_tick, key)`。
///
/// 不需要先选中：边缘 → 单音符伸缩；中部 → 单音符移动（与铅笔一致）。
/// 轨道作用域 = editing_track（存在时只查编辑音轨），否则 track_selected
/// （空 = 全部）∩ track_visible。
/// 只查可能覆盖鼠标点的音符：key_notes_in_range 左边界保守（tick - max_note_len），
/// 右边界精确，每帧 hover 开销与铅笔 hit-test 同级。
pub(crate) fn hit_test_note(
    midi: Option<&dyn yinhe_types::NoteSource>,
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    editing_track: Option<u16>,
) -> Option<(super::pencil::HitMode, u16, u32, u32, u8)> {
    const EDGE_THRESHOLD_PX: f32 = 6.0;
    let (midi, key) = (midi?, view.y_to_key(local.y));
    let raw_tick = view.x_to_tick(local.x);
    let notes = midi.key_notes_in_range(key, raw_tick as u32, (raw_tick + 1.0) as u32);
    for note in notes {
        // 轨道作用域：editing_track 优先，其次 track_selected（空 = 全部）∩ track_visible。
        let in_scope = match editing_track {
            Some(t) => note.track == t,
            None => {
                (track_selected.is_empty() || track_selected.contains(&note.track))
                    && track_visible
                        .get(note.track as usize)
                        .copied()
                        .unwrap_or(true)
            }
        };
        if !in_scope {
            continue;
        }
        let note_left = view.tick_to_x(note.start_tick as f64);
        let note_right = view.tick_to_x(note.end_tick as f64);
        if local.x < note_left || local.x > note_right {
            continue;
        }
        let dist_left = (local.x - note_left).abs();
        let dist_right = (local.x - note_right).abs();
        let mode = if dist_left <= EDGE_THRESHOLD_PX {
            super::pencil::HitMode::ResizeLeft
        } else if dist_right <= EDGE_THRESHOLD_PX {
            super::pencil::HitMode::ResizeRight
        } else {
            super::pencil::HitMode::Move // 音符中部：直接拖动移动该音符
        };
        return Some((mode, note.track, note.start_tick, note.end_tick, key));
    }
    None
}

/// 选框区域内是否至少有一个音符（数据层面，track 范围限定）。
///
/// 框选松手时判断：区域内无音符 → 自动变为垂直选框（全 128 键）。
fn rect_has_notes(
    midi: Option<&dyn yinhe_types::NoteSource>,
    t_start: u32,
    t_end: u32,
    key_lo: u8,
    key_hi: u8,
    track_lo: u16,
    track_hi: u16,
) -> bool {
    let Some(midi) = midi else { return false };
    (key_lo..=key_hi).any(|key| {
        midi.key_notes_in_range(key, t_start, t_end)
            .any(|n| n.track >= track_lo && n.track <= track_hi && n.start_tick >= t_start)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_editor_core::quantize::QuantizePreset;
    use yinhe_test_helpers::make_midi;

    /// 构造测试用的钢琴卷帘视图：1px/tick、无滚动、key 高 10px。
    fn test_view() -> yinhe_types::PianoRollView {
        yinhe_types::PianoRollView {
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 1.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: false,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            key_height: 10.0,
            viewport_h: 0.0,
        }
    }

    fn content() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    /// 取浮动工具条上的一点（用 compute_bar_rect 计算得到，避免硬编码坐标）。
    fn bar_point(view: &yinhe_types::PianoRollView) -> egui::Pos2 {
        let eff = [(0.0f64, 100.0f64, 60u8, 70u8)];
        let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
            &view.base,
            view.key_height,
            eff[0].0,
            eff[0].1,
            eff[0].2,
            eff[0].3,
        );
        let bar = crate::widgets::selection_actions::compute_bar_rect(content(), pixel_rect)
            .expect("bar 应显示");
        bar.center()
    }

    #[test]
    fn click_on_action_bar_does_not_move_playhead() {
        // 回归测试：点击浮动工具条（曾两次导致 playhead 意外跳转）
        let view = test_view();
        let eff = [(0.0, 100.0, 60, 70)];
        let pos = bar_point(&view);
        assert!(
            on_action_bar(pos, content(), &view, &eff),
            "测试前提：该点应在工具条上"
        );
        let result = cursor_tick_from_click(
            pos,
            content(),
            content(),
            &view,
            &eff,
            QuantizePreset::Fraction(1, 4),
            480,
            None,
        );
        assert_eq!(result, None, "点在工具条上时不得移动播放指示器");
    }

    #[test]
    fn click_outside_bar_moves_playhead() {
        let view = test_view();
        let eff = [(0.0, 100.0, 60, 70)];
        // 选框左侧远处、仍在 music_rect 内的点
        let pos = egui::pos2(200.0, 300.0);
        assert!(!on_action_bar(pos, content(), &view, &eff));
        let result = cursor_tick_from_click(
            pos,
            content(),
            content(),
            &view,
            &eff,
            QuantizePreset::Fraction(1, 4),
            480,
            None,
        );
        assert!(result.is_some(), "工具条外的点击应正常定位");
    }

    #[test]
    fn click_outside_music_rect_returns_none() {
        let view = test_view();
        let eff = [(0.0, 100.0, 60, 70)];
        let pos = egui::pos2(100.0, 700.0); // 超出 music_rect 下边界
        let result = cursor_tick_from_click(
            pos,
            content(),
            content(),
            &view,
            &eff,
            QuantizePreset::Fraction(1, 4),
            480,
            None,
        );
        assert_eq!(result, None);
    }

    /// 跑一帧 sel_drag_frame（Select 工具）。
    /// 返回 (note_event, preview_reqs, pencil_drag)，供双击写音符/单音符伸缩测试断言。
    #[allow(clippy::too_many_arguments)]
    fn run_sel_frame(
        ctx: &egui::Context,
        raw: egui::RawInput,
        view: &mut yinhe_types::PianoRollView,
        midi: &dyn yinhe_types::NoteSource,
        selected: &mut yinhe_core::Selection,
        cursor_tick: &mut Option<f64>,
        note_drag_delta: &mut Option<(i64, i32, bool)>,
        note_resize_delta: &mut Option<(yinhe_editor_core::ResizeSide, i64)>,
        sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
        editing_track: Option<u16>,
    ) -> (
        Option<(yinhe_core::NoteEvent, u16)>,
        Vec<crate::piano_view::PreviewReq>,
        Option<yinhe_types::PencilNoteDrag>,
    ) {
        let mut out: (
            Option<(yinhe_core::NoteEvent, u16)>,
            Vec<crate::piano_view::PreviewReq>,
            Option<yinhe_types::PencilNoteDrag>,
        ) = (None, Vec::new(), None);
        let _ = ctx.run_ui(raw, |ui| {
            let (_, _, previews, note_event, pencil_drag) = sel_drag_frame(
                ui,
                content(),
                content(),
                view,
                Some(midi),
                selected,
                QuantizePreset::Fraction(1, 16),
                480,
                None,
                10000.0,
                cursor_tick,
                note_drag_delta,
                note_resize_delta,
                sel_rect,
                &[[0.5, 0.5, 0.5, 1.0]],
                &[true],
                &std::collections::HashSet::new(),
                editing_track,
                None,
                false,
            );
            out = (note_event, previews, pencil_drag);
        });
        out
    }

    fn press_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    fn drag_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw
    }

    fn release_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    /// 回归测试：移动音符后松开鼠标不得让演奏指示线跳到释放位置。
    /// （release 帧 note_drag_origin 已被清 None，曾导致 marquee 的 simple-click
    /// 路径把 cursor_tick 设为释放点，playhead 错误跳转。）
    #[test]
    fn release_after_note_move_does_not_move_playhead() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        // 模拟已初始化的视口（viewport_h==0 时 clamp_scroll 会触发首次初始化，
        // 重算 key_height/scroll_y，干扰本测试的坐标假设）。
        view.viewport_h = 600.0;
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        // 选框覆盖音符 (tick 0..480, key 100)。key 100 → y = (127-100)*10 = 270。
        let mut selected = yinhe_core::Selection::default();
        selected.add_rect_track(0, 480, 100, 100, 0, 0);
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        sel_rect.rects.push((0.0, 480.0, 100, 100));
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // 音符中间按下 → 拖到 tick 360（1/16 网格：间隔 120）→ 松开。
        let press = egui::pos2(240.0, 275.0);
        let release = egui::pos2(360.0, 275.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(press),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert_eq!(
            note_drag_delta,
            Some((120, 0, false)),
            "音符应移动 +120 tick"
        );
        assert_eq!(cursor_tick, None, "移动后松开不得把 playhead 跳到释放位置");
    }

    /// 双击空白处 → 创建音符（选择工具）。
    #[test]
    fn double_click_creates_note() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        let midi = make_midi(vec![]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // 双击位置：tick 360（1/16 网格 480×4/16=120 的网格点）、key 90 → y = (127-90)*10 + 5 = 375。
        let pos = egui::pos2(360.0, 375.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );
        let _ = run_sel_frame(
            &ctx,
            press_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );
        // egui 在第二击 release 帧判定 double-click。
        let (note_event, previews, _) = run_sel_frame(
            &ctx,
            release_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );

        let (note, track) = note_event.expect("双击空白应创建音符");
        assert_eq!(track, 0);
        assert_eq!(note.start_tick, 360, "起点按量化 snap");
        assert_eq!(note.end_tick, 480, "长度 = 一个量化间隔");
        assert_eq!(note.key, 90);
        assert!(
            matches!(
                previews.first(),
                Some(crate::piano_view::PreviewReq::Note(_))
            ),
            "双击创建应触发听觉预览"
        );
    }

    /// 双击已有音符的位置 → 不创建（保持选择工具行为）。
    #[test]
    fn double_click_on_existing_note_does_not_create() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // 音符 (tick 300..330, key 90)：中心点 (315, 375)。
        let pos = egui::pos2(315.0, 375.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );
        let _ = run_sel_frame(
            &ctx,
            press_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );
        // egui 在第二击 release 帧判定 double-click。
        let (note_event, _, _) = run_sel_frame(
            &ctx,
            release_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(0),
        );

        assert!(note_event.is_none(), "双击已有音符不得创建新音符");
    }

    /// 双击但 editing_track 无效（None）→ 不创建。
    #[test]
    fn double_click_without_editing_track_does_not_create() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        let midi = make_midi(vec![]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        let pos = egui::pos2(300.0, 375.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            press_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        // egui 在第二击 release 帧判定 double-click。
        let (note_event, _, _) = run_sel_frame(
            &ctx,
            release_event(pos),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert!(note_event.is_none(), "无 editing_track 时双击不得创建");
    }

    /// 框选到音符 → 普通选框；框选空区域 → 自动变垂直选框（全 128 键）。
    #[test]
    fn empty_marquee_becomes_vertical_selection() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        // 音符在 key 100（tick 100..200），框选 key 85..95 区域 → 无音符。
        let midi = make_midi(vec![(100, 100, 200, 0, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // key 85 → y=(127-85)*10=420；key 95 → y=(127-95)*10=320。
        let start = egui::pos2(50.0, 420.0);
        let end = egui::pos2(150.0, 320.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(start),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(end),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(end),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert_eq!(sel_rect.rects.len(), 1, "应有一个选框");
        let (t0, t1, kl, kh) = sel_rect.rects[0];
        assert_eq!((kl, kh), (0, 127), "空区域框选应变为全 128 键垂直选框");
        assert!(t0 < t1);
        // 选中范围也应覆盖全键。
        assert!(
            selected.rects.iter().any(|r| r.2 == 0 && r.3 == 127),
            "selected 应包含全键范围"
        );
    }

    /// 框选到音符 → 保持普通选框（不垂直化）。
    #[test]
    fn marquee_with_notes_stays_rectangular() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        // key 90（tick 100..200）在框选范围内。
        let midi = make_midi(vec![(90, 100, 200, 0, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // key 85..95 区域框选（key 90 音符在内）。
        let start = egui::pos2(50.0, 420.0);
        let end = egui::pos2(150.0, 320.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(start),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(end),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(end),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert_eq!(sel_rect.rects.len(), 1);
        let (_, _, kl, kh) = sel_rect.rects[0];
        assert!(
            kl >= 85 && kh <= 95,
            "有音符的选框应保持矩形范围，实际 kl={kl} kh={kh}"
        );
    }

    /// 单音符边缘伸缩（不用先选中）：press 音符右边缘 → 拖 → release 提交。
    #[test]
    fn select_tool_resizes_single_note_without_selection() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        // 音符 (tick 300..330, key 90)：右边缘 x=330，key 90 → y=375。
        let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        let press = egui::pos2(330.0, 375.0);
        let release = egui::pos2(360.0, 375.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(press),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let (_, _, pencil_drag) = run_sel_frame(
            &ctx,
            release_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert!(
            matches!(
                pencil_drag,
                Some(yinhe_types::PencilNoteDrag::ResizeRight {
                    track: 0,
                    start_tick: 300,
                    key: 90,
                    new_end_tick: 360,
                })
            ),
            "音符右边缘应从 330 伸到 360，实际 {pencil_drag:?}"
        );
        assert_eq!(note_drag_delta, None, "未选中时按音符边缘不得启动选区移动");
        assert!(selected.is_empty(), "选区不应被修改");
        assert!(sel_rect.is_empty(), "选框不应被修改");
    }

    /// 单音符左边缘伸缩（不用先选中）。
    #[test]
    fn select_tool_resizes_single_note_left_edge() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        // 音符 (tick 300..480, key 90)：左边缘 x=300，拖到 x=240（1/16 网格点）。
        let midi = make_midi(vec![(90, 300, 480, 0, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        let press = egui::pos2(300.0, 375.0);
        let release = egui::pos2(240.0, 375.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(press),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let (_, _, pencil_drag) = run_sel_frame(
            &ctx,
            release_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert!(
            matches!(
                pencil_drag,
                Some(yinhe_types::PencilNoteDrag::ResizeLeft {
                    track: 0,
                    start_tick: 300,
                    key: 90,
                    new_start_tick: 240,
                })
            ),
            "音符左边缘应从 300 缩到 240，实际 {pencil_drag:?}"
        );
        assert_eq!(cursor_tick, None, "伸缩后松开不得把 playhead 跳到释放位置");
    }

    /// 单音符移动（不用先选中）：press 音符中部 → 拖 → release 提交。
    #[test]
    fn select_tool_moves_single_note_without_selection() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        // 音符 (tick 300..330, key 90)：中心 (315, 375)，无任何选框。
        let midi = make_midi(vec![(90, 300, 330, 0, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // press tick 315 → snap 360；release tick 435 → snap 480：dt = +120。
        let press = egui::pos2(315.0, 375.0);
        let release = egui::pos2(435.0, 375.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(press),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );
        let (_, _, pencil_drag) = run_sel_frame(
            &ctx,
            release_event(release),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            None,
        );

        assert!(
            matches!(
                pencil_drag,
                Some(yinhe_types::PencilNoteDrag::Move {
                    track: 0,
                    start_tick: 300,
                    key: 90,
                    delta_ticks: 120,
                    delta_keys: 0,
                })
            ),
            "未选中音符应直接移动 +120 tick，实际 {pencil_drag:?}"
        );
        assert_eq!(note_drag_delta, None, "不得启动选区移动");
        assert!(selected.is_empty(), "选区不应被修改");
        assert!(sel_rect.is_empty(), "选框不应被修改");
    }

    /// bug 回归：editing_track 存在时，框选只作用于编辑音轨。
    #[test]
    fn marquee_respects_editing_track() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        view.viewport_h = 600.0;
        // track 0 和 track 5 在框选区域内都有音符。
        let midi = make_midi(vec![(90, 100, 200, 0, 100), (90, 100, 200, 5, 100)]);
        let mut selected = yinhe_core::Selection::default();
        let mut sel_rect = yinhe_editor_core::edit_state::SelRectState::default();
        let mut cursor_tick: Option<f64> = None;
        let mut note_drag_delta: Option<(i64, i32, bool)> = None;
        let mut note_resize_delta: Option<(yinhe_editor_core::ResizeSide, i64)> = None;

        // 框选 tick 50..250、key 85..95 区域（两个音符都在内）。
        let start = egui::pos2(50.0, 420.0);
        let end = egui::pos2(250.0, 320.0);
        let _ = run_sel_frame(
            &ctx,
            press_event(start),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(5),
        );
        let _ = run_sel_frame(
            &ctx,
            drag_event(end),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(5),
        );
        let _ = run_sel_frame(
            &ctx,
            release_event(end),
            &mut view,
            &midi,
            &mut selected,
            &mut cursor_tick,
            &mut note_drag_delta,
            &mut note_resize_delta,
            &mut sel_rect,
            Some(5),
        );

        assert_eq!(selected.rects.len(), 1, "框选应只产生一个选区 rect");
        let (_, _, _, _, tl, th) = selected.rects[0];
        assert_eq!((tl, th), (5, 5), "框选应只作用于编辑音轨 5");
        assert!(selected.contains(5, 100, 90), "编辑音轨音符应被选中");
        assert!(!selected.contains(0, 100, 90), "非编辑音轨的音符不得被选中");
    }
}
