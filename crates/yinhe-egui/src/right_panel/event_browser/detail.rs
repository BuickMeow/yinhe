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
use egui_material_icons::icons::{ICON_CANCEL, ICON_CHECK_CIRCLE};

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::AutomationLane;
use yinhe_types::AutomationTarget;
use yinhe_types::automation::{ParamDevice, xsynth_param};

use super::bar_lookup::BarLookup;
use super::edit::{
    apply_automation_popups, apply_keysig_popups, apply_note_popups, apply_pc_popups,
    apply_text_popups, apply_timesig_popups,
};
use super::edit_ops::{
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

// ── Automation（统一 CC/PB/RPN/NRPN/Tempo） ──

fn show_automation_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
    target: &AutomationTarget,
) -> Option<JumpRequest> {
    // Automation：仅跳转不闪烁；Tempo 不切 track（note=None）
    let note = if matches!(target, AutomationTarget::Tempo) {
        None
    } else {
        Some((track, 0))
    };
    with_automation_cache(doc, track, target, |doc, events, lane_idx| {
        let (page_start, _) = event_table_section(
            ui,
            state,
            "eb_auto",
            &target.display_name(),
            events,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (t!("event_browser.header.value").as_ref(), 60.0),
                ("X1", 50.0),
                ("Y1", 50.0),
                ("X2", 50.0),
                ("Y2", 50.0),
                (t!("event_browser.header.shape").as_ref(), 90.0),
            ],
            |e| e.tick,
            |i, row, click_key, state, e, page_ticks, page_start| {
                cell_row_header(
                    row,
                    state,
                    "eb_auto_edit",
                    i,
                    page_start,
                    e.tick,
                    page_ticks,
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_auto_edit",
                    i,
                    format!("{}", e.tick),
                    EditRequest::AutoTick {
                        tick: e.tick,
                        value: e.value,
                    },
                    click_key,
                );
                cell_position(
                    row,
                    bar_lookup,
                    "eb_auto_edit",
                    i,
                    e.tick,
                    |nt| EditRequest::AutoTick {
                        tick: nt,
                        value: e.value,
                    },
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_auto_edit",
                    i,
                    crate::piano_view::automation_panel::format_display_value(target, e.value),
                    EditRequest::AutoValue {
                        tick: e.tick,
                        value: e.value,
                    },
                    click_key,
                );
                let [x1, y1, x2, y2] = curve_points_text(e.shape);
                cell_text(row, x1, click_key, i);
                cell_text(row, y1, click_key, i);
                cell_text(row, x2, click_key, i);
                cell_text(row, y2, click_key, i);
                cell_editable(
                    row,
                    "eb_auto_edit",
                    i,
                    shape_text(e.shape),
                    EditRequest::AutoShape {
                        tick: e.tick,
                        shape: e.shape,
                    },
                    click_key,
                );
            },
        );

        apply_automation_popups(ui, doc, "eb_auto_edit", track, lane_idx, target, bar_lookup);
        apply_automation_ops(ui, doc, state, "eb_auto_edit", track, target);

        take_row_click(ui, "eb_auto").map(|i| JumpRequest {
            tick: events[page_start + i].tick,
            note,
        })
    })
}

// ── TimeSig ──

