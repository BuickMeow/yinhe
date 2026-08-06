//! Pencil-tool input handling for the piano-roll view.

use eframe::egui;
use rust_i18n::t;

use super::PencilNoteDrag;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

/// Internal pencil-tool drag mode persisted across frames.
#[derive(Clone)]
pub(crate) enum PencilDrag {
    /// Creating a new note: (start_tick, key)
    Create(f64, u8),
    /// Moving an existing note: (track, original_start_tick, original_key, original_end, press_snapped_tick, last_dk)
    Move(u16, u32, u8, u32, f64, i32),
    /// Resizing right edge: (track, start_tick, end_tick, key)
    ResizeRight(u16, u32, u32, u8),
    /// Resizing left edge: (track, start_tick, end_tick, key)
    ResizeLeft(u16, u32, u32, u8),
}

/// Result of hit-testing the cursor against existing notes.
pub(crate) struct HitNote {
    pub track: u16,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub mode: HitMode,
}

#[derive(Clone)]
pub(crate) enum HitMode {
    Move,
    ResizeLeft,
    ResizeRight,
}

/// Returns the single valid target track for the Pencil tool, if any.
///
/// 规则：editing_track 必须存在、可见，且不能是 conductor
/// （conductor 不能放音符，只能编辑 Tempo automation）。
/// editing_track 一旦设置就常驻 PR 显示（见 layout.rs pr_visible），
/// 不再额外要求 track_selected —— 旧判断已退役。
pub(crate) fn valid_pencil_track(
    editing_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
) -> Option<u16> {
    let track = editing_track?;
    if Some(track) == conductor_idx {
        return None;
    }
    if !track_visible.get(track as usize).copied().unwrap_or(false) {
        return None;
    }
    Some(track)
}

/// Pencil-tool input handling: hover preview, click to write a note, drag to lengthen,
/// or hover over / drag existing notes to move or resize them.
/// Returns `(note_event, ghost_notes, hidden_notes, pencil_note_drag)`.
/// ghost_notes are (start_tick, end_tick, key, track) as u32/u8/u16 — color fetched from storage buffer in shader.
/// hidden_notes are (track, start_tick, key) for notes being dragged.
/// Pencil 工具的帧输出：新建音符、ghost/hidden、松手拖拽提交、听觉预览请求。
type PencilFrameOut = (
    Option<yinhe_core::NoteEvent>,
    Vec<super::drag::GhostNote>,
    Vec<super::drag::HiddenNote>,
    Option<PencilNoteDrag>,
    Option<super::PreviewReq>,
);

