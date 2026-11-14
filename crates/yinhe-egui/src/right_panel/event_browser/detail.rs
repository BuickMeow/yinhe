//! 右侧详情面板：根据 `SelectedItem` 分发到对应的详情视图。
//!
//! `Automation` 统一处理 CC / PitchBend / RPN / NRPN / Tempo，
//! 不再为每种类型写单独的分支。
//!
//! 大数据量列表（automation / pc / text / notes）按
//! `(doc_id, track/target/kind, revision)` 缓存排序结果，避免每帧全量
//! clone+sort；任何编辑 bump `revision` 后自动失效重建。

use std::cell::RefCell;

use eframe::egui;
use egui_extras::TableRow;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::AutomationTarget;

use super::bar_lookup::BarLookup;
pub(super) use super::edit::{
    apply_automation_popups, apply_keysig_popups, apply_note_popups, apply_pc_popups,
    apply_text_popups, apply_timesig_popups,
};
pub(super) use super::edit_ops::{
    apply_automation_ops, apply_chord_ops, apply_conductor_chord_ops, apply_conductor_lyrics_ops,
    apply_keysig_ops, apply_lyrics_ops, apply_marker_ops, apply_notes_ops, apply_pc_ops,
    apply_timesig_ops,
};
use super::state::{
    EditRequest, EventBrowserState, JumpRequest, NoteRef, SelectedItem, TextEventKind,
};
use super::table::{
    AutomationEventOwned, build_table, cell_editable, cell_position, cell_row_header, cell_text,
    curve_points_text, empty_state_add_button, handle_delete_key, paginate, render_pager,
    shape_text, take_row_click, total_pages,
};

use automation::show_automation_detail;
use keysig::show_keysig_detail;
use notes::show_notes_detail;
use pc::show_pc_detail;
use project::{show_mapping_json, show_project_json};
use text::show_text_events_detail;
use timesig::show_timesig_detail;

mod automation;
mod keysig;
mod notes;
mod pc;
mod project;
mod summary;
mod text;
mod timesig;

pub(super) use summary::{show_overview, show_track_detail};

#[cfg(test)]
mod tests;

/// 根据选中的 item 渲染详情面板，返回可能的跳转请求。
pub(super) fn show_event_detail(
    ui: &mut egui::Ui,
    item: &SelectedItem,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
) -> Option<JumpRequest> {
    match item {
        SelectedItem::ProjectJson => {
            show_project_json(ui, doc);
            None
        }
        SelectedItem::MappingJson => {
            show_mapping_json(ui, doc);
            None
        }
        SelectedItem::TimeSig => show_timesig_detail(ui, doc, bar_lookup, state),
        SelectedItem::KeySig => show_keysig_detail(ui, doc, bar_lookup, state),
        SelectedItem::Markers => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_marker",
            t!("event_browser.title.marker").as_ref(),
            TextEventKind::Marker,
        ),
        SelectedItem::ConductorLyrics => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_cond_lyrics",
            t!("event_browser.title.lyrics").as_ref(),
            TextEventKind::ConductorLyrics,
        ),
        SelectedItem::ConductorChord => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_cond_chord",
            t!("event_browser.title.chord").as_ref(),
            TextEventKind::ConductorChord,
        ),
        SelectedItem::Notes { track } => show_notes_detail(ui, doc, bar_lookup, state, *track),
        SelectedItem::ProgramChange { track } => show_pc_detail(ui, doc, bar_lookup, state, *track),
        SelectedItem::Automation { track, target } => {
            show_automation_detail(ui, doc, bar_lookup, state, *track, target)
        }
        SelectedItem::Lyrics { track } => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_lyrics",
            t!("event_browser.title.lyrics").as_ref(),
            TextEventKind::Lyrics { track: *track },
        ),
        SelectedItem::Chord { track } => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_chord",
            t!("event_browser.title.chord").as_ref(),
            TextEventKind::Chord { track: *track },
        ),
    }
}

// ── 详情数据缓存 ──
//
// 键均含 doc_id + revision：文档切换或任何编辑（bump revision）后自动失效。
// 缓存只在渲染期间短暂取出、渲染完原样放回，不长期持有 `RefCell` 借用。