fn show_timesig_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
) -> Option<JumpRequest> {
    // 先 clone 出 owned 数据，避免不可变借用阻塞后续 &mut doc 编辑
    let mut sorted: Vec<yinhe_types::TimeSigEvent> = doc.data.model.conductor.time_sig.clone();
    sorted.sort_by_key(|e| e.tick);
    let (page_start, _) = event_table_section(
        ui,
        state,
        "eb_ts",
        t!("event_browser.title.timesig").as_ref(),
        &sorted,
        &[
            ("#", 40.0),
            (t!("event_browser.header.tick").as_ref(), 70.0),
            (t!("event_browser.header.position").as_ref(), 80.0),
            (t!("event_browser.header.numerator").as_ref(), 50.0),
            (t!("event_browser.header.denominator").as_ref(), 50.0),
        ],
        |e| e.tick,
        |i, row, click_key, state, e, page_ticks, page_start| {
            let denom = 1u32 << e.denominator as u32;
            cell_row_header(
                row,
                state,
                "eb_ts_edit",
                i,
                page_start,
                e.tick,
                page_ticks,
                click_key,
            );
            cell_editable(
                row,
                "eb_ts_edit",
                i,
                format!("{}", e.tick),
                EditRequest::TimeSigTick { tick: e.tick },
                click_key,
            );
            cell_position(
                row,
                bar_lookup,
                "eb_ts_edit",
                i,
                e.tick,
                |nt| EditRequest::TimeSigTick { tick: nt },
                click_key,
            );
            cell_editable(
                row,
                "eb_ts_edit",
                i,
                format!("{}", e.numerator),
                EditRequest::TimeSigNumerator { tick: e.tick },
                click_key,
            );
            cell_editable(
                row,
                "eb_ts_edit",
                i,
                format!("{}", denom),
                EditRequest::TimeSigDenominator { tick: e.tick },
                click_key,
            );
        },
    );
    apply_timesig_popups(ui, doc, "eb_ts_edit", bar_lookup);
    apply_timesig_ops(ui, doc, state, "eb_ts_edit");
    take_row_click(ui, "eb_ts").map(|i| JumpRequest {
        tick: sorted[page_start + i].tick,
        note: None,
    })
}

// ── KeySig ──

fn show_keysig_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
) -> Option<JumpRequest> {
    // 先 clone 出 owned 数据，避免不可变借用阻塞后续 &mut doc 编辑
    let mut sorted: Vec<yinhe_types::KeySigEvent> = doc.data.model.conductor.key_sig.clone();
    sorted.sort_by_key(|e| e.tick);
    let (page_start, _) = event_table_section(
        ui,
        state,
        "eb_ks",
        t!("event_browser.title.keysig").as_ref(),
        &sorted,
        &[
            ("#", 40.0),
            (t!("event_browser.header.tick").as_ref(), 70.0),
            (t!("event_browser.header.position").as_ref(), 80.0),
            (t!("event_browser.header.keysig").as_ref(), 100.0),
            (t!("event_browser.header.root").as_ref(), 60.0),
            (t!("event_browser.header.scale").as_ref(), 80.0),
        ],
        |e| e.tick,
        |i, row, click_key, state, e, page_ticks, page_start| {
            cell_row_header(
                row,
                state,
                "eb_ks_edit",
                i,
                page_start,
                e.tick,
                page_ticks,
                click_key,
            );
            cell_editable(
                row,
                "eb_ks_edit",
                i,
                format!("{}", e.tick),
                EditRequest::KeySigTick { tick: e.tick },
                click_key,
            );
            cell_position(
                row,
                bar_lookup,
                "eb_ks_edit",
                i,
                e.tick,
                |nt| EditRequest::KeySigTick { tick: nt },
                click_key,
            );
            cell_text(row, keysig_text(e.root, e.scale), click_key, i);
            cell_editable(
                row,
                "eb_ks_edit",
                i,
                format!("{} ({})", ROOT_NAMES[e.root as usize % 12], e.root),
                EditRequest::KeySigRoot { tick: e.tick },
                click_key,
            );
            cell_editable(
                row,
                "eb_ks_edit",
                i,
                e.scale.display_name(),
                EditRequest::KeySigScale { tick: e.tick },
                click_key,
            );
        },
    );
    apply_keysig_popups(ui, doc, "eb_ks_edit", bar_lookup);
    apply_keysig_ops(ui, doc, state, "eb_ks_edit");
    take_row_click(ui, "eb_ks").map(|i| JumpRequest {
        tick: sorted[page_start + i].tick,
        note: None,
    })
}

