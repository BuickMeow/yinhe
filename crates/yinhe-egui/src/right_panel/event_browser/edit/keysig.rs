//! 调号的 tick / root / scale 编辑 popup。
//!
//! popup 打开期间不修改 Document，pending 写到 egui memory。
//! 关闭时（Closed）一次性 apply + push undo；取消（Cancelled）仅清理。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{EventListItem, EventListTarget};
use yinhe_types::ScaleType;

use super::super::bar_lookup::BarLookup;
use super::super::state::EditRequest;
use super::super::table::{peek_edit_request, peek_pos_edit_request, remove_pos_edit_request};
use super::{
    ChoicePopupAction, PopupAction, PopupConfig, cleanup_edit_request, push_event_list_undo,
    show_choice_popup, show_number_popup, show_tick_popup,
};

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

/// 调号事件列表的当前快照（用于 undo before/after）。
fn keysig_snapshot(doc: &Document) -> Vec<EventListItem> {
    doc.data.model.conductor.key_sig.iter().cloned().map(EventListItem::KeySig).collect()
}

fn show_keysig_tick_popup(
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
            // old_tick 来自 EditRequest（tick），new_tick 来自 pending
            if let Some(e) = doc.data.model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                let before = keysig_snapshot(doc);
                doc.set_keysig_event(tick, new_tick, e.root, e.scale);
                let after = keysig_snapshot(doc);
                push_event_list_undo(doc, EventListTarget::KeySig, before, after, t!("undo.edit_keysig").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_keysig_root_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
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
        PopupAction::Closed(new_root_f) => {
            let new_root = new_root_f as u8;
            // root 来自 EditRequest（tick），新 root 来自 pending
            if let Some(e) = doc.data.model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                let before = keysig_snapshot(doc);
                doc.set_keysig_event(tick, tick, new_root, e.scale);
                let after = keysig_snapshot(doc);
                push_event_list_undo(doc, EventListTarget::KeySig, before, after, t!("undo.edit_keysig").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_keysig_scale_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    tick: u32,
) {
    let scale = doc.data.model.conductor.key_sig.iter()
        .find(|e| e.tick == tick).map(|e| e.scale).unwrap_or(ScaleType::Major);
    let action = show_choice_popup(ui, salt, t!("event_browser.edit_keysig_scale").as_ref(),
        scale, ScaleType::ALL, |s| s.display_name().to_string());
    match action {
        ChoicePopupAction::Closed(new_scale) => {
            // scale 来自 EditRequest（tick），新 scale 来自 pending
            if let Some(e) = doc.data.model.conductor.key_sig.iter().find(|e| e.tick == tick) {
                let before = keysig_snapshot(doc);
                doc.set_keysig_event(tick, tick, e.root, new_scale);
                let after = keysig_snapshot(doc);
                push_event_list_undo(doc, EventListTarget::KeySig, before, after, t!("undo.edit_keysig").as_ref());
            }
            cleanup_edit_request(ui, salt);
        }
        ChoicePopupAction::Cancelled => cleanup_edit_request(ui, salt),
        ChoicePopupAction::None => {}
    }
}
