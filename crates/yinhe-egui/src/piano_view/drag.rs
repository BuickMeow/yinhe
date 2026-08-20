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
        let pixel_rect =
            crate::selection::drag::music_sel_to_pixel_rect(view, t_start, t_end, key_lo, key_hi);
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
    let (main_px, _) = main_cross_x_y(view, (local.x, local.y));
    let tick = main_px_to_tick_dir(view, main_px);
    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
    Some(snapped.max(0.0))
}

/// sel_drag_frame 的帧内可变状态：5 个互斥拖拽状态机 + 帧输出。
///
/// 曾全部内联在 sel_drag_frame 一个 800+ 行的函数里；拆分后各状态机
/// 函数共享本结构，主函数只负责加载 / 分发 / 持久化。
struct SelDragFrameState {
    /// 选区整体移动：(origin_tick, origin_key, alt)。None = 未在移动。
    note_drag_origin: Option<(f64, f64, bool)>,
    /// 拖拽中预计算的选中音符（选区移动/选区缩放共用，press 时构建一次）。
    drag_notes: Option<Vec<SelDragNoteInfo>>,
    /// 拖拽中已触发预览的 key delta（每变化 1 key 触发一次整组预览）。
    preview_last_dk: i32,
    /// 选区边缘缩放：(side, origin_boundary_tick, other_boundary_tick)。
    sel_resize_state: Option<(ResizeSide, f64, f64)>,
    /// 单音符边缘伸缩：(side, track, start_tick, end_tick, key)。
    sel_note_resize: Option<SelNoteResize>,
    /// 单音符移动：(track, orig_start, orig_key, orig_end, press_tick, last_dk, alt)。
    sel_note_move: Option<(u16, u32, u8, u32, f64, i32, bool)>,
    /// 帧输出：幽灵音符 / 隐藏音符 / 预览请求。
    ghost_notes: Vec<GhostNote>,
    hidden_notes: Vec<HiddenNote>,
    preview_reqs: Vec<super::PreviewReq>,
}

/// 是否有选框工具的拖拽状态机正在进行（跨帧持久化状态）。
///
/// 供 `effective_tool` 在拖拽期间锁定选择工具：否则 Alt 拖拽克隆时
/// 鼠标一旦移出音符原位，hover 命中失败就会被误判为"悬停空白"，
/// 临时切成铅笔工具、中断本次拖拽。
pub(crate) fn sel_drag_in_progress(ui: &egui::Ui) -> bool {
    let id = ui.id();
    ui.data_mut(|d| {
        d.get_persisted::<Option<(f64, f64, bool)>>(id.with("note_drag_origin"))
            .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<(ResizeSide, f64, f64)>>(id.with("sel_resize_state"))
                .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<SelNoteResize>>(id.with("sel_note_resize_state"))
                .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<(u16, u32, u8, u32, f64, i32, bool)>>(
                id.with("sel_note_move_state"),
            )
            .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<((f64, f32), egui::Pos2, egui::Pos2)>>(id.with("sel_drag"))
                .is_some_and(|v| v.is_some())
    })
}

impl SelDragFrameState {
    /// 从 egui 持久化加载拖拽状态（拖拽跨帧保持）。
    fn load(ui: &mut egui::Ui) -> Self {
        Self {
            note_drag_origin: ui
                .data_mut(|d| d.get_persisted(ui.id().with("note_drag_origin")))
                .unwrap_or(None),
            drag_notes: ui
                .data_mut(|d| d.get_persisted(ui.id().with("drag_notes")))
                .unwrap_or(None),
            preview_last_dk: ui
                .data_mut(|d| d.get_persisted(ui.id().with("note_drag_preview_dk")))
                .unwrap_or(0),
            sel_resize_state: ui
                .data_mut(|d| d.get_persisted(ui.id().with("sel_resize_state")))
                .unwrap_or(None),
            sel_note_resize: ui
                .data_mut(|d| d.get_persisted(ui.id().with("sel_note_resize_state")))
                .unwrap_or(None),
            sel_note_move: ui
                .data_mut(|d| d.get_persisted(ui.id().with("sel_note_move_state")))
                .unwrap_or(None),
            ghost_notes: Vec::new(),
            hidden_notes: Vec::new(),
            preview_reqs: Vec::new(),
        }
    }

