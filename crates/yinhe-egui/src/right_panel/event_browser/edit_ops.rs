//! Event browser 的删除/插入操作（多选删除、右键插入、空表加号）。
//!
//! 与 `edit/` 子模块（单字段编辑 popup）不同，本模块处理整行删除和插入。
//! 每个 `apply_*_ops` 函数检查 `EditRequest::DeleteSelected` / `InsertAbove` /
//! `InsertBelow` / `InsertFirst`，执行对应 Document 方法并 push undo。

use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{UndoAction, UndoEntry};

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

// ── TimeSig ──

pub fn apply_timesig_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_time_sig_events(&state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::TimeSig { old: before, new: after },
                        label: "删除拍号事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.conductor.time_sig.clone();
            doc.insert_time_sig_event(tick);
            let after = doc.data.model.conductor.time_sig.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::TimeSig { old: before, new: after },
                    label: "插入拍号事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.conductor.time_sig.clone();
            doc.insert_time_sig_event(tick);
            let after = doc.data.model.conductor.time_sig.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::TimeSig { old: before, new: after },
                    label: "新建拍号事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── KeySig ──

pub fn apply_keysig_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_key_sig_events(&state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::KeySig { old: before, new: after },
                        label: "删除调号事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.conductor.key_sig.clone();
            doc.insert_key_sig_event(tick);
            let after = doc.data.model.conductor.key_sig.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::KeySig { old: before, new: after },
                    label: "插入调号事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.conductor.key_sig.clone();
            doc.insert_key_sig_event(tick);
            let after = doc.data.model.conductor.key_sig.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::KeySig { old: before, new: after },
                    label: "新建调号事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Marker ──

pub fn apply_marker_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_marker_events(&state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::Marker { old: before, new: after },
                        label: "删除标记事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.conductor.markers.clone();
            doc.insert_marker_event(tick);
            let after = doc.data.model.conductor.markers.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Marker { old: before, new: after },
                    label: "插入标记事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.conductor.markers.clone();
            doc.insert_marker_event(tick);
            let after = doc.data.model.conductor.markers.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Marker { old: before, new: after },
                    label: "新建标记事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Conductor Lyrics ──

pub fn apply_conductor_lyrics_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_conductor_lyrics_events(&state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::ConductorLyrics { old: before, new: after },
                        label: "删除歌词事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.conductor.lyrics.clone();
            doc.insert_conductor_lyrics_event(tick);
            let after = doc.data.model.conductor.lyrics.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::ConductorLyrics { old: before, new: after },
                    label: "插入歌词事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.conductor.lyrics.clone();
            doc.insert_conductor_lyrics_event(tick);
            let after = doc.data.model.conductor.lyrics.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::ConductorLyrics { old: before, new: after },
                    label: "新建歌词事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Conductor Chord ──

pub fn apply_conductor_chord_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_conductor_chord_events(&state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::ConductorChord { old: before, new: after },
                        label: "删除和弦事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.conductor.chord.clone();
            doc.insert_conductor_chord_event(tick);
            let after = doc.data.model.conductor.chord.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::ConductorChord { old: before, new: after },
                    label: "插入和弦事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.conductor.chord.clone();
            doc.insert_conductor_chord_event(tick);
            let after = doc.data.model.conductor.chord.clone();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::ConductorChord { old: before, new: after },
                    label: "新建和弦事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Per-track Lyrics ──

pub fn apply_lyrics_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str, track: u16) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_lyrics_events(track, &state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::Lyrics { track, old: before, new: after },
                        label: "删除歌词事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.tracks.get(track as usize).map(|t| t.lyrics.clone()).unwrap_or_default();
            doc.insert_lyrics_event(track, tick);
            let after = doc.data.model.tracks.get(track as usize).map(|t| t.lyrics.clone()).unwrap_or_default();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Lyrics { track, old: before, new: after },
                    label: "插入歌词事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.tracks.get(track as usize).map(|t| t.lyrics.clone()).unwrap_or_default();
            doc.insert_lyrics_event(track, tick);
            let after = doc.data.model.tracks.get(track as usize).map(|t| t.lyrics.clone()).unwrap_or_default();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Lyrics { track, old: before, new: after },
                    label: "新建歌词事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Per-track Chord ──

pub fn apply_chord_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str, track: u16) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_chord_events(track, &state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::Chord { track, old: before, new: after },
                        label: "删除和弦事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.tracks.get(track as usize).map(|t| t.chord.clone()).unwrap_or_default();
            doc.insert_chord_event(track, tick);
            let after = doc.data.model.tracks.get(track as usize).map(|t| t.chord.clone()).unwrap_or_default();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Chord { track, old: before, new: after },
                    label: "插入和弦事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.tracks.get(track as usize).map(|t| t.chord.clone()).unwrap_or_default();
            doc.insert_chord_event(track, tick);
            let after = doc.data.model.tracks.get(track as usize).map(|t| t.chord.clone()).unwrap_or_default();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::Chord { track, old: before, new: after },
                    label: "新建和弦事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Program Change ──

pub fn apply_pc_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str, track: u16) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                let (before, after) = doc.delete_program_change_events(track, &state.selected_ticks);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::ProgramChange { track, old: before, new: after },
                        label: "删除音色变更事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            let before = doc.data.model.tracks.get(track as usize).map(|t| t.program_change.clone()).unwrap_or_default();
            doc.insert_program_change_event(track, tick);
            let after = doc.data.model.tracks.get(track as usize).map(|t| t.program_change.clone()).unwrap_or_default();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::ProgramChange { track, old: before, new: after },
                    label: "插入音色变更事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let before = doc.data.model.tracks.get(track as usize).map(|t| t.program_change.clone()).unwrap_or_default();
            doc.insert_program_change_event(track, tick);
            let after = doc.data.model.tracks.get(track as usize).map(|t| t.program_change.clone()).unwrap_or_default();
            if before != after {
                doc.history.push(UndoEntry {
                    action: UndoAction::ProgramChange { track, old: before, new: after },
                    label: "新建音色变更事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}

// ── Notes ──

pub fn apply_notes_ops(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState, salt: &str, track: u16) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                // 用矩形选区覆盖每个选中 tick 的所有 key，调用 delete_selected
                doc.edit.selected.clear();
                for &tick in &state.selected_ticks {
                    // tick 到 tick+1 的窄矩形，覆盖全 key 范围，限定 track
                    doc.edit.selected.add_rect_track(tick, tick + 1, 0, 127, track, track);
                }
                if let Some(action) = doc.delete_selected() {
                    doc.history.push(UndoEntry {
                        action,
                        label: "删除音符".to_string(),
                        selected: Default::default(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            // 新建音符：C4(60)，四分音符，力度 100
            let ppq = doc.data.model.meta.ppq;
            let note = yinhe_core::NoteEvent {
                id: 0, // add_note 会分配 id
                start_tick: tick,
                end_tick: tick + ppq,
                key: 60,
                velocity: 100,
            };
            if let Some(action) = doc.add_note(track, note) {
                doc.history.push(UndoEntry {
                    action,
                    label: "插入音符".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
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
                velocity: 100,
            };
            if let Some(action) = doc.add_note(track, note) {
                doc.history.push(UndoEntry {
                    action,
                    label: "新建音符".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
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
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::DeleteSelected => {
            if !state.selected_ticks.is_empty() {
                // 先查找 lane_idx（Tempo 固定为 0）
                let lane_idx = if matches!(target, yinhe_types::AutomationTarget::Tempo) {
                    0usize
                } else {
                    doc.data.model.tracks.get(track as usize)
                        .and_then(|t| t.automation_lanes.iter().position(|l| &l.target == target))
                        .unwrap_or(0)
                };
                // 逐个删除选中 tick 的 automation 事件，合并为 Composite undo
                let mut actions: Vec<UndoAction> = Vec::new();
                for &tick in &state.selected_ticks {
                    if let Some(action) = doc.delete_automation_event(track as usize, lane_idx, target, tick) {
                        actions.push(action);
                    }
                }
                if !actions.is_empty() {
                    doc.history.push(UndoEntry {
                        action: UndoAction::Composite(actions),
                        label: "删除自动化事件".to_string(),
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
                cleanup(state, ui, salt);
            } else {
                remove_edit_request(ui, salt);
            }
        }
        EditRequest::InsertAbove { tick } | EditRequest::InsertBelow { tick } => {
            // 默认 value：Tempo=120（BPM），其他=0.0；shape=Step
            let value = if matches!(target, yinhe_types::AutomationTarget::Tempo) { 120.0 } else { 0.0 };
            let event = yinhe_types::AutomationEvent {
                tick,
                value,
                shape: yinhe_types::SegmentShape::Step,
            };
            if let Some((_, _, action)) = doc.add_automation_event(track as usize, target.clone(), event) {
                doc.history.push(UndoEntry {
                    action,
                    label: "插入自动化事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        EditRequest::InsertFirst => {
            let tick = current_tick(doc);
            let value = if matches!(target, yinhe_types::AutomationTarget::Tempo) { 120.0 } else { 0.0 };
            let event = yinhe_types::AutomationEvent {
                tick,
                value,
                shape: yinhe_types::SegmentShape::Step,
            };
            if let Some((_, _, action)) = doc.add_automation_event(track as usize, target.clone(), event) {
                doc.history.push(UndoEntry {
                    action,
                    label: "新建自动化事件".to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            remove_edit_request(ui, salt);
        }
        _ => {}
    }
}
