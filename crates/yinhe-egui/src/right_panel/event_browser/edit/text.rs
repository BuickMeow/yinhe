//! 文本类事件（Marker / Lyrics / Chord）的 tick / text 编辑 popup。
//!
//! 三种事件共用 `EditRequest::TextEventTick` / `TextEventText`，
//! 通过 `TextEventKind` 区分事件归属，分派到对应的 Document 方法。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;

use super::super::bar_lookup::BarLookup;
use super::super::state::{EditRequest, TextEventKind};
use super::super::table::{
    peek_edit_request, peek_pos_edit_request, remove_edit_request, remove_pos_edit_request,
    update_edit_request, update_pos_edit_request,
};
use super::{PopupAction, show_tick_popup};

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
    let Some(req) = peek_edit_request(ui, salt) else { return };
    match req {
        EditRequest::TextEventTick { kind, tick } => show_text_tick_popup(ui, doc, salt, kind, tick, None),
        EditRequest::TextEventText { kind, tick } => show_text_value_popup(ui, doc, salt, kind, tick),
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
    let before = record_text_before(ui, doc, salt, kind);
    let action = show_tick_popup(ui, salt, t!("event_browser.edit_tick").as_ref(), tick, 0, bar_lookup);
    match action {
        PopupAction::Changed(new_tick) => {
            let new_tick = new_tick as u32;
            if new_tick != tick {
                let text = text_event_text(&doc.data.model, kind, tick);
                if let Some(text) = text {
                    apply_text_event_edit(doc, kind, tick, new_tick, text);
                }
                let req = EditRequest::TextEventTick { kind, tick: new_tick };
                if bar_lookup.is_some() {
                    update_pos_edit_request(ui, salt, req);
                } else {
                    update_edit_request(ui, salt, req);
                }
            }
        }
        PopupAction::Closed => {
            finalize_text_undo(ui, doc, salt, before, kind, t!("undo.edit_text_event").as_ref());
        }
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
    let before = record_text_before(ui, doc, salt, kind);
    let old_text = text_event_text(&doc.data.model, kind, tick).unwrap_or_default();
    let action = show_text_edit_popup(ui, salt, t!("event_browser.edit_text").as_ref(), old_text.clone());
    match action {
        TextPopupAction::Changed(new_text) => {
            if new_text != old_text {
                apply_text_event_edit(doc, kind, tick, tick, new_text);
            }
        }
        TextPopupAction::Closed => {
            finalize_text_undo(ui, doc, salt, before, kind, t!("undo.edit_text_event").as_ref());
        }
        TextPopupAction::None => {}
    }
}

/// 文本编辑 popup 的动作。
enum TextPopupAction {
    None,
    Changed(String),
    Closed,
}

/// 渲染文本编辑 popup（Area + TextEdit + confirm）。
fn show_text_edit_popup(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    initial: String,
) -> TextPopupAction {
    let state_id = egui::Id::new((salt, "state"));
    let popup_id = ui.id().with((salt, "popup"));

    let mut state: String = ui.memory(|m| m.data.get_temp::<String>(state_id).unwrap_or_else(|| initial.clone()));
    let old_state = state.clone();
    let mut action = TextPopupAction::None;
    let mut open = true;
    let popup_pos = ui.clip_rect().min + egui::vec2(20.0, 20.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.label(egui::RichText::new(title).strong().size(11.0));
                ui.add_space(2.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut state)
                        .desired_width(200.0)
                        .font(egui::FontId::monospace(11.0)),
                );
                // 用直接比较替代 resp.changed()——后者在 Area 中不可靠
                if state != old_state {
                    action = TextPopupAction::Changed(state.clone());
                    ui.memory_mut(|m| m.data.insert_temp(state_id, state.clone()));
                }
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
                    }
                });
            });
        });

    if !open {
        ui.memory_mut(|m| m.data.remove::<String>(state_id));
        TextPopupAction::Closed
    } else {
        action
    }
}

/// 取文本类事件当前 text 字段值。
fn text_event_text(model: &yinhe_core::YinModel, kind: TextEventKind, tick: u32) -> Option<String> {
    match kind {
        TextEventKind::Marker => model.conductor.markers.iter()
            .find(|e| e.tick == tick).map(|e| e.text.clone()),
        TextEventKind::ConductorLyrics => model.conductor.lyrics.iter()
            .find(|e| e.tick == tick).map(|e| e.text.clone()),
        TextEventKind::ConductorChord => model.conductor.chord.iter()
            .find(|e| e.tick == tick).map(|e| e.text.clone()),
        TextEventKind::Lyrics { track } => model.tracks.get(track as usize)
            .and_then(|t| t.lyrics.iter().find(|e| e.tick == tick))
            .map(|e| e.text.clone()),
        TextEventKind::Chord { track } => model.tracks.get(track as usize)
            .and_then(|t| t.chord.iter().find(|e| e.tick == tick))
            .map(|e| e.text.clone()),
    }
}

/// 应用文本类事件编辑到 Document。
fn apply_text_event_edit(doc: &mut Document, kind: TextEventKind, old_tick: u32, new_tick: u32, new_text: String) {
    match kind {
        TextEventKind::Marker => doc.set_marker_event(old_tick, new_tick, new_text),
        TextEventKind::ConductorLyrics => doc.set_conductor_lyrics_event(old_tick, new_tick, new_text),
        TextEventKind::ConductorChord => doc.set_conductor_chord_event(old_tick, new_tick, new_text),
        TextEventKind::Lyrics { track } => doc.set_lyrics_event(track, old_tick, new_tick, new_text),
        TextEventKind::Chord { track } => doc.set_chord_event(track, old_tick, new_tick, new_text),
    }
}