/// 12 个 pitch class 的显示名（0=C, 1=C#/Db, ..., 11=B）。
const ROOT_NAMES: [&str; 12] = [
    "C", "C#/Db", "D", "D#/Eb", "E", "F", "F#/Gb", "G", "G#/Ab", "A", "A#/Bb", "B",
];

/// 调号文本：根音名 + 音阶名（如 "D 多利亚"）。
fn keysig_text(root: u8, scale: yinhe_types::ScaleType) -> String {
    let name = ROOT_NAMES[root as usize % 12];
    format!("{} {}", name, scale.display_name())
}

// ── 通用文本事件（Marker / Lyrics / Chord） ──

fn show_text_events_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    table_id: &str,
    label: &str,
    kind: TextEventKind,
) -> Option<JumpRequest> {
    with_text_cache(doc, kind, |doc, events| {
        let edit_salt = format!("{}_edit", table_id);
        let (page_start, _) = event_table_section(
            ui,
            state,
            table_id,
            label,
            events,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (label, 200.0),
            ],
            |item| item.0,
            |i, row, click_key, state, item, page_ticks, page_start| {
                let (tick, text) = item;
                cell_row_header(
                    row, state, &edit_salt, i, page_start, *tick, page_ticks, click_key,
                );
                cell_editable(
                    row,
                    &edit_salt,
                    i,
                    format!("{}", tick),
                    EditRequest::TextEventTick { kind, tick: *tick },
                    click_key,
                );
                cell_position(
                    row,
                    bar_lookup,
                    &edit_salt,
                    i,
                    *tick,
                    move |nt| EditRequest::TextEventTick { kind, tick: nt },
                    click_key,
                );
                cell_editable(
                    row,
                    &edit_salt,
                    i,
                    text.clone(),
                    EditRequest::TextEventText { kind, tick: *tick },
                    click_key,
                );
            },
        );
        apply_text_popups(ui, doc, &edit_salt, bar_lookup);
        // 分派删除/插入操作
        match kind {
            TextEventKind::Marker => apply_marker_ops(ui, doc, state, &edit_salt),
            TextEventKind::ConductorLyrics => {
                apply_conductor_lyrics_ops(ui, doc, state, &edit_salt)
            }
            TextEventKind::ConductorChord => apply_conductor_chord_ops(ui, doc, state, &edit_salt),
            TextEventKind::Lyrics { track } => apply_lyrics_ops(ui, doc, state, &edit_salt, track),
            TextEventKind::Chord { track } => apply_chord_ops(ui, doc, state, &edit_salt, track),
        }
        take_row_click(ui, table_id).map(|i| JumpRequest {
            tick: events[page_start + i].0,
            note: None,
        })
    })
}

// ── Notes ──

