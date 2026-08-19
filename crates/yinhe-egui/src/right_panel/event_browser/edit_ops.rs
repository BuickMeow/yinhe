//! Event browser 的删除/插入操作（多选删除、右键插入、空表加号）。
//!
//! 与 `edit/` 子模块（单字段编辑 popup）不同，本模块处理整行删除和插入。
//! 每个 `apply_*_ops` 函数检查 `EditRequest::DeleteSelected` / `InsertAbove` /
//! `InsertBelow` / `InsertFirst`，执行对应 Document 方法并 push undo。
//!
//! 所有事件列表共用 `UndoAction::EventList`，每个事件类型只需提供：
//! - `target`：写入目标
//! - `snapshot(doc) -> Vec<EventListItem>`：取当前事件列表快照
//! - `delete(doc, &ticks)`：删除选中事件
//! - `insert(doc, tick)`：在 tick 处插入新事件

use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{EventListItem, EventListTarget, UndoAction};

use super::edit::push_event_list_undo;
use super::state::{EditRequest, EventBrowserState};
use super::table::{peek_edit_request, remove_edit_request};

/// 获取当前播放位置 tick（用于新建事件的默认 tick）。
fn current_tick(doc: &Document) -> u32 {
    doc.edit.cursor_tick.map(|t| t.max(0.0) as u32).unwrap_or(0)
}

/// 清空选中状态 + 清除 EditRequest（删除/插入操作完成后调用）。
fn cleanup(state: &mut EventBrowserState, ui: &egui::Ui, salt: &str) {
    state.selected_ticks.clear();
    state.last_clicked_tick = None;
    remove_edit_request(ui, salt);
}

/// 事件列表删除/插入操作的统一分派。
///
/// `ctx` 封装了事件类型相关的 4 个回调，避免每个 `apply_*_ops` 写一遍样板。
fn dispatch_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
    target: EventListTarget,
    ctx: EventOpsCtx,
) {
    let Some(req) = peek_edit_request(ui, salt) else {
        return;
    };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let snapshot = doc.capture_snapshot();
                let before = (ctx.snapshot)(doc);
                (ctx.delete)(doc, &state.selected_ticks);
                let after = (ctx.snapshot)(doc);
                push_event_list_undo(doc, target, before, after, ctx.delete_label, snapshot);
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let snapshot = doc.capture_snapshot();
            let before = (ctx.snapshot)(doc);
            (ctx.insert)(doc, tick);
            let after = (ctx.snapshot)(doc);
            push_event_list_undo(doc, target, before, after, ctx.insert_label, snapshot);
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let snapshot = doc.capture_snapshot();
            let before = (ctx.snapshot)(doc);
            (ctx.insert)(doc, tick);
            let after = (ctx.snapshot)(doc);
            push_event_list_undo(doc, target, before, after, ctx.first_label, snapshot);
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

/// 事件列表快照函数：编辑前捕获 before/after 用。
type SnapshotFn = Box<dyn Fn(&Document) -> Vec<EventListItem>>;
/// 按选中的 tick 集合批量删除。
type DeleteFn = Box<dyn Fn(&mut Document, &std::collections::HashSet<u32>)>;
/// 在指定 tick 插入新事件。
type InsertFn = Box<dyn Fn(&mut Document, u32)>;

/// 单个事件列表类型的操作上下文。
///
/// 使用 `Box<dyn Fn>` 而非 `fn` 指针，因为 per-track 类型需要在闭包中捕获 `track`。
struct EventOpsCtx {
    snapshot: SnapshotFn,
    delete: DeleteFn,
    insert: InsertFn,
    delete_label: &'static str,
    insert_label: &'static str,
    first_label: &'static str,
}

// ── TimeSig ──

pub fn apply_timesig_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::TimeSig,
        EventOpsCtx {
            snapshot: Box::new(|doc| {
                doc.data
                    .model
                    .conductor
                    .time_sig
                    .iter()
                    .cloned()
                    .map(EventListItem::TimeSig)
                    .collect()
            }),
            delete: Box::new(|doc, ticks| {
                doc.delete_time_sig_events(ticks);
            }),
            insert: Box::new(|doc, tick| doc.insert_time_sig_event(tick)),
            delete_label: "删除拍号事件",
            insert_label: "插入拍号事件",
            first_label: "新建拍号事件",
        },
    );
}

// ── KeySig ──