/// popup 显示期间记录 before 快照（仅第一次记录）。
/// Marker/Lyrics/Chord 三种事件的 before 都序列化为 `Vec<(u32, String)>`，
/// 避免为三种类型各写一份 record/finalize。
fn record_text_before(
    ui: &egui::Ui,
    doc: &Document,
    salt: &str,
    kind: TextEventKind,
) -> Option<Vec<(u32, String)>> {
    let before_id = egui::Id::new((salt, "before"));
    let recorded_id = before_id.with("recorded");
    let recorded = ui.memory(|m| m.data.get_temp::<bool>(recorded_id).unwrap_or(false));
    if !recorded {
        let before: Vec<(u32, String)> = match kind {
            TextEventKind::Marker => doc.data.model.conductor.markers
                .iter().map(|e| (e.tick, e.text.clone())).collect(),
            TextEventKind::ConductorLyrics => doc.data.model.conductor.lyrics
                .iter().map(|e| (e.tick, e.text.clone())).collect(),
            TextEventKind::ConductorChord => doc.data.model.conductor.chord
                .iter().map(|e| (e.tick, e.text.clone())).collect(),
            TextEventKind::Lyrics { track } => doc.data.model.tracks.get(track as usize)
                .map(|t| t.lyrics.iter().map(|e| (e.tick, e.text.clone())).collect())
                .unwrap_or_default(),
            TextEventKind::Chord { track } => doc.data.model.tracks.get(track as usize)
                .map(|t| t.chord.iter().map(|e| (e.tick, e.text.clone())).collect())
                .unwrap_or_default(),
        };
        ui.memory_mut(|m| {
            m.data.insert_temp(before_id, before.clone());
            m.data.insert_temp(recorded_id, true);
        });
        Some(before)
    } else {
        ui.memory(|m| m.data.get_temp::<Vec<(u32, String)>>(before_id))
    }
}

/// popup 关闭时取 after 对比，push undo，清除所有 popup 状态。
fn finalize_text_undo(
    ui: &egui::Ui,
    doc: &mut Document,
    salt: &str,
    before: Option<Vec<(u32, String)>>,
    kind: TextEventKind,
    label: &str,
) {
    use yinhe_editor_core::history::{UndoAction, UndoEntry};
    if let Some(before) = before {
        let after: Vec<(u32, String)> = match kind {
            TextEventKind::Marker => doc.data.model.conductor.markers
                .iter().map(|e| (e.tick, e.text.clone())).collect(),
            TextEventKind::ConductorLyrics => doc.data.model.conductor.lyrics
                .iter().map(|e| (e.tick, e.text.clone())).collect(),
            TextEventKind::ConductorChord => doc.data.model.conductor.chord
                .iter().map(|e| (e.tick, e.text.clone())).collect(),
            TextEventKind::Lyrics { track } => doc.data.model.tracks.get(track as usize)
                .map(|t| t.lyrics.iter().map(|e| (e.tick, e.text.clone())).collect())
                .unwrap_or_default(),
            TextEventKind::Chord { track } => doc.data.model.tracks.get(track as usize)
                .map(|t| t.chord.iter().map(|e| (e.tick, e.text.clone())).collect())
                .unwrap_or_default(),
        };
        if before != after {
            let action = match kind {
                TextEventKind::Marker => {
                    let old: Vec<_> = before.into_iter().map(|(tick, text)| yinhe_types::MarkerEvent { tick, text }).collect();
                    let new: Vec<_> = after.into_iter().map(|(tick, text)| yinhe_types::MarkerEvent { tick, text }).collect();
                    UndoAction::Marker { old, new }
                }
                TextEventKind::ConductorLyrics => {
                    let old: Vec<_> = before.into_iter().map(|(tick, text)| yinhe_types::LyricsEvent { tick, text }).collect();
                    let new: Vec<_> = after.into_iter().map(|(tick, text)| yinhe_types::LyricsEvent { tick, text }).collect();
                    UndoAction::ConductorLyrics { old, new }
                }
                TextEventKind::ConductorChord => {
                    let old: Vec<_> = before.into_iter().map(|(tick, text)| yinhe_types::ChordEvent { tick, text }).collect();
                    let new: Vec<_> = after.into_iter().map(|(tick, text)| yinhe_types::ChordEvent { tick, text }).collect();
                    UndoAction::ConductorChord { old, new }
                }
                TextEventKind::Lyrics { track } => {
                    let old: Vec<_> = before.into_iter().map(|(tick, text)| yinhe_types::LyricsEvent { tick, text }).collect();
                    let new: Vec<_> = after.into_iter().map(|(tick, text)| yinhe_types::LyricsEvent { tick, text }).collect();
                    UndoAction::Lyrics { track, old, new }
                }
                TextEventKind::Chord { track } => {
                    let old: Vec<_> = before.into_iter().map(|(tick, text)| yinhe_types::ChordEvent { tick, text }).collect();
                    let new: Vec<_> = after.into_iter().map(|(tick, text)| yinhe_types::ChordEvent { tick, text }).collect();
                    UndoAction::Chord { track, old, new }
                }
            };
            doc.history.push(UndoEntry {
                action,
                label: label.to_string(),
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
        }
    }
    let before_id = egui::Id::new((salt, "before"));
    ui.memory_mut(|m| {
        m.data.remove::<Vec<(u32, String)>>(before_id);
        m.data.remove::<bool>(before_id.with("recorded"));
    });
    remove_edit_request(ui, salt);
    remove_pos_edit_request(ui, salt);
}
