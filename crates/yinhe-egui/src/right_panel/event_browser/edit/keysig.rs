//! 调号的 tick / root / scale 编辑 popup。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::ScaleType;

use super::super::bar_lookup::BarLookup;
use super::super::state::EditRequest;
use super::super::table::{
    peek_edit_request, peek_pos_edit_request, remove_edit_request, remove_pos_edit_request,
    update_edit_request, update_pos_edit_request,
};
use super::{ChoicePopupAction, PopupAction, PopupConfig, show_choice_popup, show_number_popup, show_tick_popup};

/// 处理调号的 tick / root / scale 编辑 popup。
///
/// 优先响应位置编辑请求（`(salt, "edit_pos")` key），再响应普通编辑请求。
pub fn apply_keysig_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    bar_lookup: &BarLookup,
) {
    if let Some(req) = peek_pos_edit_request(ui, salt) {
        match req {
            EditRequest::KeySigTick { tick } => show_keysig_tick_popup(ui, doc, salt, tick, Some(bar_lookup)),
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::KeySigTick { tick } => show_keysig_tick_popup(ui, doc, salt, tick, None),
        EditRequest::KeySigRoot { tick } => show_keysig_root_popup(ui, doc, salt, tick),
        EditRequest::KeySigScale { tick } => show_keysig_scale_popup(ui, doc, salt, tick),
        _ => {}
    }
}

fn show_keysig_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
    bar_lookup: Option<&BarLookup>,
) {
    let before = record_keysig_before(ui, doc, salt);
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_tick").as_ref(), tick, 0, bar_lookup);
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != tick {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                    doc.set_keysig_event(tick, new_tick, e.root, e.scale);
                }
                let req = EditRequest::KeySigTick { tick: new_tick };
                if bar_lookup.is_some() {
                    update_pos_edit_request(ui, salt, req);
                } else {
                    update_edit_request(ui, salt, req);
                }
            }
        }
        PopupAction::Closed => {
            finalize_keysig_undo(ui, doc, salt, before, t!("undo.edit_keysig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_keysig_root_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_keysig_before(ui, doc, salt);
    let root = doc.data.model.conductor.key_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.root).unwrap_or(0);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_keysig_root").as_ref(),
        initial: root as f64,
        range_min: 0.0,
        range_max: 11.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_root) => {
            let new_root = new_root as u8;
            if new_root != root {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                    doc.set_keysig_event(tick, tick, new_root, e.scale);
                }
            }
        }
        PopupAction::Closed => {
            finalize_keysig_undo(ui, doc, salt, before, t!("undo.edit_keysig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_keysig_scale_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_keysig_before(ui, doc, salt);
    let scale = doc.data.model.conductor.key_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.scale).unwrap_or(ScaleType::Major);
    let action = show_choice_popup(ui, salt, t!("event_browser.edit_keysig_scale").as_ref(),
        scale, ScaleType::ALL, |s| s.display_name().to_string());
    match action {
        ChoicePopupAction::Changed(new_scale) => {
            if new_scale != scale {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                    doc.set_keysig_event(tick, tick, e.root, new_scale);
                }
            }
        }
        ChoicePopupAction::Closed => {
            finalize_keysig_undo(ui, doc, salt, before, t!("undo.edit_keysig").as_ref());
        }
        ChoicePopupAction::None => {}
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
    remove_pos_edit_request(ui, salt);
}