    /// 持久化拖拽状态（拖拽跨帧保持）。
    fn save(&mut self, ui: &mut egui::Ui) {
        // 解构出各状态字段的可变引用，避免闭包整体 move `self`。
        let Self {
            note_drag_origin,
            drag_notes,
            sel_resize_state,
            sel_note_resize,
            sel_note_move,
            ..
        } = self;
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("note_drag_origin"), note_drag_origin.take())
        });
        ui.data_mut(|d| d.insert_persisted(ui.id().with("drag_notes"), drag_notes.take()));
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("sel_resize_state"), sel_resize_state.take())
        });
        ui.data_mut(|d| {
            d.insert_persisted(
                ui.id().with("sel_note_resize_state"),
                sel_note_resize.take(),
            )
        });
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("sel_note_move_state"), sel_note_move.take())
        });
    }
}

/// 拖拽推出屏幕时的 auto-scroll + 视口 clamp（4 个状态机共用）。
fn drag_scroll_and_clamp(
    ui: &mut egui::Ui,
    view: &mut yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    total_ticks: f64,
    pos: egui::Pos2,
) {
    // auto-scroll：拖拽能推出屏幕（pos 未 clamp）。方向感知：clamp 按主轴/副轴
    // 拆分（纵向 scroll_x = 音高、scroll_y = 时间），由 view.clamp_scroll 统一处理。
    crate::selection::drag::auto_scroll_on_drag_dir(ui, view, music_rect, pos, |view, w, h| {
        view.clamp_scroll(w, h, total_ticks);
    });
    view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);
}

/// 位置 clamp 到 music_rect（避免鼠标飞出后产生异常值）并换算 local 坐标。
fn clamped_local(pos: egui::Pos2, content_rect: egui::Rect, music_rect: egui::Rect) -> (f32, f32) {
    let clamped = pos.clamp(music_rect.min, music_rect.max);
    (
        clamped.x - content_rect.min.x,
        clamped.y - content_rect.min.y,
    )
}

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

            // ── 音符 hit-test（不用先选中，与铅笔一致）──
            // 轨道作用域 = track_selected（空 = 全部）∩ track_visible。
            // 边缘 → 单音符伸缩；中部（未选中）→ 单音符移动。
            if let Some((mode, track, start_tick, end_tick, key)) =
                hit_test_note(midi, view, local, track_visible, track_selected)
            {
                match mode {
                    super::pencil::HitMode::ResizeLeft | super::pencil::HitMode::ResizeRight => {
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
                state.sel_resize_state = Some((side, origin_boundary_tick, other_boundary_tick));
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
                    let tick =
                        crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
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
            if let Some(pos) = pointer.hover_pos() {
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
                *note_drag_delta = Some((dt, dk, alt));
                sel_rect.update_drag(dt, dk);

                // Keep ghost/hidden alive on the release frame so the original
                // notes don't flash back before the model is updated.
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
            }
            state.preview_reqs.push(super::PreviewReq::Stop);
            sel_rect.end_drag();
            state.note_drag_origin = None;
            state.drag_notes = None;
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
            if let Some(pos) = pointer.hover_pos() {
                let (local_x, local_y) = clamped_local(pos, content_rect, music_rect);
                let (main_px, cross_px) = main_cross_x_y(view, (local_x, local_y));
                let raw_tick = main_px_to_tick_dir(view, main_px);
                let snapped_tick =
                    crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let dt = (snapped_tick - press_tick).round() as i64;
                let dk = if vertical {
                    0
                } else {
                    view.cross_px_to_key(cross_px) as i32 - orig_key as i32
                };
                if alt {
                    // Alt = 复制：先把该音符置为唯一选中，再走选区复制通道
                    // （duplicate_selected_to 复制后选区跟随副本，便于连续 Alt+拖动）。
                    selected.clear();
                    selected.add_rect_track(orig_start, orig_end, orig_key, orig_key, trk, trk);
                    *note_drag_delta = Some((dt, dk, true));
                } else {
                    *pencil_note_drag = Some(yinhe_types::PencilNoteDrag::Move {
                        track: trk,
                        start_tick: orig_start,
                        key: orig_key,
                        delta_ticks: dt,
                        delta_keys: dk,
                    });
                }
                // Keep ghost/hidden alive on the release frame
                let new_start = (orig_start as i64 + dt).max(0) as u32;
                let new_key = (orig_key as i32 + dk).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                state.ghost_notes.push((
                    new_start,
                    new_start + (orig_end - orig_start),
                    new_key,
                    trk,
                ));
                if !alt {
                    state.hidden_notes.push((trk, orig_start, orig_key));
                }
            }
            state.preview_reqs.push(super::PreviewReq::Stop);
            state.sel_note_move = None;
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
) -> SelFrameOut {
    // 双击写音符的提交（note + track），由 show() 转成 PianoViewEvent::AddNote。
    let mut note_event: SelNoteEvent = None;
    // 单音符边缘伸缩的提交（复用铅笔的单音符伸缩通道）。
    let mut pencil_note_drag: Option<yinhe_types::PencilNoteDrag> = None;

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
    );

    // 无编辑目标（未选中音轨 / 主音轨不可见 / 主音轨是 Conductor）时，
    // 禁止音符移动/缩放（选框工具也不允许这些操作），但框选与点选仍可用。
    let can_edit =
        super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx).is_some();
    if can_edit {
        // ── 四个互斥拖拽状态机（同一时刻至多一个激活）──
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
    // egui 在第二击 release 时判定 double-click。条件：
    // - 无 note drag / resize 进行中（排除双击选框内音符/边缘的情况）
    // - 不在浮动工具条上（防事件穿透）
    // - write_track 有效且点击位置无音符 → 创建，长度 = 一个量化间隔。
    // 双击命中音符时 double_click_note 返回 None，保持选中/拖拽行为。
    if ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    }) && state.note_drag_origin.is_none()
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

