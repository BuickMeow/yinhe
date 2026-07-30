//! 拍号的 tick / numerator / denominator 编辑 popup。
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

/// 处理拍号的 tick / numerator / denominator 编辑 popup。
///
/// 优先响应位置编辑请求（`(salt, "edit_pos")` key），再响应普通编辑请求。
pub fn apply_timesig_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    bar_lookup: &BarLookup,
) {
    if let Some(req) = peek_pos_edit_request(ui, salt) {
        match req {
            EditRequest::TimeSigTick { tick } => show_timesig_tick_popup(ui, doc, salt, tick, Some(bar_lookup)),
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::TimeSigTick { tick } => show_timesig_tick_popup(ui, doc, salt, tick, None),
        EditRequest::TimeSigNumerator { tick } => show_timesig_num_popup(ui, doc, salt, tick),
        EditRequest::TimeSigDenominator { tick } => show_timesig_den_popup(ui, doc, salt, tick),
        _ => {}
    }
}

/// 拍号事件列表的当前快照（用于 undo before/after）。
fn timesig_snapshot(doc: &Document) -> Vec<EventListItem> {
    doc.data.model.conductor.time_sig.iter().cloned().map(EventListItem::TimeSig).collect()
}

fn show_timesig_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
    bar_lookup: Option<&BarLookup>,
) {
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_tick").as_ref(), tick, 0, bar_lookup);
    match action {
        PopupAction::Closed(new_tick_f) => {
            let new_tick = new_tick_f as u32;
            if let Some(e) = doc.data.model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                let before = timesig_snapshot(doc);
                doc.set_time_sig_event(tick, new_tick, e.numerator, e.denominator);
                let after = timesig_snapshot(doc);
                push_event_list_undo(doc, EventListTarget::TimeSig, before, after, t!("undo.edit_timesig").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_timesig_num_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
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
        PopupAction::Closed(new_num_f) => {
            let new_num = new_num_f as u8;
            if let Some(e) = doc.data.model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                let before = timesig_snapshot(doc);
                doc.set_time_sig_event(tick, tick, new_num, e.denominator);
                let after = timesig_snapshot(doc);
                push_event_list_undo(doc, EventListTarget::TimeSig, before, after, t!("undo.edit_timesig").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_timesig_den_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
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
        PopupAction::Closed(new_den_f) => {
            let new_den = new_den_f as u8;
            if let Some(e) = doc.data.model.conductor.time_sig.iter().find(|e| e.tick == tick) {
                let before = timesig_snapshot(doc);
                doc.set_time_sig_event(tick, tick, e.numerator, new_den);
                let after = timesig_snapshot(doc);
                push_event_list_undo(doc, EventListTarget::TimeSig, before, after, t!("undo.edit_timesig").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}
