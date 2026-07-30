//! Program Change 的 tick / program 编辑 popup。
//!
//! popup 打开期间不修改 Document，pending 写到 egui memory。
//! 关闭时（Closed）一次性 apply + push undo；取消（Cancelled）仅清理。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{EventListItem, EventListTarget};

use super::super::bar_lookup::BarLookup;
use super::super::state::EditRequest;
use super::super::table::{peek_edit_request, peek_pos_edit_request, remove_pos_edit_request};
use super::{PopupAction, PopupConfig, cleanup_edit_request, push_event_list_undo, show_number_popup, show_tick_popup};

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

/// 某 track 的 program_change 事件列表快照（用于 undo before/after）。
fn pc_snapshot(doc: &Document, track: u16) -> Vec<EventListItem> {
    doc.data.model.tracks.get(track as usize)
        .map(|t| t.program_change.iter().cloned().map(EventListItem::ProgramChange).collect())
        .unwrap_or_default()
}

fn show_pc_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    track: u16,
    tick: u32,
    bar_lookup: Option<&BarLookup>,
) {
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_tick").as_ref(), tick, 0, bar_lookup);
    match action {
        PopupAction::Closed(new_tick_f) => {
            let new_tick = new_tick_f as u32;
            // old_tick 来自 EditRequest（tick），new_tick 来自 pending；program 保持不变
            let program = doc.data.model.tracks.get(track as usize)
                .and_then(|t| t.program_change.iter().find(|e| e.tick == tick))
                .map(|e| e.program);
            if let Some(program) = program {
                let before = pc_snapshot(doc, track);
                doc.set_program_change_event(track, tick, new_tick, program);
                let after = pc_snapshot(doc, track);
                push_event_list_undo(doc, EventListTarget::ProgramChange { track }, before, after, t!("undo.edit_pc").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
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
        PopupAction::Closed(new_program_f) => {
            let new_program = new_program_f as u8;
            // tick 来自 EditRequest（tick），新 program 来自 pending
            if doc.data.model.tracks.get(track as usize)
                .and_then(|t| t.program_change.iter().find(|e| e.tick == tick))
                .is_some()
            {
                let before = pc_snapshot(doc, track);
                doc.set_program_change_event(track, tick, tick, new_program);
                let after = pc_snapshot(doc, track);
                push_event_list_undo(doc, EventListTarget::ProgramChange { track }, before, after, t!("undo.edit_pc").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}