fn show_notes_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
) -> Option<JumpRequest> {
    // 全轨音符收集 + 排序是 3GB 级开销（1.64 亿场景），按 (doc_id, track,
    // revision) 缓存；任何编辑 bump revision 后自动重建。
    let revision = doc.data.revision;
    let cache_valid = matches!(
        &state.notes_cache,
        Some((d, t, r, _)) if *d == doc.doc_id && *t == track && *r == revision
    );
    if !cache_valid {
        let model = &doc.data.model;
        let track_count = model
            .track_note_count
            .get(track as usize)
            .copied()
            .unwrap_or(0) as usize;
        let mut notes: Vec<yinhe_core::NoteEvent> = Vec::with_capacity(track_count);
        for (key, bucket) in model.notes.iter().enumerate() {
            if !model.bucket_track_stats[key].contains_key(&track) {
                continue;
            }
            for n in bucket.iter().filter(|n| n.track == track) {
                notes.push(yinhe_core::NoteEvent {
                    id: n.id,
                    start_tick: n.start_tick,
                    end_tick: n.end_tick,
                    key: key as u8,
                    velocity: n.velocity,
                });
            }
        }
        notes.sort_by_key(|n| n.start_tick);
        state.notes_cache = Some((doc.doc_id, track, revision, notes));
    }
    // 短暂取出 owned 数据，避免与 `&mut state`（分页/行选择）借用冲突。
    let (cache_doc, cache_track, cache_rev, notes) = state.notes_cache.take()?;
    let (page_start, _) = event_table_section(
        ui,
        state,
        "eb_notes",
        t!("event_browser.title.notes").as_ref(),
        &notes,
        &[
            ("#", 40.0),
            ("id", 70.0),
            (t!("event_browser.header.tick").as_ref(), 70.0),
            (t!("event_browser.header.position").as_ref(), 80.0),
            ("gate", 60.0),
            (t!("event_browser.header.end_tick").as_ref(), 80.0),
            (t!("event_browser.header.end_position").as_ref(), 90.0),
            (t!("event_browser.header.key").as_ref(), 50.0),
            (t!("event_browser.header.velocity").as_ref(), 50.0),
        ],
        |n| n.start_tick,
        |i, row, click_key, state, n, page_ticks, page_start| {
            let note_ref = NoteRef {
                id: n.id,
                start_tick: n.start_tick,
                end_tick: n.end_tick,
                key: n.key,
                velocity: n.velocity,
                track,
            };
            let gate = n.end_tick.saturating_sub(n.start_tick);
            cell_row_header(
                row,
                state,
                "eb_notes_edit",
                i,
                page_start,
                n.start_tick,
                page_ticks,
                click_key,
            );
            cell_text(row, format!("#{}", n.id), click_key, i);
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.start_tick),
                EditRequest::NoteStartTick { note: note_ref },
                click_key,
            );
            let nr_start = note_ref;
            cell_position(
                row,
                bar_lookup,
                "eb_notes_edit",
                i,
                n.start_tick,
                move |nt| EditRequest::NoteStartTick {
                    note: NoteRef {
                        start_tick: nt,
                        ..nr_start
                    },
                },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", gate),
                EditRequest::NoteGate { note: note_ref },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.end_tick),
                EditRequest::NoteEndTick { note: note_ref },
                click_key,
            );
            let nr_end = note_ref;
            cell_position(
                row,
                bar_lookup,
                "eb_notes_edit",
                i,
                n.end_tick,
                move |nt| EditRequest::NoteEndTick {
                    note: NoteRef {
                        end_tick: nt,
                        ..nr_end
                    },
                },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.key),
                EditRequest::NoteKey { note: note_ref },
                click_key,
            );
            cell_editable(
                row,
                "eb_notes_edit",
                i,
                format!("{}", n.velocity),
                EditRequest::NoteVelocity { note: note_ref },
                click_key,
            );
        },
    );
    // 音符：切到音符所在 track
    let jump = take_row_click(ui, "eb_notes").map(|i| {
        let n = &notes[page_start + i];
        JumpRequest {
            tick: n.start_tick,
            note: Some((track, n.key)),
        }
    });
    state.notes_cache = Some((cache_doc, cache_track, cache_rev, notes));
    apply_note_popups(ui, doc, "eb_notes_edit", bar_lookup);
    apply_notes_ops(ui, doc, state, "eb_notes_edit", track);
    jump
}

// ── Program Change ──

