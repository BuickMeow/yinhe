//! 拍号的 tick / numerator / denominator 编辑 popup。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;

use super::super::state::EditRequest;
use super::super::table::{peek_edit_request, remove_edit_request, update_edit_request};
use super::{PopupAction, PopupConfig, show_number_popup};

/// 处理拍号的 tick / numerator / denominator 编辑 popup。
pub fn apply_timesig_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
) {
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::TimeSigTick { tick } => show_timesig_tick_popup(ui, doc, salt, tick),
        EditRequest::TimeSigNumerator { tick } => show_timesig_num_popup(ui, doc, salt, tick),
        EditRequest::TimeSigDenominator { tick } => show_timesig_den_popup(ui, doc, salt, tick),
        _ => {}
    }
}

fn show_timesig_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_timesig_before(ui, doc, salt);
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
                if let Some(e) = model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                    doc.set_time_sig_event(tick, new_tick, e.numerator, e.denominator);
                }
                update_edit_request(ui, salt, EditRequest::TimeSigTick { tick: new_tick });
            }
        }
        PopupAction::Closed => {
            finalize_timesig_undo(ui, doc, salt, before, t!("undo.edit_timesig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_timesig_num_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_timesig_before(ui, doc, salt);
    let numerator = doc.data.model.conductor.time_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.numerator).unwrap_or(4);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_numerator").as_ref(),
        initial: numerator as f64,
        range_min: 1.0,
        range_max: 255.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_num) => {
            let new_num = new_num as u8;
            if new_num != numerator {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                    doc.set_time_sig_event(tick, tick, new_num, e.denominator);
                }
            }
        }
        PopupAction::Closed => {
            finalize_timesig_undo(ui, doc, salt, before, t!("undo.edit_timesig").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_timesig_den_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let before = record_timesig_before(ui, doc, salt);
    let denominator = doc.data.model.conductor.time_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.denominator).unwrap_or(2);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_denominator").as_ref(),
        initial: denominator as f64,
        range_min: 0.0,
        range_max: 8.0,  // 2^8 = 256
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_den) => {
            let new_den = new_den as u8;
            if new_den != denominator {
                let model = &doc.data.model;
                if let Some(e) = model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                    doc.set_time_sig_event(tick, tick, e.numerator, new_den);
                }
            }
        }
        PopupAction::Closed => {
            finalize_timesig_undo(ui, doc, salt, before, t!("undo.edit_timesig").as_ref());
        }
        PopupAction::None => {}
    }
}

/// popup 显示期间记录 time_sig before 快照（仅第一次记录）。
fn record_timesig_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
) -> Option<Vec<yinhe_types::TimeSigEvent>> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let before = doc.data.model.conductor.time_sig.clone();
        ui.memory_mut(|m| {
            m.data.insert_temp(before_id, before.clone());
            m.data.insert_temp(recorded_id, true);
        });
        Some(before)
    } else {
        ui.memory(|m| m.data.get_temp::<Vec<yinhe_types::TimeSigEvent>>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_timesig_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<Vec<yinhe_types::TimeSigEvent>>,
    label: &str,
) {
    use yinhe_editor_core::history::{UndoAction, UndoEntry};
    if let Some(before) = before {
        let after = doc.data.model.conductor.time_sig.clone();
        if before != after {
            doc.history.push(UndoEntry {
                action: UndoAction::TimeSig { old: before, new: after },
                label: label.to_string(),
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<Vec<yinhe_types::TimeSigEvent>>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
}
