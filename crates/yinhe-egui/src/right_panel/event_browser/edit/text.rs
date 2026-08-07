//! 文本类事件（Marker / Lyrics / Chord）的 tick / text 编辑 popup。
//!
//! 三种事件共用 `EditRequest::TextEventTick` / `TextEventText`，
//! 通过 `TextEventKind` 区分事件归属，分派到对应的 Document 方法。
//! popup 打开期间不修改 Document，pending 写到 egui memory。
//! 关闭时（Closed）一次性 apply + push undo；取消（Cancelled）仅清理。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{EventListItem, EventListTarget};

use super::super::bar_lookup::BarLookup;
use super::super::state::{EditRequest, TextEventKind};
use super::super::table::{peek_edit_request, peek_pos_edit_request, remove_pos_edit_request};
use super::{PopupAction, cleanup_edit_request, push_event_list_undo, show_tick_popup};

/// 处理文本类事件（Marker/Lyrics/Chord）的 tick / text 编辑 popup。
///
/// 优先响应位置编辑请求（`(salt, "edit_pos")` key），再响应普通编辑请求。
pub fn apply_text_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    bar_lookup: &BarLookup,
) {
    if let Some(req) = peek_pos_edit_request(ui, salt) {
        match req {
            EditRequest::TextEventTick { kind, tick } => {
                show_text_tick_popup(ui, doc, salt, kind, tick, Some(bar_lookup))
            }
            _ => remove_pos_edit_request(ui, salt),
        }
        return;
    }
    let Some(req) = peek_edit_request(ui, salt) else {
        return;
    };
    match req {
        EditRequest::TextEventTick { kind, tick } => {
            show_text_tick_popup(ui, doc, salt, kind, tick, None)
        }
        EditRequest::TextEventText { kind, tick } => {
            show_text_value_popup(ui, doc, salt, kind, tick)
        }
        _ => {}
    }
}

fn show_text_tick_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    kind: TextEventKind,
    tick: u32,
    bar_lookup: Option<&BarLookup>,
) {
    let action = show_tick_popup(
        ui,
        salt,
        t!("event_browser.edit_tick").as_ref(),
        tick,
        0,
        bar_lookup,
    );
    match action {
        PopupAction::Closed(new_tick_f) => {
            let new_tick = new_tick_f as u32;
            // old_tick 来自 EditRequest（tick），new_tick 来自 pending；text 保持不变
            if let Some(text) = text_event_text(&doc.data.model, kind, tick) {
                let snapshot = doc.capture_snapshot();
                let before = text_snapshot(doc, kind);
                apply_text_event_edit(doc, kind, tick, new_tick, text);
                let after = text_snapshot(doc, kind);
                push_event_list_undo(
                    doc,
                    text_target(kind),
                    before,
                    after,
                    t!("undo.edit_text_event").as_ref(),
                    snapshot,
                );
            }
            cleanup_edit_request(ui, salt);
        }
        PopupAction::Cancelled => cleanup_edit_request(ui, salt),
        PopupAction::None => {}
    }
}

fn show_text_value_popup(
    ui: &mut egui::Ui,
    doc: &mut Document,
    salt: &str,
    kind: TextEventKind,
    tick: u32,
) {
    let old_text = text_event_text(&doc.data.model, kind, tick).unwrap_or_default();
    let action = show_text_edit_popup(
        ui,
        salt,
        t!("event_browser.edit_text").as_ref(),
        old_text.clone(),
    );
    match action {
        TextPopupAction::Closed(new_text) => {
            if new_text != old_text {
                let snapshot = doc.capture_snapshot();
                let before = text_snapshot(doc, kind);
                apply_text_event_edit(doc, kind, tick, tick, new_text);
                let after = text_snapshot(doc, kind);
                push_event_list_undo(
                    doc,
                    text_target(kind),
                    before,
                    after,
                    t!("undo.edit_text_event").as_ref(),
                    snapshot,
                );
            }
            cleanup_edit_request(ui, salt);
        }
        TextPopupAction::Cancelled => cleanup_edit_request(ui, salt),
        TextPopupAction::None => {}
    }
}

/// 文本编辑 popup 的关闭事件。语义与 `PopupAction` 一致。
enum TextPopupAction {
    None,
    Closed(String),
    Cancelled,
}