fn show_pc_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
) -> Option<JumpRequest> {
    with_pc_cache(doc, track, |doc, events| {
        let (page_start, _) = event_table_section(
            ui,
            state,
            "eb_pc",
            t!("event_browser.title.program_change").as_ref(),
            events,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (t!("event_browser.header.program").as_ref(), 50.0),
            ],
            |e| e.tick,
            |i, row, click_key, state, e, page_ticks, page_start| {
                cell_row_header(
                    row,
                    state,
                    "eb_pc_edit",
                    i,
                    page_start,
                    e.tick,
                    page_ticks,
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_pc_edit",
                    i,
                    format!("{}", e.tick),
                    EditRequest::PcTick { tick: e.tick },
                    click_key,
                );
                cell_position(
                    row,
                    bar_lookup,
                    "eb_pc_edit",
                    i,
                    e.tick,
                    |nt| EditRequest::PcTick { tick: nt },
                    click_key,
                );
                cell_editable(
                    row,
                    "eb_pc_edit",
                    i,
                    format!("{}", e.program),
                    EditRequest::PcProgram { tick: e.tick },
                    click_key,
                );
            },
        );
        apply_pc_popups(ui, doc, "eb_pc_edit", track, bar_lookup);
        apply_pc_ops(ui, doc, state, "eb_pc_edit", track);
        // PC：切到所在 track，仅跳转不闪烁
        take_row_click(ui, "eb_pc").map(|i| JumpRequest {
            tick: events[page_start + i].tick,
            note: Some((track, 0)),
        })
    })
}

// ── project.json / mapping.json ──

/// 键值行（project.json / mapping.json / track_detail 共用）。
fn kv(ui: &mut egui::Ui, k: &str, v: String) {
    crate::widgets::rows::value_row(ui, k, v);
}

fn show_project_json(ui: &mut egui::Ui, doc: &Document) {
    let pf = &doc.data.project_file;
    crate::widgets::rows::section_header(ui, "project.json");

    kv(ui, "version", format!("{}", pf.version));
    kv(ui, "name", pf.name.clone());
    kv(ui, "artist", pf.artist.clone());
    kv(ui, "description", pf.description.clone());
    kv(ui, "ppq", format!("{}", pf.ppq));
    kv(ui, "compression_level", format!("{}", pf.compression_level));
    kv(
        ui,
        "soundfont_channels",
        format!("{}", pf.sf_channel_overrides.len()),
    );

    if !pf.sf_channel_overrides.is_empty() {
        ui.add_space(crate::theme::GAP_SM);
        ui.label(
            egui::RichText::new("soundfont_overrides")
                .strong()
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(crate::theme::text_bright()),
        );
        for po in &pf.sf_channel_overrides {
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::INDENT_STEP);
                ui.label(
                    egui::RichText::new(format!("channel {}:", po.channel))
                        .size(crate::scaling::scaled_font(
                            ui.ctx(),
                            crate::theme::SMALL_FONT,
                        ))
                        .color(crate::theme::text_label()),
                );
            });
            for entry in &po.entries {
                ui.horizontal(|ui| {
                    ui.add_space(crate::theme::INDENT_STEP * 2.0);
                    let (icon, color) = if entry.enabled {
                        (ICON_CHECK_CIRCLE, crate::theme::accent_active())
                    } else {
                        (ICON_CANCEL, crate::theme::text_disabled())
                    };
                    ui.label(crate::widgets::icon_text::icon_text(
                        icon,
                        &format!("{} ({})", entry.name, entry.path),
                        crate::scaling::scaled_font(ui.ctx(), crate::theme::SMALL_LABEL_FONT),
                        color,
                    ));
                });
            }
        }
    }
}

fn show_mapping_json(ui: &mut egui::Ui, doc: &Document) {
    let mf = &doc.data.mapping_file;
    crate::widgets::rows::section_header(ui, "mapping.json");

    kv(ui, "version", format!("{}", mf.version));

    ui.add_space(crate::theme::GAP_SM);
    ui.label(
        egui::RichText::new("ports")
            .strong()
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::SMALL_FONT,
            ))
            .color(crate::theme::text_bright()),
    );
    for p in &mf.ports {
        for ch in &p.channels {
            for t in &ch.tracks {
                let muted = if t.muted {
                    t!("event_browser.muted_badge").to_string()
                } else {
                    String::new()
                };
                let soloed = if t.soloed {
                    t!("event_browser.solo_badge").to_string()
                } else {
                    String::new()
                };
                kv(
                    ui,
                    &format!("P{} Ch{}", p.port, ch.channel + 1),
                    format!("{} ({}){}{}", t.name, &t.uuid[..8], muted, soloed),
                );
            }
        }
    }
}

