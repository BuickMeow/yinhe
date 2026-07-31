//! 音符的 start_tick / end_tick / gate / key / velocity 编辑 popup。
//!
//! popup 打开期间不修改 Document，pending 写到 egui memory。
//! 关闭时（Closed）一次性 apply + push NoteDelta undo；取消（Cancelled）仅清理。
//! 注意：音符用 `UndoAction::Notes(NoteDelta)`，before/after 是 `Vec<(Note, u8)>`，
//! 不是 EventListItem（与 keysig/timesig/text/pc 的事件列表 undo 不同）。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{EditSnapshot, NoteDelta, UndoAction};
use yinhe_types::{Note, PencilNoteDrag};

use super::super::bar_lookup::BarLookup;
use super::super::state::{EditRequest, NoteRef};
use super::super::table::{peek_edit_request, peek_pos_edit_request, remove_pos_edit_request};
use super::{PopupAction, PopupConfig, cleanup_edit_request, show_number_popup, show_tick_popup};

/// 处理音符的 start_tick / end_tick / gate / key / velocity 编辑 popup。
///
/// 优先响应位置编辑请求（`(salt, "edit_pos")` key），再响应普通编辑请求。
pub fn apply_note_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    bar_lookup: &BarLookup,
) {
    if let Some(req) = peek_pos_edit_request(ui, salt) {
        match req {
            EditRequest::NoteStartTick { note } => {
                show_note_start_tick_popup(ui, doc, salt, note, Some(bar_lookup))
            }
            EditRequest::NoteEndTick { note } => {
                show_note_end_tick_popup(ui, doc, salt, note, Some(bar_lookup))
            }
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else {
        return;
    };
    match req {
        EditRequest::NoteStartTick { note } => {
            show_note_start_tick_popup(ui, doc, salt, note, None)
        }
        EditRequest::NoteEndTick { note } => show_note_end_tick_popup(ui, doc, salt, note, None),
        EditRequest::NoteGate { note } => show_note_gate_popup(ui, doc, salt, note),
        EditRequest::NoteKey { note } => show_note_key_popup(ui, doc, salt, note),
        EditRequest::NoteVelocity { note } => show_note_velocity_popup(ui, doc, salt, note),
        _ => {}
    }
}

/// 在 key 桶中按 id 查找音符。
fn find_note(model: &yinhe_core::YinModel, id: u32, key: u8) -> Option<Note> {
    model
        .notes
        .get(key as usize)
        .and_then(|bucket| bucket.iter().find(|n| n.id == id))
        .copied()
}

/// 比较音符可变字段（id 不变，不比）。Note 未实现 PartialEq，需手动比较。
fn note_fields_changed(a: &Note, b: &Note) -> bool {
    a.start_tick != b.start_tick
        || a.end_tick != b.end_tick
        || a.velocity != b.velocity
        || a.track != b.track
}

/// 取 before/after 快照并 push NoteDelta undo（仅当字段变化时）。
/// `snapshot` 必须是编辑**前**捕获的界面状态快照。
fn push_note_undo(
    doc: &mut Document,
    before: Option<(Note, u8)>,
    after: Option<(Note, u8)>,
    label: &str,
    snapshot: EditSnapshot,
) {
    let Some((b_note, b_key)) = before else {
        return;
    };
    let Some((a_note, a_key)) = after else {
        return;
    };
    if note_fields_changed(&b_note, &a_note) || b_key != a_key {
        doc.push_undo(
            UndoAction::Notes(NoteDelta {
                before: vec![(b_note, b_key)],
                after: vec![(a_note, a_key)],
            }),
            label,
            snapshot,
        );
    }
}

fn show_note_start_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
    bar_lookup: Option<&BarLookup>,
) {
    let action = show_tick_popup(
        ui,
        salt,
        t!("event_browser.edit_tick").as_ref(),
        note.start_tick,
        0,
        bar_lookup,
    );
    match action {
        PopupAction::Closed(new_tick_f) => {
            let new_tick = new_tick_f as u32;
            let snapshot = doc.capture_snapshot();
            let before = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            let delta_ticks = new_tick as i64 - note.start_tick as i64;
            doc.pencil_drag_note(&PencilNoteDrag::Move {
                track: note.track,
                start_tick: note.start_tick,
                key: note.key,
                delta_ticks,
                delta_keys: 0,
            });
            // start_tick 改了，key 不变，仍在原 key 桶
            let after = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            push_note_undo(
                doc,
                before,
                after,
                t!("undo.edit_anchor_tick").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_note_end_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
    bar_lookup: Option<&BarLookup>,
) {
    let action = show_tick_popup(
        ui,
        salt,
        t!("event_browser.edit_end_tick").as_ref(),
        note.end_tick,
        note.start_tick + 1,
        bar_lookup,
    );
    match action {
        PopupAction::Closed(new_end_f) => {
            let new_end = new_end_f as u32;
            let snapshot = doc.capture_snapshot();
            let before = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            doc.pencil_drag_note(&PencilNoteDrag::ResizeRight {
                track: note.track,
                start_tick: note.start_tick,
                key: note.key,
                new_end_tick: new_end,
            });
            let after = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            push_note_undo(
                doc,
                before,
                after,
                t!("undo.edit_anchor_tick").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_note_gate_popup(ui: &mut egui::Ui, doc: &mut Document, salt: &str, note: NoteRef) {
    let gate = note.end_tick.saturating_sub(note.start_tick);
    let action = show_number_popup(
        ui,
        PopupConfig {
            salt,
            title: t!("event_browser.edit_gate").as_ref(),
            initial: gate as f64,
            range_min: 1.0,
            range_max: u32::MAX as f64,
            speed: 1.0,
            fixed_decimals: None,
        },
    );
    match action {
        PopupAction::Closed(new_gate_f) => {
            let new_gate = new_gate_f as u32;
            let new_end = note.start_tick + new_gate;
            let snapshot = doc.capture_snapshot();
            let before = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            doc.pencil_drag_note(&PencilNoteDrag::ResizeRight {
                track: note.track,
                start_tick: note.start_tick,
                key: note.key,
                new_end_tick: new_end,
            });
            let after = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            push_note_undo(
                doc,
                before,
                after,
                t!("undo.edit_anchor_tick").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_note_key_popup(ui: &mut egui::Ui, doc: &mut Document, salt: &str, note: NoteRef) {
    let action = show_number_popup(
        ui,
        PopupConfig {
            salt,
            title: t!("event_browser.edit_key").as_ref(),
            initial: note.key as f64,
            range_min: 0.0,
            range_max: 127.0,
            speed: 1.0,
            fixed_decimals: None,
        },
    );
    match action {
        PopupAction::Closed(new_key_f) => {
            let new_key = new_key_f as i32;
            let snapshot = doc.capture_snapshot();
            let before = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            let delta_keys = new_key - note.key as i32;
            doc.pencil_drag_note(&PencilNoteDrag::Move {
                track: note.track,
                start_tick: note.start_tick,
                key: note.key,
                delta_ticks: 0,
                delta_keys,
            });
            // key 改了，需在新 key 桶中查找 after
            let after =
                find_note(&doc.data.model, note.id, new_key as u8).map(|n| (n, new_key as u8));
            push_note_undo(
                doc,
                before,
                after,
                t!("undo.edit_anchor_tick").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_note_velocity_popup(ui: &mut egui::Ui, doc: &mut Document, salt: &str, note: NoteRef) {
    let action = show_number_popup(
        ui,
        PopupConfig {
            salt,
            title: t!("event_browser.edit_velocity").as_ref(),
            initial: note.velocity as f64,
            range_min: 0.0,
            range_max: 127.0,
            speed: 1.0,
            fixed_decimals: None,
        },
    );
    match action {
        PopupAction::Closed(new_vel_f) => {
            let new_vel = new_vel_f as u8;
            let snapshot = doc.capture_snapshot();
            let before = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            doc.set_note_velocity(note.track, note.start_tick, note.key, new_vel);
            let after = find_note(&doc.data.model, note.id, note.key).map(|n| (n, note.key));
            push_note_undo(
                doc,
                before,
                after,
                t!("undo.edit_anchor_tick").as_ref(),
                snapshot,
            );
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}
