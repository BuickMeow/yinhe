//! 调号的 tick / sf / mi 编辑 popup。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;

use super::super::state::EditRequest;
use super::super::table::{peek_edit_request, remove_edit_request, update_edit_request};
use super::{PopupAction, PopupConfig, show_number_popup};

/// 处理调号的 tick / sf / mi 编辑 popup。
pub fn apply_keysig_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::KeySigTick { tick } => show_keysig_tick_popup(ui, doc, salt, tick),
        EditRequest::KeySigSf { tick } => show_keysig_sf_popup(ui, doc, salt, tick),
        EditRequest::KeySigMi { tick } => show_keysig_mi_popup(ui, doc, salt, tick),
        _ => {}
    }
}

fn show_keysig_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_keysig_before(ui, doc, salt);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_tick").as_ref(),
        initial: tick as f64,
        range_min: 0.0,
        range_max: u32::MAX as f64,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != tick {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                    doc.set_keysig_event(tick, new_tick, e.sf, e.mi);
                }
                update_edit_request(ui, salt, EditRequest::KeySigTick { tick: new_tick });
            }
        }
        PopupAction::Closed => {
            finalize_keysig_undo(ui, doc, salt, before, t!("undo.edit_keysig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_keysig_sf_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_keysig_before(ui, doc, salt);
    let sf = doc.data.model.conductor.key_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.sf).unwrap_or(0);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_keysig_sf").as_ref(),
        initial: sf as f64,
        range_min: -7.0,
        range_max: 7.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_sf) => {
            let new_sf = new_sf as i8;
            if new_sf != sf {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                    doc.set_keysig_event(tick, tick, new_sf, e.mi);
                }
            }
        }
        PopupAction::Closed => {
            finalize_keysig_undo(ui, doc, salt, before, t!("undo.edit_keysig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_keysig_mi_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_keysig_before(ui, doc, salt);
    let mi = doc.data.model.conductor.key_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.mi).unwrap_or(0);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_keysig_mi").as_ref(),
        initial: mi as f64,
        range_min: 0.0,
        range_max: 1.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_mi) => {
            let new_mi = new_mi as u8;
            if new_mi != mi {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                    doc.set_keysig_event(tick, tick, e.sf, new_mi);
                }
            }
        }
        PopupAction::Closed => {
            finalize_keysig_undo(ui, doc, salt, before, t!("undo.edit_keysig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn record_keysig_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
) -> Option<Vec<yinhe_types::KeySigEvent>> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let before = doc.data.model.conductor.key_sig.clone();
        ui.memory_mut(|m| {
            m.data.insert_temp(before_id, before.clone());
            m.data.insert_temp(recorded_id, true);
        });
        Some(before)
    } else {
        ui.memory(|m| m.data.get_temp::<Vec<yinhe_types::KeySigEvent>>(before_id))
    }
}

fn finalize_keysig_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<Vec<yinhe_types::KeySigEvent>>,
    label: &str,
) {
    use yinhe_editor_core::history::{UndoAction, UndoEntry};
    if let Some(before) = before {
        let after = doc.data.model.conductor.key_sig.clone();
        if before != after {
            doc.history.push(UndoEntry {
                action: UndoAction::KeySig { old: before, new: after },
                label: label.to_string(),
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<Vec<yinhe_types::KeySigEvent>>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
}