// ── Overview / Track detail ──

/// automation lane 分类统计（overview 与 track_detail 共用）。
#[derive(Debug, Default, PartialEq, Eq)]
struct TargetSummary {
    cc_total: usize,
    /// `(controller, 事件数)`，按首次出现顺序排列。
    cc_per_controller: Vec<(u8, usize)>,
    pb_total: usize,
    param_total: usize,
    rpn_total: usize,
}

impl TargetSummary {
    /// 累加一条 lane 的事件计数（设备参数中 Pitch Bend 单独归类）。
    fn add_lane(&mut self, lane: &AutomationLane) {
        let n = lane.events.len();
        match &lane.target {
            AutomationTarget::CC { controller } => {
                self.cc_total += n;
                match self
                    .cc_per_controller
                    .iter_mut()
                    .find(|(c, _)| c == controller)
                {
                    Some((_, count)) => *count += n,
                    None => self.cc_per_controller.push((*controller, n)),
                }
            }
            AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { .. },
                id,
                ..
            } if *id == xsynth_param::PITCH_BEND => self.pb_total += n,
            AutomationTarget::PitchBend => self.pb_total += n,
            AutomationTarget::Param { .. } => self.param_total += n,
            AutomationTarget::Rpn { .. } | AutomationTarget::Nrpn { .. } => self.rpn_total += n,
            AutomationTarget::Tempo => {}
        }
    }

    /// 合并另一份统计（CC 分组保持首次出现顺序）。
    fn merge(&mut self, other: Self) {
        self.cc_total += other.cc_total;
        for (controller, count) in other.cc_per_controller {
            match self
                .cc_per_controller
                .iter_mut()
                .find(|(c, _)| *c == controller)
            {
                Some((_, total)) => *total += count,
                None => self.cc_per_controller.push((controller, count)),
            }
        }
        self.pb_total += other.pb_total;
        self.param_total += other.param_total;
        self.rpn_total += other.rpn_total;
    }
}

fn summarize_targets(lanes: &[AutomationLane]) -> TargetSummary {
    let mut out = TargetSummary::default();
    for lane in lanes {
        out.add_lane(lane);
    }
    out
}

pub(super) fn show_overview(ui: &mut egui::Ui, model: &yinhe_core::YinModel) {
    crate::right_panel::region_hint(ui, t!("hint.panel.event_overview"));
    let name = if model.meta.name.is_empty() {
        t!("event_browser.meta.untitled").to_string()
    } else {
        model.meta.name.clone()
    };
    let artist = if model.meta.artist.is_empty() {
        t!("event_browser.meta.unfilled").to_string()
    } else {
        model.meta.artist.clone()
    };
    crate::widgets::rows::value_row(ui, t!("event_browser.meta.name"), name);
    crate::widgets::rows::value_row(ui, t!("event_browser.meta.artist"), artist);
    crate::widgets::rows::value_row(ui, "PPQ:", format!("{}", model.meta.ppq));
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.zstd_level"),
        format!("{}", model.meta.compression_level),
    );
    let groups = super::group_tracks_by_port_channel(model, None);
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.active_ports"),
        format!("{}", groups.len()),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.tracks"),
        t!("event_browser.count", n = model.tracks.len()),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.notes"),
        t!("event_browser.count", n = model.note_count),
    );
    let mut summary = TargetSummary::default();
    let mut pc = 0usize;
    for t in &model.tracks {
        summary.merge(summarize_targets(&t.automation_lanes));
        pc += t.program_change.len();
    }
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.cc"),
        t!("event_browser.count", n = summary.cc_total),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.params"),
        t!("event_browser.count", n = summary.param_total),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.pitch_bend"),
        t!("event_browser.count", n = summary.pb_total),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.program_change"),
        t!("event_browser.count", n = pc),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.tempo"),
        t!(
            "event_browser.count",
            n = model.conductor.tempo.events.len()
        ),
    );
    crate::widgets::rows::value_row(
        ui,
        t!("event_browser.meta.timesig"),
        t!("event_browser.count", n = model.conductor.time_sig.len()),
    );
    if !model.conductor.key_sig.is_empty() {
        crate::widgets::rows::value_row(
            ui,
            t!("event_browser.meta.keysig"),
            t!("event_browser.count", n = model.conductor.key_sig.len()),
        );
    }
    if !model.conductor.markers.is_empty() {
        crate::widgets::rows::value_row(
            ui,
            t!("event_browser.meta.markers"),
            t!("event_browser.count", n = model.conductor.markers.len()),
        );
    }
}