#[allow(clippy::too_many_arguments)]
pub(crate) fn pencil_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    editing_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
    midi: Option<&dyn yinhe_types::NoteSource>,
    _track_colors: &[[f32; 4]],
    total_ticks: f64,
) -> PencilFrameOut {
    let pencil_id = ui.id().with("pencil_drag");
    let mut drag_state: Option<PencilDrag> =
        ui.data_mut(|d| d.get_persisted(pencil_id)).unwrap_or(None);

    // 音符听觉预览请求（key 变化 / 新建按下 / 松手）。
    let mut preview_req: Option<super::PreviewReq> = None;

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state.
    if drag_state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (None, Vec::new(), Vec::new(), None, None);
    }

    let hover_pos = pointer.hover_pos();
    let can_write = valid_pencil_track(editing_track, track_visible, conductor_idx).is_some();
    let track = valid_pencil_track(editing_track, track_visible, conductor_idx);
    let track_idx = track.unwrap_or(0);

    // Hover / drag preview.
    // 拖拽中：允许鼠标越出 music_rect，用 clamp 限定位置 + 触发 auto-scroll。
    // 非拖拽：严格要求 music_rect.contains（hit-test 和创建前 hover 预览）。
    let preview = hover_pos.and_then(|pos| {
        let pos = if drag_state.is_some() {
            pos.clamp(music_rect.min, music_rect.max)
        } else if music_rect.contains(pos) {
            pos
        } else {
            return None;
        };
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let raw_tick = view.x_to_tick(local.x);
        let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
        let key = view.y_to_key(local.y);
        Some((tick.max(0.0), key))
    });

    // ── Hit-test existing notes (only when not dragging) ──
    // Returns the closest note under cursor with its hit mode.
    // This is independent of `preview` / `snap_tick` so that clicking
    // on a note always starts a drag, never accidentally creates a new note.
    const EDGE_THRESHOLD_PX: f32 = 6.0;
    let kb_w = music_rect.min.x - content_rect.min.x;

    let hit_note = if drag_state.is_none() && can_write {
        // Use a closure so `?` returns from the closure, not from pencil_frame
        (|| -> Option<HitNote> {
            let mouse_screen = hover_pos?;
            if !music_rect.contains(mouse_screen) {
                return None;
            }
            let mouse_local_x = mouse_screen.x - music_rect.min.x;
            let mouse_local_y = mouse_screen.y - music_rect.min.y;
            let key = view.y_to_key(mouse_local_y);
            let midi = midi?;
            let active_track = track?;
            let notes = midi.key_notes_in_range(key, 0, u32::MAX);

            for note in notes {
                // 只命中正在编辑的 track，避免跨 track 误触（即使其他 track
                // 的音符在 PR 中不可见，数据中仍存在）。
                if note.track != active_track {
                    continue;
                }
                let note_left = view.tick_to_x(note.start_tick as f64) - kb_w;
                let note_right = view.tick_to_x(note.end_tick as f64) - kb_w;
                let note_top = view.key_to_y(key);
                let note_bottom = note_top + view.key_height;

                if mouse_local_x >= note_left
                    && mouse_local_x <= note_right
                    && mouse_local_y >= note_top
                    && mouse_local_y <= note_bottom
                {
                    let dist_left = (mouse_local_x - note_left).abs();
                    let dist_right = (mouse_local_x - note_right).abs();
                    let mode = if dist_left < EDGE_THRESHOLD_PX {
                        HitMode::ResizeLeft
                    } else if dist_right < EDGE_THRESHOLD_PX {
                        HitMode::ResizeRight
                    } else {
                        HitMode::Move
                    };
                    return Some(HitNote {
                        track: note.track,
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        key,
                        mode,
                    });
                }
            }
            None
        })()
    } else {
        None
    };

    // ── Set cursor based on hit test ──
    if let Some(ref hit) = hit_note {
        match hit.mode {
            HitMode::ResizeLeft => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest),
            HitMode::ResizeRight => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast),
            HitMode::Move => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
        }
    }

    // ── Ghost notes: only when not over an existing note ──
    let mut ghost_notes: Vec<super::drag::GhostNote> = Vec::new();
    let mut hidden_notes: Vec<super::drag::HiddenNote> = Vec::new();
    if can_write
        && drag_state.is_none()
        && hit_note.is_none()
        && let Some((tick, key)) = preview
    {
        let interval = quantize.tick_interval(ppq) as f64;
        // Not dragging (drag_state is None due to the outer condition),
        // show preview at hover position
        ghost_notes.push((tick as u32, (tick + interval) as u32, key, track_idx));
    }

    // ── Start drag ──
    if pointer.primary_pressed() {
        if let Some(hit) = hit_note {
            // 点击音符出声（像键盘预览一样）：播放该音符（gate 长度，原力度）。
            // vel <= 1 的音符（黑乐谱隐藏音符）不响，与播放筛除一致。
            if let Some(vel) = note_velocity(midi, hit.track, hit.start_tick, hit.key)
                && vel > 1
            {
                preview_req = Some(super::PreviewReq::Note(super::NotePreview {
                    track: hit.track,
                    key: hit.key,
                    velocity: Some(vel),
                    target_tick: hit.start_tick,
                    duration_ticks: hit.end_tick - hit.start_tick,
                }));
            }
            let new_drag = match hit.mode {
                HitMode::ResizeLeft => {
                    PencilDrag::ResizeLeft(hit.track, hit.start_tick, hit.end_tick, hit.key)
                }
                HitMode::ResizeRight => {
                    PencilDrag::ResizeRight(hit.track, hit.start_tick, hit.end_tick, hit.key)
                }
                HitMode::Move => {
                    let press_tick = preview.map(|(t, _)| t).unwrap_or(0.0);
                    PencilDrag::Move(
                        hit.track,
                        hit.start_tick,
                        hit.key,
                        hit.end_tick,
                        press_tick,
                        0,
                    )
                }
            };
            ui.data_mut(|d| d.insert_persisted(pencil_id, Some(new_drag)));
        } else if let Some((tick, key)) = preview {
            ui.data_mut(|d| d.insert_persisted(pencil_id, Some(PencilDrag::Create(tick, key))));
            // 音符预览：按住期间持续响，松手 Stop。力度用该音轨最近修改值。
            preview_req = Some(super::PreviewReq::Note(super::NotePreview {
                track: track_idx,
                key,
                velocity: None,
                target_tick: tick.max(0.0) as u32,
                duration_ticks: 0,
            }));
        }
    }

    // ── Compute drag output ──
    let mut result = None;
    let mut pencil_note_drag = None;
    // Move 分支的 last_dk 在拖拽中更新，需要写回持久化。
    // 注意不能无条件写回帧首快照：那会把 press/release 分支刚写入的
    // 状态（Create / None）覆盖回旧值，导致点击创建音符失效。
    let mut persist_state = false;

    match &mut drag_state {
        Some(PencilDrag::Create(s_tick, s_key)) => {
            // Show ghost while dragging (before release)
            if pointer.primary_down() && !pointer.primary_released() {
                // auto-scroll：让长音符能拖出屏幕（pos 未 clamp）
                if let Some(pos) = hover_pos {
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
                }
                if let Some((tick, key)) = preview {
                    let interval = quantize.tick_interval(ppq) as f64;
                    let current_end = tick.max(*s_tick + interval);
                    // 向右拖不超过一个量化时，key 跟随鼠标（可上下拖变调），
                    // 像移动音符那样：变 key 播放一次（长度 = 当前 gate）。
                    // 超过一个量化后 key 锁定，继续拖长度。
                    if current_end - *s_tick <= interval && key != *s_key {
                        *s_key = key;
                        persist_state = true; // 变 key 后的最终 key 要随 release 提交
                        preview_req = Some(super::PreviewReq::Note(super::NotePreview {
                            track: track_idx,
                            key: *s_key,
                            velocity: None,
                            target_tick: *s_tick as u32,
                            duration_ticks: (current_end - *s_tick) as u32,
                        }));
                    }
                    ghost_notes.push((*s_tick as u32, current_end as u32, *s_key, track_idx));

                    // ── Tooltip：显示 key / tick / gate ──
                    let gate = (current_end - *s_tick) as u32;
                    let lines = vec![
                        t!("pencil.key", n = s_key).to_string(),
                        t!("pencil.tick", n = *s_tick as u32).to_string(),
                        t!("pencil.gate", n = gate).to_string(),
                    ];
                    if let Some(pos) = hover_pos {
                        crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
                    }
                }
            }
            // Release -> commit note.
            if pointer.primary_released() {
                preview_req = Some(super::PreviewReq::Stop);
                if can_write {
                    let interval = quantize.tick_interval(ppq) as f64;
                    let end_tick = if let Some((tick, _)) = preview {
                        let current_end = tick.max(*s_tick + interval);
                        let snapped_end = crate::view_interaction::snap_tick_ceil(
                            current_end,
                            quantize,
                            ppq,
                            bar_line_data,
                        );
                        snapped_end.max(*s_tick + interval)
                    } else {
                        *s_tick + interval
                    };
                    result = Some(yinhe_core::NoteEvent {
                        id: 0, // 由 Document::add_note 分配
                        start_tick: *s_tick as u32,
                        end_tick: end_tick as u32,
                        key: *s_key,
                        velocity: 100,
                    });
                }
                ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
            }
        }
        Some(PencilDrag::Move(trk, orig_tick, orig_key, orig_end, press_tick, last_dk)) => {
            // auto-scroll：让音符能拖出屏幕（pos 未 clamp）
            if pointer.primary_down()
                && !pointer.primary_released()
                && let Some(pos) = hover_pos
            {
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
            }
            if let Some((tick, key)) = preview {
                let dt = (tick as i64) - (*press_tick as i64);
                let dk = (key as i32) - (*orig_key as i32);

                // 音符预览：每变化 1 key 触发一次，长度 = 音符 gate，力度 = 音符原值。
                // vel <= 1 的音符（黑乐谱隐藏音符）不预览，与播放筛除一致。
                if dk != *last_dk {
                    *last_dk = dk;
                    persist_state = true;
                    if let Some(vel) = note_velocity(midi, *trk, *orig_tick, *orig_key)
                        && vel > 1
                    {
                        preview_req = Some(super::PreviewReq::Note(super::NotePreview {
                            track: *trk,
                            key: (*orig_key as i32 + dk).clamp(0, 127) as u8,
                            velocity: Some(vel),
                            target_tick: (*orig_tick as i64 + dt).max(0) as u32,
                            duration_ticks: *orig_end - *orig_tick,
                        }));
                    }
                }

                // Show ghost at the dragged position for visual feedback.
                // The original note stays in place until release.
                let new_start = (*orig_tick as i64 + dt).max(0) as u32;
                let new_end = new_start + (*orig_end - *orig_tick);
                ghost_notes.push((new_start, new_end, key, *trk));
                hidden_notes.push((*trk, *orig_tick, *orig_key));

                // ── Tooltip：显示 ±key / ±tick（已按量化 snap）──
                let lines = vec![
                    crate::view_interaction::format_signed("tick", dt),
                    crate::view_interaction::format_signed("key", dk as i64),
                ];
                if let Some(pos) = hover_pos {
                    crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
                }

                // Only output drag on release — do NOT modify the model during drag.
                if pointer.primary_released() {
                    preview_req = Some(super::PreviewReq::Stop);
                    pencil_note_drag = Some(PencilNoteDrag::Move {
                        track: *trk,
                        start_tick: *orig_tick,
                        key: *orig_key,
                        delta_ticks: dt,
                        delta_keys: dk,
                    });
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            } else {
                if pointer.primary_released() {
                    preview_req = Some(super::PreviewReq::Stop);
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            }
        }
        Some(PencilDrag::ResizeRight(trk, orig_tick, orig_end, orig_key)) => {
            // auto-scroll：右边缘能拖出屏幕
            if pointer.primary_down()
                && !pointer.primary_released()
                && let Some(pos) = hover_pos
            {
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
            }
            if let Some((tick, _)) = preview {
                let interval = quantize.tick_interval(ppq) as f64;
                let snapped = crate::view_interaction::snap_tick_ceil(
                    tick.max(*orig_tick as f64 + interval),
                    quantize,
                    ppq,
                    bar_line_data,
                );
                let new_end = snapped
                    .max(*orig_tick as f64 + interval)
                    .min(u32::MAX as f64) as u32;

                // Show ghost and hide original note
                ghost_notes.push((*orig_tick, new_end, *orig_key, *trk));
                hidden_notes.push((*trk, *orig_tick, *orig_key));

                // ── Tooltip：显示 ±gate（新长度 - 原长度）──
                let orig_gate = *orig_end as i64 - *orig_tick as i64;
                let new_gate = new_end as i64 - *orig_tick as i64;
                let lines = vec![crate::view_interaction::format_signed(
                    "gate",
                    new_gate - orig_gate,
                )];
                if let Some(pos) = hover_pos {
                    crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
                }

                // Only output on release
                if pointer.primary_released() {
                    preview_req = Some(super::PreviewReq::Stop);
                    pencil_note_drag = Some(PencilNoteDrag::ResizeRight {
                        track: *trk,
                        start_tick: *orig_tick,
                        key: *orig_key,
                        new_end_tick: new_end,
                    });
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            } else {
                if pointer.primary_released() {
                    preview_req = Some(super::PreviewReq::Stop);
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            }
        }
        Some(PencilDrag::ResizeLeft(trk, orig_tick, orig_end, orig_key)) => {
            // auto-scroll：左边缘能拖出屏幕
            if pointer.primary_down()
                && !pointer.primary_released()
                && let Some(pos) = hover_pos
            {
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
            }
            if let Some((tick, _)) = preview {
                let interval = quantize.tick_interval(ppq) as f64;
                let snapped =
                    crate::view_interaction::snap_tick_floor(tick, quantize, ppq, bar_line_data);
                let new_start = (snapped as u32).min(*orig_end - 1);
                // Ensure minimum length: new_start must be <= orig_end - interval
                let max_start = (*orig_end as f64 - interval).max(0.0) as u32;
                let new_start = new_start.min(max_start);

                // Show ghost and hide original note
                ghost_notes.push((new_start, *orig_end, *orig_key, *trk));
                hidden_notes.push((*trk, *orig_tick, *orig_key));

                // ── Tooltip：显示 ±tick / ±gate（左端变化量 + 长度变化量）──
                let dt = new_start as i64 - *orig_tick as i64;
                let orig_gate = *orig_end as i64 - *orig_tick as i64;
                let new_gate = *orig_end as i64 - new_start as i64;
                let lines = vec![
                    crate::view_interaction::format_signed("tick", dt),
                    crate::view_interaction::format_signed("gate", new_gate - orig_gate),
                ];
                if let Some(pos) = hover_pos {
                    crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
                }

                // Only output on release
                if pointer.primary_released() {
                    preview_req = Some(super::PreviewReq::Stop);
                    pencil_note_drag = Some(PencilNoteDrag::ResizeLeft {
                        track: *trk,
                        start_tick: *orig_tick,
                        key: *orig_key,
                        new_start_tick: new_start,
                    });
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            } else {
                if pointer.primary_released() {
                    preview_req = Some(super::PreviewReq::Stop);
                    ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
                }
            }
        }
        None => {}
    }

    // 仅在 Move 的 last_dk 变化时写回（带新 last_dk 的完整状态）。
    if persist_state {
        ui.data_mut(|d| d.insert_persisted(pencil_id, drag_state));
    }

    (
        result,
        ghost_notes,
        hidden_notes,
        pencil_note_drag,
        preview_req,
    )
}

/// 查音符的 velocity（预览用）：按 (track, start_tick, key) 定位。
fn note_velocity(
    midi: Option<&dyn yinhe_types::NoteSource>,
    track: u16,
    start_tick: u32,
    key: u8,
) -> Option<u8> {
    let midi = midi?;
    midi.key_notes_in_range(key, start_tick, start_tick.saturating_add(1))
        .find(|n| n.track == track && n.start_tick == start_tick)
        .map(|n| n.velocity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piano_view::PreviewReq;
    use eframe::egui;

    /// 空音符源（点击创建测试用）。
    struct MockNotes {
        buckets: [yinhe_types::NoteBucket; 128],
    }
    impl MockNotes {
        fn new() -> Self {
            Self {
                buckets: std::array::from_fn(|_| yinhe_types::NoteBucket::default()),
            }
        }

        fn with_note(mut self, start: u32, end: u32, key: u8, vel: u8) -> Self {
            self.buckets[key as usize].insert_sorted(yinhe_types::Note {
                id: 0,
                start_tick: start,
                end_tick: end,
                velocity: vel,
                track: 0,
            });
            self
        }
    }
    impl yinhe_types::NoteSource for MockNotes {
        fn key_notes(&self, key: u8) -> &yinhe_types::NoteBucket {
            &self.buckets[key as usize]
        }
        fn duration(&self) -> f64 {
            0.0
        }
    }

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

    fn frame_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    /// 跑一帧 pencil_frame，返回（新建音符事件，听觉预览请求）。
    fn run_frame(
        ctx: &egui::Context,
        raw: egui::RawInput,
        view: &mut yinhe_types::PianoRollView,
        midi: &MockNotes,
    ) -> (Option<yinhe_core::NoteEvent>, Option<PreviewReq>) {
        let mut out = (None, None);
        let _ = ctx.run_ui(raw, |ui| {
            let r = pencil_frame(
                ui,
                frame_rect(),
                frame_rect(),
                view,
                QuantizePreset::Fraction(1, 16), // 与 PR 默认量化一致（网格 120）
                480,
                None,
                Some(0),
                &[true],
                None,
                Some(midi),
                &[],
                1000.0,
            );
            out = (r.0, r.4);
        });
        out
    }

    /// 拖拽帧：按钮保持按下，鼠标移动到新位置。
    fn drag_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw
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

    /// 回归测试：点击必须创建音符（曾因帧首快照被无条件写回 persisted、
    /// 覆盖 press 分支刚存入的状态而失效，音符永远无法创建）。
    /// 同时验证第二次点击不被残留状态污染（音符创建在旧位置）。
    #[test]
    fn click_creates_note_at_click_position() {
        let ctx = egui::Context::default();
        let midi = MockNotes::new();
        let mut view = test_view();

        let pos = egui::pos2(120.0, 50.0); // 120 是 4 分音符量化的网格点
        let expected_key = view.y_to_key(50.0);

        // press 帧：不创建
        assert!(
            run_frame(&ctx, press_event(pos), &mut view, &midi)
                .0
                .is_none()
        );
        // release 帧：创建音符
        let (note, _) = run_frame(&ctx, release_event(pos), &mut view, &midi);
        let note = note.expect("点击应创建音符");
        assert_eq!(note.start_tick, 120);
        assert_eq!(note.key, expected_key);

        // 第二次点击不同位置：必须在新位置创建（旧 bug：残留 Create 状态
        // 导致音符被创建在第一次点击的位置）。
        let pos2 = egui::pos2(360.0, 100.0); // 量化网格点（120×3）
        let expected_key2 = view.y_to_key(100.0);
        assert!(
            run_frame(&ctx, press_event(pos2), &mut view, &midi)
                .0
                .is_none()
        );
        let (note2, _) = run_frame(&ctx, release_event(pos2), &mut view, &midi);
        let note2 = note2.expect("第二次点击应创建音符");
        assert_eq!(note2.start_tick, 360);
        assert_eq!(note2.key, expected_key2);
    }

    /// 点击已有音符（tick 120~240，key 122）→ 立即出声（像键盘预览）。
    /// 测试视图高 600px、128 键 × 10px：key 122 在 y≈55 可见。
    #[test]
    fn click_on_note_triggers_preview() {
        let ctx = egui::Context::default();
        let midi = MockNotes::new().with_note(120, 240, 122, 100);
        let mut view = test_view();
        let key_y = view.key_to_y(122) + view.key_height / 2.0;

        let (_, preview) = run_frame(
            &ctx,
            press_event(egui::pos2(150.0, key_y)),
            &mut view,
            &midi,
        );
        let PreviewReq::Note(p) = preview.expect("点击音符应触发预览") else {
            panic!("应为 Note 预览");
        };
        assert_eq!(p.key, 122);
        assert_eq!(p.velocity, Some(100));
        assert_eq!(p.target_tick, 120);
        assert_eq!(p.duration_ticks, 120, "gate = end - start");
    }

    /// vel <= 1 的音符（黑乐谱隐藏音符）点击不预览。
    #[test]
    fn click_on_vel1_note_does_not_preview() {
        let ctx = egui::Context::default();
        let midi = MockNotes::new().with_note(120, 240, 122, 1);
        let mut view = test_view();
        let key_y = view.key_to_y(122) + view.key_height / 2.0;

        let (_, preview) = run_frame(
            &ctx,
            press_event(egui::pos2(150.0, key_y)),
            &mut view,
            &midi,
        );
        assert!(preview.is_none(), "vel=1 隐藏音符不预览");
    }

    /// 新建音符：向右拖不超过一个量化时，上下拖动可改变 key，并像移动音符那样出声。
    #[test]
    fn create_drag_changes_key_within_one_quantize() {
        let ctx = egui::Context::default();
        let midi = MockNotes::new();
        let mut view = test_view();

        // press：tick 120，key 122 区域（屏幕内可见）
        let p0 = egui::pos2(120.0, view.key_to_y(122) + view.key_height / 2.0);
        let (_, preview) = run_frame(&ctx, press_event(p0), &mut view, &midi);
        let PreviewReq::Note(p) = preview.expect("按下应触发持续音预览") else {
            panic!("应为 Note 预览");
        };
        assert_eq!(p.key, 122);
        assert_eq!(p.duration_ticks, 0, "新建按住为持续音");

        // 拖拽帧：横向不动（仍在一个量化内），纵向移到 key 125
        let p1 = egui::pos2(120.0, view.key_to_y(125) + view.key_height / 2.0);
        let (_, preview) = run_frame(&ctx, drag_event(p1), &mut view, &midi);
        let PreviewReq::Note(p) = preview.expect("变 key 应触发预览") else {
            panic!("应为 Note 预览");
        };
        assert_eq!(p.key, 125, "量化内上下拖改变 key");
        assert!(p.duration_ticks > 0, "变 key 播放为定长（像移动音符）");

        // release：音符用最终 key
        let (note, _) = run_frame(&ctx, release_event(p1), &mut view, &midi);
        let note = note.expect("松手应创建音符");
        assert_eq!(note.key, 125);
        assert_eq!(note.start_tick, 120);
    }
}
