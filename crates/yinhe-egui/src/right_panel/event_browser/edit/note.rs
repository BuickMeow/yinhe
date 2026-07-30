//! 音符的 start_tick / end_tick / gate / key / velocity 编辑 popup。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::PencilNoteDrag;

use super::super::bar_lookup::BarLookup;
use super::super::state::{EditRequest, NoteRef};
use super::super::table::{
    peek_edit_request, peek_pos_edit_request, remove_edit_request, remove_pos_edit_request,
    update_edit_request, update_pos_edit_request,
};
use super::{PopupAction, PopupConfig, show_number_popup, show_tick_popup};

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
            EditRequest::NoteStartTick { note } => show_note_start_tick_popup(ui, doc, salt, note, Some(bar_lookup)),
            EditRequest::NoteEndTick { note } => show_note_end_tick_popup(ui, doc, salt, note, Some(bar_lookup)),
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::NoteStartTick { note } => show_note_start_tick_popup(ui, doc, salt, note, None),
        EditRequest::NoteEndTick { note } => show_note_end_tick_popup(ui, doc, salt, note, None),
        EditRequest::NoteGate { note } => show_note_gate_popup(ui, doc, salt, note),
        EditRequest::NoteKey { note } => show_note_key_popup(ui, doc, salt, note),
        EditRequest::NoteVelocity { note } => show_note_velocity_popup(ui, doc, salt, note),
        _ => {}
    }
}

fn show_note_start_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
    bar_lookup: Option<&BarLookup>,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_tick").as_ref(), note.start_tick, 0, bar_lookup);
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != note.start_tick {
                let delta_ticks = new_tick as i64 - note.start_tick as i64;
                doc.pencil_drag_note(&PencilNoteDrag::Move {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    delta_ticks,
                    delta_keys: 0,
                });
                // 保持 gate 不变：end_tick 跟着平移
                let new_end = note.end_tick as i64 + delta_ticks;
                let new_end = new_end.max(0) as u32;
                // 更新 NoteRef，下次寻址用新值
                let req = EditRequest::NoteStartTick {
                    note: NoteRef { start_tick: new_tick, end_tick: new_end, ..note },
                };
                if bar_lookup.is_some() {
                    update_pos_edit_request(ui, salt, req);
                } else {
                    update_edit_request(ui, salt, req);
                }
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
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
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_end_tick").as_ref(),
        note.end_tick, note.start_tick + 1, bar_lookup);
    match action {
        PopupAction::Changed(new_end) => {
            let new_end = new_end as u32;
            if new_end != note.end_tick {
                doc.pencil_drag_note(&PencilNoteDrag::ResizeRight {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    new_end_tick: new_end,
                });
                let req = EditRequest::NoteEndTick {
                    note: NoteRef { end_tick: new_end, ..note },
                };
                if bar_lookup.is_some() {
                    update_pos_edit_request(ui, salt, req);
                } else {
                    update_edit_request(ui, salt, req);
                }
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_gate_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let gate = note.end_tick.saturating_sub(note.start_tick);
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_gate").as_ref(),
        initial: gate as f64,
        range_min: 1.0,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_gate) => {
            let new_gate = new_gate as u32;
            let new_end = note.start_tick + new_gate;
            if new_end != note.end_tick {
                doc.pencil_drag_note(&PencilNoteDrag::ResizeRight {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    new_end_tick: new_end,
                });
                update_edit_request(ui, salt, EditRequest::NoteGate {
                    note: NoteRef { end_tick: new_end, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_key_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_key").as_ref(),
        initial: note.key as f64,
        range_min: 0.0,
        range_max: 127.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_key) => {
            let new_key = new_key as i32;
            if new_key != note.key as i32 {
                let delta_keys = new_key - note.key as i32;
                doc.pencil_drag_note(&PencilNoteDrag::Move {
                    track: note.track,
                    start_tick: note.start_tick,
                    key: note.key,
                    delta_ticks: 0,
                    delta_keys,
                });
                update_edit_request(ui, salt, EditRequest::NoteKey {
                    note: NoteRef { key: new_key as u8, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_note_velocity_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    note: NoteRef,
) {
    let before = record_note_before(ui, doc, salt, &note);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_velocity").as_ref(),
        initial: note.velocity as f64,
        range_min: 0.0,
        range_max: 127.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_vel) => {
            let new_vel = new_vel as u8;
            if new_vel != note.velocity {
                doc.set_note_velocity(note.track, note.start_tick, note.key, new_vel);
                update_edit_request(ui, salt, EditRequest::NoteVelocity {
                    note: NoteRef { velocity: new_vel, ..note },
                });
            }
        }
        PopupAction::Closed => {
            finalize_note_undo(ui, doc, salt, before, &note, t!("undo.edit_anchor_tick").as_ref());
        }
        PopupAction::None => {}
    }
}

/// popup 显示期间记录音符 before 快照（仅第一次记录）。
fn record_note_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
    note: &NoteRef,
) -> Option<(yinhe_types::Note, u8)> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let model = &doc.data.model;
        let k = note.key as usize;
        let before = model.notes.get(k)
            .and_then(|bucket| bucket.iter().find(|n| n.id == note.id))
            .copied();
        if let Some(before) = before {
            ui.memory_mut(|m| {
                m.data.insert_temp(before_id, (before, note.key));
                m.data.insert_temp(recorded_id, true);
            });
            Some((before, note.key))
        } else {
            None
        }
    } else {
        ui.memory(|m| m.data.get_temp::<(yinhe_types::Note, u8)>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_note_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<(yinhe_types::Note, u8)>,
    note: &NoteRef,
    label: &str,
) {
    use yinhe_editor_core::history::{NoteDelta, UndoAction, UndoEntry};

    if let Some((before_note, before_key)) = before {
        let model = &doc.data.model;
        let k = note.key as usize;
        if let Some(after_note) = model.notes.get(k)
            .and_then(|bucket| bucket.iter().find(|n| n.id == note.id))
            .copied()
        {
            // Note 未实现 PartialEq，按字段比较（id 不变，只比可变字段）
            let changed = before_note.start_tick != after_note.start_tick
                || before_note.end_tick != after_note.end_tick
                || before_note.velocity != after_note.velocity
                || before_note.track != after_note.track
                || before_key != note.key;
            if changed {
                doc.history.push(UndoEntry {
                    action: UndoAction::Notes(NoteDelta {
                        before: vec![(before_note, before_key)],
                        after: vec![(after_note, note.key)],
                    }),
                    label: label.to_string(),
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<(yinhe_types::Note, u8)>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
    remove_pos_edit_request(ui, salt);
}