pub(super) fn show_track_detail(
    ui: &mut egui::Ui,
    idx: u16,
    track: &yinhe_core::TrackData,
    model: &yinhe_core::YinModel,
) {
    crate::right_panel::region_hint(ui, t!("hint.panel.event_track_detail"));
    let header = if track.name.is_empty() {
        t!("event_browser.track_unnamed", n = idx).to_string()
    } else {
        t!("event_browser.track_named", n = idx, name = &track.name).to_string()
    };
    crate::widgets::rows::section_header(ui, &header);

    kv(ui, "UUID", track.uuid.clone());
    kv(
        ui,
        "Port / Channel",
        format!("{} / {}", super::port_letter(track.port), track.channel + 1),
    );
    kv(
        ui,
        t!("event_browser.channel_prefix").as_ref(),
        match track.channel_prefix {
            Some(c) => format!("{}", c),
            None => t!("common.none").to_string(),
        },
    );
    kv(
        ui,
        t!("event_browser.color").as_ref(),
        format!(
            "[{:.2}, {:.2}, {:.2}]",
            track.color[0], track.color[1], track.color[2]
        ),
    );
    kv(
        ui,
        t!("event_browser.muted_soloed").as_ref(),
        format!("{} / {}", track.muted, track.soloed),
    );
    ui.add_space(crate::theme::GAP_SM);
    ui.label(
        egui::RichText::new(t!("event_browser.event_counts"))
            .strong()
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::BODY_FONT,
            ))
            .color(crate::theme::text_bright()),
    );
    kv(
        ui,
        "Notes",
        format!(
            "{}",
            model
                .track_note_count
                .get(idx as usize)
                .copied()
                .unwrap_or(0)
        ),
    );
    // 按 automation target 类型汇总（与 overview 共用同一分类逻辑）
    let summary = summarize_targets(&track.automation_lanes);
    if !summary.cc_per_controller.is_empty() {
        kv(
            ui,
            "CC",
            t!(
                "event_browser.cc_summary",
                controllers = summary.cc_per_controller.len(),
                events = summary.cc_total
            )
            .to_string(),
        );
        for (ctrl, count) in &summary.cc_per_controller {
            kv(
                ui,
                &format!(
                    "  {}",
                    AutomationTarget::CC { controller: *ctrl }.display_name()
                ),
                t!("event_browser.cc_count", n = count).to_string(),
            );
        }
    }
    kv(ui, "Pitch Bend", format!("{}", summary.pb_total));
    if summary.param_total > 0 {
        kv(
            ui,
            t!("event_browser.params").as_ref(),
            format!("{}", summary.param_total),
        );
    }
    kv(
        ui,
        "Program Change",
        format!("{}", track.program_change.len()),
    );
    if summary.rpn_total > 0 {
        kv(
            ui,
            "RPN/NRPN",
            t!("event_browser.rpn_summary", n = summary.rpn_total).to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::table::EVENT_PAGE_SIZE;
    use super::*;
    use yinhe_types::SegmentShape;
    use yinhe_types::automation::AutomationEvent;

    fn lane(target: AutomationTarget, count: usize) -> AutomationLane {
        AutomationLane {
            target,
            track: 0,
            events: (0..count)
                .map(|i| AutomationEvent {
                    id: i as u32,
                    tick: i as u32 * 10,
                    value: 0.5,
                    shape: SegmentShape::Step,
                })
                .collect(),
        }
    }

    #[test]
    fn total_pages_rounds_up_and_is_at_least_one() {
        assert_eq!(total_pages(0), 1);
        assert_eq!(total_pages(1), 1);
        assert_eq!(total_pages(EVENT_PAGE_SIZE), 1);
        assert_eq!(total_pages(EVENT_PAGE_SIZE + 1), 2);
    }

    #[test]
    fn paginate_clamps_out_of_range_page() {
        let mut state = EventBrowserState::default();
        let items: Vec<u32> = (0..(EVENT_PAGE_SIZE * 3)).map(|i| i as u32).collect();
        state.event_page = 99;
        let (page, start, slice) = paginate(&mut state, &items);
        assert_eq!(page, 2);
        assert_eq!(state.event_page, 2);
        assert_eq!(start, EVENT_PAGE_SIZE * 2);
        assert_eq!(slice.len(), EVENT_PAGE_SIZE);
        assert_eq!(slice[0], (EVENT_PAGE_SIZE * 2) as u32);
    }

    #[test]
    fn paginate_slices_tail_page() {
        let mut state = EventBrowserState::default();
        let items: Vec<u32> = (0..(EVENT_PAGE_SIZE + 7)).map(|i| i as u32).collect();
        state.event_page = 1;
        let (page, start, slice) = paginate(&mut state, &items);
        assert_eq!(page, 1);
        assert_eq!(start, EVENT_PAGE_SIZE);
        assert_eq!(slice.len(), 7);
        assert_eq!(slice[6], (EVENT_PAGE_SIZE + 6) as u32);
    }

    #[test]
    fn paginate_empty_list_is_first_page() {
        let mut state = EventBrowserState::default();
        let items: Vec<u32> = Vec::new();
        let (page, start, slice) = paginate(&mut state, &items);
        assert_eq!(page, 0);
        assert_eq!(start, 0);
        assert!(slice.is_empty());
    }

    #[test]
    fn summarize_targets_classifies_and_preserves_cc_order() {
        let lanes = vec![
            lane(AutomationTarget::CC { controller: 1 }, 2),
            lane(AutomationTarget::CC { controller: 7 }, 3),
            lane(AutomationTarget::CC { controller: 1 }, 4),
            lane(
                AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::PITCH_BEND,
                    name: String::new(),
                },
                5,
            ),
            lane(
                AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: 42,
                    name: "Cutoff".into(),
                },
                6,
            ),
            lane(AutomationTarget::Rpn { parameter: 0 }, 7),
            lane(AutomationTarget::Nrpn { parameter: 1 }, 8),
            lane(AutomationTarget::Tempo, 9),
        ];
        let s = summarize_targets(&lanes);
        assert_eq!(s.cc_total, 9);
        assert_eq!(s.cc_per_controller, vec![(1, 6), (7, 3)]);
        assert_eq!(s.pb_total, 5);
        assert_eq!(s.param_total, 6);
        assert_eq!(s.rpn_total, 15);
    }

    #[test]
    fn target_summary_merge_accumulates_across_tracks() {
        let mut total = summarize_targets(&[lane(AutomationTarget::CC { controller: 1 }, 2)]);
        total.merge(summarize_targets(&[
            lane(AutomationTarget::CC { controller: 1 }, 3),
            lane(AutomationTarget::CC { controller: 7 }, 4),
        ]));
        assert_eq!(total.cc_total, 9);
        assert_eq!(total.cc_per_controller, vec![(1, 5), (7, 4)]);
    }
}