/// 双击写音符：write_track 有效且点击位置无音符时创建新音符。
///
/// 音符长度 = 一个量化间隔（与铅笔点击一致）。返回 `(note, track)`。
/// 命中已有音符（write_track 上）时返回 `None`——双击保持选中/拖拽行为。
#[allow(clippy::too_many_arguments)]
fn double_click_note(
    midi: Option<&dyn yinhe_types::NoteSource>,
    write_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> Option<(yinhe_core::NoteEvent, u16)> {
    let track = super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx)?;
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let raw_tick = main_px_to_tick_dir(view, main_px);
    let key = view.cross_px_to_key(cross_px);
    // 点击位置已有音符（write_track 上）→ 不创建。
    // key_notes_in_range 左边界保守（tick - max_note_len），右边界精确，
    // 任何覆盖该像素点的音符都会被包含；像素判定过滤跨边界长音符。
    if let Some(midi) = midi {
        let hit = midi
            .key_notes_in_range(key, raw_tick as u32, (raw_tick + 1.0) as u32)
            .any(|n| {
                n.track == track
                    && tick_to_main_px_dir(view, n.start_tick as f64) <= main_px
                    && main_px <= tick_to_main_px_dir(view, n.end_tick as f64)
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
/// 轨道作用域 = track_selected（空 = 全部）∩ track_visible。
/// 只查可能覆盖鼠标点的音符：key_notes_in_range 左边界保守（tick - max_note_len），
/// 右边界精确，每帧 hover 开销与铅笔 hit-test 同级。
pub(crate) fn hit_test_note(
    midi: Option<&dyn yinhe_types::NoteSource>,
    view: &yinhe_types::PianoRollView,
    local: egui::Pos2,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
) -> Option<(super::pencil::HitMode, u16, u32, u32, u8)> {
    const EDGE_THRESHOLD_PX: f32 = 6.0;
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let (midi, key) = (midi?, view.cross_px_to_key(cross_px));
    let raw_tick = main_px_to_tick_dir(view, main_px);
    let notes = midi.key_notes_in_range(key, raw_tick as u32, (raw_tick + 1.0) as u32);
    for note in notes {
        // 轨道作用域：track_selected（空 = 全部）∩ track_visible。
        let in_scope = (track_selected.is_empty() || track_selected.contains(&note.track))
            && track_visible
                .get(note.track as usize)
                .copied()
                .unwrap_or(true);
        if !in_scope {
            continue;
        }
        // 方向感知的像素矩形：横向 x = tick、y = key；纵向 x = key、y = tick。
        let a = tick_to_main_px_dir(view, note.start_tick as f64);
        let b = tick_to_main_px_dir(view, note.end_tick as f64);
        let c = view.key_to_cross_px(key);
        let note_rect = orient_rect(view, a, b, c, c + view.key_height);
        if !note_rect.contains(local) {
            continue;
        }
        // 主轴上到两端距离：起点 = 伸缩左缘，终点 = 伸缩右缘。
        let dist_start = (main_px - a).abs();
        let dist_end = (main_px - b).abs();
        let mode = if dist_start <= EDGE_THRESHOLD_PX {
            super::pencil::HitMode::ResizeLeft
        } else if dist_end <= EDGE_THRESHOLD_PX {
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
#[path = "drag_tests.rs"]
mod tests;