/// Automation 详情缓存：目标 lane 的全部事件（已按 tick 排序）。
struct AutomationCacheEntry {
    doc_id: u64,
    track: u16,
    target: AutomationTarget,
    revision: u64,
    /// 目标 lane 在 track 内的索引（Tempo 恒为 0）。
    lane_idx: usize,
    events: Vec<AutomationEventOwned>,
}

/// Program Change 详情缓存。
struct PcCacheEntry {
    doc_id: u64,
    track: u16,
    revision: u64,
    events: Vec<yinhe_types::PcEvent>,
}

/// 文本事件（Marker / Lyrics / Chord）详情缓存。
struct TextCacheEntry {
    doc_id: u64,
    kind: TextEventKind,
    revision: u64,
    events: Vec<(u32, String)>,
}

thread_local! {
    static AUTOMATION_CACHE: RefCell<Option<AutomationCacheEntry>> = const { RefCell::new(None) };
    static PC_CACHE: RefCell<Option<PcCacheEntry>> = const { RefCell::new(None) };
    static TEXT_CACHE: RefCell<Option<TextCacheEntry>> = const { RefCell::new(None) };
}

/// 收集 target 对应 lane 的事件（未排序）及 lane 索引。
fn collect_automation_events(
    doc: &Document,
    track: u16,
    target: &AutomationTarget,
) -> (usize, Vec<AutomationEventOwned>) {
    if matches!(target, AutomationTarget::Tempo) {
        let events = doc
            .data
            .model
            .conductor
            .tempo
            .events
            .iter()
            .map(|e| AutomationEventOwned {
                tick: e.tick,
                value: e.value,
                shape: e.shape,
            })
            .collect();
        return (0usize, events);
    }
    if let Some(td) = doc.data.model.tracks.get(track as usize) {
        for (li, lane) in td.automation_lanes.iter().enumerate() {
            if &lane.target == target {
                let events = lane
                    .events
                    .iter()
                    .map(|e| AutomationEventOwned {
                        tick: e.tick,
                        value: e.value,
                        shape: e.shape,
                    })
                    .collect();
                return (li, events);
            }
        }
    }
    (0usize, Vec::new())
}

/// 取出/重建 automation 缓存并传给 `f`；返回后原样放回。
fn with_automation_cache<R>(
    doc: &mut Document,
    track: u16,
    target: &AutomationTarget,
    f: impl FnOnce(&mut Document, &[AutomationEventOwned], usize) -> R,
) -> R {
    let doc_id = doc.doc_id;
    let revision = doc.data.revision;
    let mut entry = AUTOMATION_CACHE.with(|c| c.borrow_mut().take());
    let valid = matches!(
        &entry,
        Some(e) if e.doc_id == doc_id
            && e.track == track
            && e.target == *target
            && e.revision == revision
    );
    if !valid {
        let (lane_idx, mut events) = collect_automation_events(doc, track, target);
        events.sort_by_key(|e| e.tick);
        entry = Some(AutomationCacheEntry {
            doc_id,
            track,
            target: target.clone(),
            revision,
            lane_idx,
            events,
        });
    }
    let Some(entry) = entry else {
        return f(doc, &[], 0);
    };
    let result = f(doc, &entry.events, entry.lane_idx);
    AUTOMATION_CACHE.with(|c| *c.borrow_mut() = Some(entry));
    result
}

/// 取出/重建 Program Change 缓存并传给 `f`；返回后原样放回。
fn with_pc_cache<R>(
    doc: &mut Document,
    track: u16,
    f: impl FnOnce(&mut Document, &[yinhe_types::PcEvent]) -> R,
) -> R {
    let doc_id = doc.doc_id;
    let revision = doc.data.revision;
    let mut entry = PC_CACHE.with(|c| c.borrow_mut().take());
    let valid = matches!(
        &entry,
        Some(e) if e.doc_id == doc_id && e.track == track && e.revision == revision
    );
    if !valid {
        let mut events: Vec<yinhe_types::PcEvent> = doc
            .data
            .model
            .tracks
            .get(track as usize)
            .map(|td| td.program_change.to_vec())
            .unwrap_or_default();
        events.sort_by_key(|e| e.tick);
        entry = Some(PcCacheEntry {
            doc_id,
            track,
            revision,
            events,
        });
    }
    let Some(entry) = entry else {
        return f(doc, &[]);
    };
    let result = f(doc, &entry.events);
    PC_CACHE.with(|c| *c.borrow_mut() = Some(entry));
    result
}