pub fn apply_keysig_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::KeySig,
        EventOpsCtx {
            snapshot: Box::new(|doc| {
                doc.data
                    .model
                    .conductor
                    .key_sig
                    .iter()
                    .cloned()
                    .map(EventListItem::KeySig)
                    .collect()
            }),
            delete: Box::new(|doc, ticks| {
                doc.delete_key_sig_events(ticks);
            }),
            insert: Box::new(|doc, tick| doc.insert_key_sig_event(tick)),
            delete_label: "删除调号事件",
            insert_label: "插入调号事件",
            first_label: "新建调号事件",
        },
    );
}

// ── Marker ──

pub fn apply_marker_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::Marker,
        EventOpsCtx {
            snapshot: Box::new(|doc| {
                doc.data
                    .model
                    .conductor
                    .markers
                    .iter()
                    .cloned()
                    .map(EventListItem::Marker)
                    .collect()
            }),
            delete: Box::new(|doc, ticks| {
                doc.delete_marker_events(ticks);
            }),
            insert: Box::new(|doc, tick| doc.insert_marker_event(tick)),
            delete_label: "删除标记事件",
            insert_label: "插入标记事件",
            first_label: "新建标记事件",
        },
    );
}

// ── Conductor Lyrics ──

pub fn apply_conductor_lyrics_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::ConductorLyrics,
        EventOpsCtx {
            snapshot: Box::new(|doc| {
                doc.data
                    .model
                    .conductor
                    .lyrics
                    .iter()
                    .cloned()
                    .map(EventListItem::Lyrics)
                    .collect()
            }),
            delete: Box::new(|doc, ticks| {
                doc.delete_conductor_lyrics_events(ticks);
            }),
            insert: Box::new(|doc, tick| doc.insert_conductor_lyrics_event(tick)),
            delete_label: "删除歌词事件",
            insert_label: "插入歌词事件",
            first_label: "新建歌词事件",
        },
    );
}

// ── Conductor Chord ──

pub fn apply_conductor_chord_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::ConductorChord,
        EventOpsCtx {
            snapshot: Box::new(|doc| {
                doc.data
                    .model
                    .conductor
                    .chord
                    .iter()
                    .cloned()
                    .map(EventListItem::Chord)
                    .collect()
            }),
            delete: Box::new(|doc, ticks| {
                doc.delete_conductor_chord_events(ticks);
            }),
            insert: Box::new(|doc, tick| doc.insert_conductor_chord_event(tick)),
            delete_label: "删除和弦事件",
            insert_label: "插入和弦事件",
            first_label: "新建和弦事件",
        },
    );
}

// ── Per-track Lyrics ──

pub fn apply_lyrics_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
    track: u16,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::Lyrics { track },
        EventOpsCtx {
            snapshot: Box::new(move |doc| {
                doc.data
                    .model
                    .tracks
                    .get(track as usize)
                    .map(|t| {
                        t.lyrics
                            .iter()
                            .cloned()
                            .map(EventListItem::Lyrics)
                            .collect()
                    })
                    .unwrap_or_default()
            }),
            delete: Box::new(move |doc, ticks| {
                doc.delete_lyrics_events(track, ticks);
            }),
            insert: Box::new(move |doc, tick| doc.insert_lyrics_event(track, tick)),
            delete_label: "删除歌词事件",
            insert_label: "插入歌词事件",
            first_label: "新建歌词事件",
        },
    );
}

// ── Per-track Chord ──

pub fn apply_chord_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
    track: u16,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::Chord { track },
        EventOpsCtx {
            snapshot: Box::new(move |doc| {
                doc.data
                    .model
                    .tracks
                    .get(track as usize)
                    .map(|t| t.chord.iter().cloned().map(EventListItem::Chord).collect())
                    .unwrap_or_default()
            }),
            delete: Box::new(move |doc, ticks| {
                doc.delete_chord_events(track, ticks);
            }),
            insert: Box::new(move |doc, tick| doc.insert_chord_event(track, tick)),
            delete_label: "删除和弦事件",
            insert_label: "插入和弦事件",
            first_label: "新建和弦事件",
        },
    );
}

// ── Program Change ──