/// 渲染文本编辑 popup（Area + TextEdit + confirm/cancel）。
///
/// 状态持久化到 `(salt, "state")`，每帧从 memory 读出。关闭时返回
/// `Closed(text)` 携带 pending 文本，或 `Cancelled`。
fn show_text_edit_popup(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    initial: String,
) -> TextPopupAction {
    let state_id = egui::Id::new((salt, "state"));
    let popup_id = ui.id().with((salt, "popup"));

    let mut state: String = ui.memory(|m| {
        m.data
            .get_temp::<String>(state_id)
            .unwrap_or_else(|| initial.clone())
    });
    let mut open = true;
    let mut cancelled = false;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.label(
                    egui::RichText::new(title)
                        .strong()
                        .size(crate::theme::SMALL_FONT),
                );
                ui.add_space(2.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut state)
                        .desired_width(200.0)
                        .font(egui::FontId::monospace(11.0)),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    open = false;
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("common.confirm").as_ref()).clicked() {
                        open = false;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        open = false;
                        cancelled = true;
                    }
                });
            });
        });

    if !open {
        ui.memory_mut(|m| m.data.remove::<String>(state_id));
        if cancelled {
            TextPopupAction::Cancelled
        } else {
            TextPopupAction::Closed(state)
        }
    } else {
        ui.memory_mut(|m| m.data.insert_temp(state_id, state));
        TextPopupAction::None
    }
}

/// 取文本类事件当前 text 字段值。
fn text_event_text(model: &yinhe_core::YinModel, kind: TextEventKind, tick: u32) -> Option<String> {
    match kind {
        TextEventKind::Marker => model
            .conductor
            .markers
            .iter()
            .find(|e| e.tick == tick)
            .map(|e| e.text.clone()),
        TextEventKind::ConductorLyrics => model
            .conductor
            .lyrics
            .iter()
            .find(|e| e.tick == tick)
            .map(|e| e.text.clone()),
        TextEventKind::ConductorChord => model
            .conductor
            .chord
            .iter()
            .find(|e| e.tick == tick)
            .map(|e| e.text.clone()),
        TextEventKind::Lyrics { track } => model
            .tracks
            .get(track as usize)
            .and_then(|t| t.lyrics.iter().find(|e| e.tick == tick))
            .map(|e| e.text.clone()),
        TextEventKind::Chord { track } => model
            .tracks
            .get(track as usize)
            .and_then(|t| t.chord.iter().find(|e| e.tick == tick))
            .map(|e| e.text.clone()),
    }
}

/// 应用文本类事件编辑到 Document（set = upsert）。
fn apply_text_event_edit(
    doc: &mut Document,
    kind: TextEventKind,
    old_tick: u32,
    new_tick: u32,
    new_text: String,
) {
    match kind {
        TextEventKind::Marker => doc.set_marker_event(old_tick, new_tick, new_text),
        TextEventKind::ConductorLyrics => {
            doc.set_conductor_lyrics_event(old_tick, new_tick, new_text)
        }
        TextEventKind::ConductorChord => {
            doc.set_conductor_chord_event(old_tick, new_tick, new_text)
        }
        TextEventKind::Lyrics { track } => {
            doc.set_lyrics_event(track, old_tick, new_tick, new_text)
        }
        TextEventKind::Chord { track } => doc.set_chord_event(track, old_tick, new_tick, new_text),
    }
}

/// 文本类事件的 undo 写入目标。
fn text_target(kind: TextEventKind) -> EventListTarget {
    match kind {
        TextEventKind::Marker => EventListTarget::Marker,
        TextEventKind::ConductorLyrics => EventListTarget::ConductorLyrics,
        TextEventKind::ConductorChord => EventListTarget::ConductorChord,
        TextEventKind::Lyrics { track } => EventListTarget::Lyrics { track },
        TextEventKind::Chord { track } => EventListTarget::Chord { track },
    }
}

/// 文本类事件列表的当前快照（用于 undo before/after）。
fn text_snapshot(doc: &Document, kind: TextEventKind) -> Vec<EventListItem> {
    match kind {
        TextEventKind::Marker => doc
            .data
            .model
            .conductor
            .markers
            .iter()
            .cloned()
            .map(EventListItem::Marker)
            .collect(),
        TextEventKind::ConductorLyrics => doc
            .data
            .model
            .conductor
            .lyrics
            .iter()
            .cloned()
            .map(EventListItem::Lyrics)
            .collect(),
        TextEventKind::ConductorChord => doc
            .data
            .model
            .conductor
            .chord
            .iter()
            .cloned()
            .map(EventListItem::Chord)
            .collect(),
        TextEventKind::Lyrics { track } => doc
            .data
            .model
            .tracks
            .get(track as usize)
            .map(|t| {
                t.lyrics
                    .iter()
                    .cloned()
                    .map(EventListItem::Lyrics)
                    .collect()
            })
            .unwrap_or_default(),
        TextEventKind::Chord { track } => doc
            .data
            .model
            .tracks
            .get(track as usize)
            .map(|t| t.chord.iter().cloned().map(EventListItem::Chord).collect())
            .unwrap_or_default(),
    }
}