/// 取出/重建文本事件缓存并传给 `f`；返回后原样放回。
fn with_text_cache<R>(
    doc: &mut Document,
    kind: TextEventKind,
    f: impl FnOnce(&mut Document, &[(u32, String)]) -> R,
) -> R {
    let doc_id = doc.doc_id;
    let revision = doc.data.revision;
    let mut entry = TEXT_CACHE.with(|c| c.borrow_mut().take());
    let valid = matches!(
        &entry,
        Some(e) if e.doc_id == doc_id && e.kind == kind && e.revision == revision
    );
    if !valid {
        let mut events: Vec<(u32, String)> = match kind {
            TextEventKind::Marker => doc
                .data
                .model
                .conductor
                .markers
                .iter()
                .map(|e| (e.tick, e.text.clone()))
                .collect(),
            TextEventKind::ConductorLyrics => doc
                .data
                .model
                .conductor
                .lyrics
                .iter()
                .map(|e| (e.tick, e.text.clone()))
                .collect(),
            TextEventKind::ConductorChord => doc
                .data
                .model
                .conductor
                .chord
                .iter()
                .map(|e| (e.tick, e.text.clone()))
                .collect(),
            TextEventKind::Lyrics { track } => doc
                .data
                .model
                .tracks
                .get(track as usize)
                .map(|t| t.lyrics.iter().map(|e| (e.tick, e.text.clone())).collect())
                .unwrap_or_default(),
            TextEventKind::Chord { track } => doc
                .data
                .model
                .tracks
                .get(track as usize)
                .map(|t| t.chord.iter().map(|e| (e.tick, e.text.clone())).collect())
                .unwrap_or_default(),
        };
        events.sort_by_key(|e| e.0);
        entry = Some(TextCacheEntry {
            doc_id,
            kind,
            revision,
            events,
        });
    }
    let Some(entry) = entry else {
        return f(doc, &[]);
    };
    let result = f(doc, &entry.events);
    TEXT_CACHE.with(|c| *c.borrow_mut() = Some(entry));
    result
}

// ── 表格骨架 ──

/// 事件详情表格骨架：标题 + 翻页 + 空状态/表格 + 删除键。
///
/// `tick_of` 提供每行 tick（页内多选锚点）；`row_cb` 接收
/// `(页内行号, row, click_key, state, 当前行, 页内 tick 列表, 页起始索引)`。
/// 返回 `(page_start, page_len)`，调用方用 `events[page_start + i]` 取点击行。
#[allow(clippy::too_many_arguments)] // 表格透传上下文，见 table.rs 同款约定
fn event_table_section<T>(
    ui: &mut egui::Ui,
    state: &mut EventBrowserState,
    table_id: &str,
    title_prefix: &str,
    events: &[T],
    headers: &[(&str, f32)],
    tick_of: impl Fn(&T) -> u32,
    mut row_cb: impl FnMut(usize, &mut TableRow, egui::Id, &mut EventBrowserState, &T, &[u32], usize),
) -> (usize, usize) {
    crate::right_panel::region_hint(ui, t!("hint.panel.event_table"));
    let edit_salt = format!("{}_edit", table_id);
    let (page, page_start, page_items) = paginate(state, events);
    let total = events.len();
    ui.add_space(4.0);
    crate::widgets::rows::inline_row(ui, |ui| {
        ui.label(
            egui::RichText::new(t!(
                "event_browser.table_title",
                title = title_prefix,
                n = total
            ))
            .strong()
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::SUB_TITLE_FONT,
            ))
            .color(crate::theme::text_bright()),
        );
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(crate::theme::GAP_TIGHT);
    if total == 0 {
        empty_state_add_button(ui, &edit_salt);
    } else {
        let page_ticks: Vec<u32> = page_items.iter().map(tick_of).collect();
        build_table(
            ui,
            table_id,
            headers,
            page_items.len(),
            |i, row, click_key| {
                let item = &page_items[i];
                row.set_selected(state.selected_ticks.contains(&page_ticks[i]));
                row_cb(i, row, click_key, state, item, &page_ticks, page_start);
            },
        );
        handle_delete_key(ui, &edit_salt, !state.selected_ticks.is_empty());
    }
    (page_start, page_items.len())
}

/// 键值行（project.json / mapping.json / track_detail 共用）。
fn kv(ui: &mut egui::Ui, k: &str, v: String) {
    crate::widgets::rows::value_row(ui, k, v);
}
