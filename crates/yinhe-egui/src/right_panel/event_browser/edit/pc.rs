//! Program Change 的 tick / program 编辑 popup。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;

use super::super::bar_lookup::BarLookup;
use super::super::state::EditRequest;
use super::super::table::{
    peek_edit_request, peek_pos_edit_request, remove_edit_request, remove_pos_edit_request,
    update_edit_request, update_pos_edit_request,
};
use super::{PopupAction, PopupConfig, show_number_popup, show_tick_popup};

/// 处理 Program Change 的 tick / program 编辑 popup。
///
/// 优先响应位置编辑请求（`(salt, "edit_pos")` key），再响应普通编辑请求。
pub fn apply_pc_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    track: u16,
    bar_lookup: &BarLookup,
) {
    if let Some(req) = peek_pos_edit_request(ui, salt) {
        match req {
            EditRequest::PcTick { tick } => show_pc_tick_popup(ui, doc, salt, track, tick, Some(bar_lookup)),
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::PcTick { tick } => show_pc_tick_popup(ui, doc, salt, track, tick, None),
        EditRequest::PcProgram { tick } => show_pc_program_popup(ui, doc, salt, track, tick),
        _ => {}
    }
}

fn show_pc_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    track: u16,
    tick: u32,
    bar_lookup: Option<&BarLookup>,
) {
    let before = record_pc_before(ui, doc, salt, track);
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_tick").as_ref(), tick, 0, bar_lookup);
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != tick {
                let program = doc.data.model.tracks.get(track as usize)
                    .and_then(|t| t.program_change.iter().find(|e| e.tick == tick))
                    .map(|e| e.program);
                if let Some(program) = program {
                    doc.set_program_change_event(track, tick, new_tick, program);
                }
                let req = EditRequest::PcTick { tick: new_tick };
                if bar_lookup.is_some() {
                    update_pos_edit_request(ui, salt, req);
                } else {
                    update_edit_request(ui, salt, req);
                }
            }
        }
        PopupAction::Closed => {
            finalize_pc_undo(ui, doc, salt, track, before, t!("undo.edit_pc").as_ref());
        }
        PopupAction::None => {}
    }
}

fn show_pc_program_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    track: u16,
    tick: u32,
) {
    let before = record_pc_before(ui, doc, salt, track);
    let program = doc.data.model.tracks.get(track as usize)
        .and_then(|t| t.program_change.iter().find(|e| e.tick == tick))
        .map(|e| e.program)
        .unwrap_or(0);
    let action = show_number_popup(ui, PopupConfig {
        salt,
        title: t!("event_browser.edit_program").as_ref(),
        initial: program as f64,
        range_min: 0.0,
        range_max: 127.0,
        speed: 1.0,
        fixed_decimals: None,
    });
    match action {
        PopupAction::Changed(new_program) => {
            let new_program = new_program as u8;
            if new_program != program {
                doc.set_program_change_event(track, tick, tick, new_program);
            }
        }
        PopupAction::Closed => {
            finalize_pc_undo(ui, doc, salt, track, before, t!("undo.edit_pc").as_ref());
        }
        PopupAction::None => {}
    }
}

/// popup 显示期间记录 program_change before 快照（仅第一次记录）。
fn record_pc_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
    track: u16,
) -> Option<Vec<yinhe_types::PcEvent>> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let before = doc.data.model.tracks.get(track as usize)
            .map(|t| t.program_change.clone())
            .unwrap_or_default();
        ui.memory_mut(|m| {
            m.data.insert_temp(before_id, before.clone());
            m.data.insert_temp(recorded_id, true);
        });
        Some(before)
    } else {
        ui.memory(|m| m.data.get_temp::<Vec<yinhe_types::PcEvent>>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_pc_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    track: u16,
    before: Option<Vec<yinhe_types::PcEvent>>,
    label: &str,
) {
    use yinhe_editor_core::history::{UndoAction, UndoEntry};
    if let Some(before) = before {
        let after = doc.data.model.tracks.get(track as usize)
            .map(|t| t.program_change.clone())
            .unwrap_or_default();
        if before != after {
            doc.history.push(UndoEntry {
                action: UndoAction::ProgramChange { track, old: before, new: after },
                label: label.to_string(),
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<Vec<yinhe_types::PcEvent>>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
    remove_pos_edit_request(ui, salt);
}