pub fn apply_pc_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
    track: u16,
) {
    dispatch_ops(
        ui,
        doc,
        state,
        salt,
        EventListTarget::ProgramChange { track },
        EventOpsCtx {
            snapshot: Box::new(move |doc| {
                doc.data
                    .model
                    .tracks
                    .get(track as usize)
                    .map(|t| {
                        t.program_change
                            .iter()
                            .cloned()
                            .map(EventListItem::ProgramChange)
                            .collect()
                    })
                    .unwrap_or_default()
            }),
            delete: Box::new(move |doc, ticks| {
                doc.delete_program_change_events(track, ticks);
            }),
            insert: Box::new(move |doc, tick| doc.insert_program_change_event(track, tick)),
            delete_label: "删除音色变更事件",
            insert_label: "插入音色变更事件",
            first_label: "新建音色变更事件",
        },
    );
}

// ── Notes ──

pub fn apply_notes_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
    track: u16,
) {
    let Some(req) = peek_edit_request(ui, salt) else {
        return;
    };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                // 用矩形选区覆盖每个选中 tick 的所有 key，调用 delete_selected
                let before = doc.capture_snapshot();
                doc.edit.selected.clear();
                for &tick in &state.selected_ticks {
                    // tick 到 tick+1 的窄矩形，覆盖全 key 范围，限定 track
                    doc.edit.selected.add_rect_track(
                        tick,
                        tick + 1,
                        0,
                        yinhe_types::MAX_KEY,
                        track,
                        track,
                    );
                }
                if let Some(action) = doc.delete_selected() {
                    doc.push_undo(action, "删除音符", before);
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            // 新建音符：C4(60)，四分音符，力度取该音轨最近修改值
            let ppq = doc.data.model.meta.ppq;
            let note = yinhe_core::NoteEvent {
                id: 0, // add_note 会分配 id
                start_tick: tick,
                end_tick: tick + ppq,
                key: 60,
                velocity: doc.edit.default_velocity(track),
            };
            let before = doc.capture_snapshot();
            if let Some(action) = doc.add_note(track, note) {
                doc.push_undo(action, "插入音符", before);
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let ppq = doc.data.model.meta.ppq;
            let note = yinhe_core::NoteEvent {
                id: 0,
                start_tick: tick,
                end_tick: tick + ppq,
                key: 60,
                velocity: doc.edit.default_velocity(track),
            };
            let before = doc.capture_snapshot();
            if let Some(action) = doc.add_note(track, note) {
                doc.push_undo(action, "新建音符", before);
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Automation ──

pub fn apply_automation_ops(
    ui: &mut egui::Ui,
    doc: &mut Document,
    state: &mut EventBrowserState,
    salt: &str,
    track: u16,
    target: &yinhe_types::AutomationTarget,
) {
    let Some(req) = peek_edit_request(ui, salt) else {
        return;
    };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                // 先查找 lane_idx（Tempo 固定为 0）
                let lane_idx = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
                    0usize
                } else {
                    doc.data
                        .model
                        .tracks
                        .get(track as usize)
                        .and_then(|t| t.automation_lanes.iter().position(|l| &l.target == target))
                        .unwrap_or(0)
                };
                // 逐个删除选中 tick 的 automation 事件，合并为 Composite undo
                let before = doc.capture_snapshot();
                let mut actions: Vec<UndoAction> = Vec::new();
                for &tick in &state.selected_ticks {
                    if let Some(action) =
                        doc.delete_automation_event(track as usize, lane_idx, target, tick)
                    {
                        actions.push(action);
                    }
                }
                if !actions.is_empty() {
                    doc.push_undo(UndoAction::Composite(actions), "删除自动化事件", before);
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            // 默认 value：Tempo=120（BPM），其他=0.0；shape=Step
            let value = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
                120.0
            } else {
                0.0
            };
            let event = yinhe_types::AutomationEvent {
                tick,
                value,
                shape: yinhe_types::SegmentShape::Step,
            };
            let before = doc.capture_snapshot();
            if let Some((_, _, action)) =
                doc.add_automation_event(track as usize, target.clone(), event)
            {
                doc.push_undo(action, "插入自动化事件", before);
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let value = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
                120.0
            } else {
                0.0
            };
            let event = yinhe_types::AutomationEvent {
                tick,
                value,
                shape: yinhe_types::SegmentShape::Step,
            };
            let before = doc.capture_snapshot();
            if let Some((_, _, action)) =
                doc.add_automation_event(track as usize, target.clone(), event)
            {
                doc.push_undo(action, "新建自动化事件", before);
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}
