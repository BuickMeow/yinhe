//! 右侧详情面板：根据 `SelectedItem` 分发到对应的详情视图。
//!
//! `Automation` 统一处理 CC / PitchBend / RPN / NRPN / Tempo，
//! 不再为每种类型写单独的分支。

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::AutomationTarget;

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
            "标记",
            TextEventKind::Marker,
        ),
        SelectedItem::ConductorLyrics => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_cond_lyrics",
            "歌词",
            TextEventKind::ConductorLyrics,
        ),
        SelectedItem::ConductorChord => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_cond_chord",
            "和弦",
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
            "歌词",
            TextEventKind::Lyrics { track: *track },
        ),
        SelectedItem::Chord { track } => show_text_events_detail(
            ui,
            doc,
            bar_lookup,
            state,
            "eb_chord",
            "和弦",
            TextEventKind::Chord { track: *track },
        ),
    }
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
    // 先 clone 出 owned 数据，避免不可变借用阻塞后续 &mut doc 编辑
    let (lane_idx, mut events): (usize, Vec<AutomationEventOwned>) =
        if matches!(target, AutomationTarget::Tempo) {
            let evts = doc
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
            (0usize, evts)
        } else {
            let mut lane_idx = 0usize;
            let mut evts = Vec::new();
            if let Some(td) = doc.data.model.tracks.get(track as usize) {
                for (li, lane) in td.automation_lanes.iter().enumerate() {
                    if &lane.target == target {
                        lane_idx = li;
                        evts.extend(lane.events.iter().map(|e| AutomationEventOwned {
                            tick: e.tick,
                            value: e.value,
                            shape: e.shape,
                        }));
                        break;
                    }
                }
            }
            (lane_idx, evts)
        };
    events.sort_by_key(|e| e.tick);

    let (page, page_start, page_items) = paginate(state, &events);
    let total = events.len();
    let title = format!("{} {} 个", target.display_name(), total);

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(12.0).strong());
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(2.0);

    if total == 0 {
        empty_state_add_button(ui, "eb_auto_edit");
    } else {
        let page_ticks: Vec<u32> = page_items.iter().map(|e| e.tick).collect();
        build_table(
            ui,
            "eb_auto",
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
            page_items.len(),
            |i, row, click_key| {
                let e = &page_items[i];
                cell_row_header(
                    row,
                    state,
                    "eb_auto_edit",
                    i,
                    page_start,
                    e.tick,
                    &page_ticks,
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
                    format!("{}", e.value),
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
        handle_delete_key(ui, "eb_auto_edit", !state.selected_ticks.is_empty());
    }

    apply_automation_popups(ui, doc, "eb_auto_edit", track, lane_idx, target, bar_lookup);
    apply_automation_ops(ui, doc, state, "eb_auto_edit", track, target);

    // Automation：仅跳转不闪烁；Tempo 不切 track（note=None）
    let note = if matches!(target, AutomationTarget::Tempo) {
        None
    } else {
        Some((track, 0))
    };
    take_row_click(ui, "eb_auto").map(|i| JumpRequest {
        tick: page_items[i].tick,
        note,
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
    let (page, page_start, page_items) = paginate(state, &sorted);
    let total = sorted.len();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("拍号 {} 个", total))
                .size(12.0)
                .strong(),
        );
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(2.0);
    if total == 0 {
        empty_state_add_button(ui, "eb_ts_edit");
    } else {
        let page_ticks: Vec<u32> = page_items.iter().map(|e| e.tick).collect();
        build_table(
            ui,
            "eb_ts",
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                ("分子", 50.0),
                ("分母", 50.0),
            ],
            page_items.len(),
            |i, row, click_key| {
                let e = &page_items[i];
                let denom = 1u32 << e.denominator as u32;
                cell_row_header(
                    row,
                    state,
                    "eb_ts_edit",
                    i,
                    page_start,
                    e.tick,
                    &page_ticks,
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
        handle_delete_key(ui, "eb_ts_edit", !state.selected_ticks.is_empty());
    }
    apply_timesig_popups(ui, doc, "eb_ts_edit", bar_lookup);
    apply_timesig_ops(ui, doc, state, "eb_ts_edit");
    take_row_click(ui, "eb_ts").map(|i| JumpRequest {
        tick: page_items[i].tick,
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
    let (page, page_start, page_items) = paginate(state, &sorted);
    let total = sorted.len();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("调号 {} 个", total))
                .size(12.0)
                .strong(),
        );
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(2.0);
    if total == 0 {
        empty_state_add_button(ui, "eb_ks_edit");
    } else {
        let page_ticks: Vec<u32> = page_items.iter().map(|e| e.tick).collect();
        build_table(
            ui,
            "eb_ks",
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                ("调号", 100.0),
                ("根音", 60.0),
                ("音阶", 80.0),
            ],
            page_items.len(),
            |i, row, click_key| {
                let e = &page_items[i];
                cell_row_header(
                    row,
                    state,
                    "eb_ks_edit",
                    i,
                    page_start,
                    e.tick,
                    &page_ticks,
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
        handle_delete_key(ui, "eb_ks_edit", !state.selected_ticks.is_empty());
    }
    apply_keysig_popups(ui, doc, "eb_ks_edit", bar_lookup);
    apply_keysig_ops(ui, doc, state, "eb_ks_edit");
    take_row_click(ui, "eb_ks").map(|i| JumpRequest {
        tick: page_items[i].tick,
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
    // 先 clone 出 owned 数据，避免不可变借用阻塞后续 &mut doc 编辑
    let mut sorted: Vec<(u32, String)> = match kind {
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
    sorted.sort_by_key(|e| e.0);
    let (page, page_start, page_items) = paginate(state, &sorted);
    let total = sorted.len();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {} 个", label, total))
                .size(12.0)
                .strong(),
        );
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(2.0);
    let edit_salt = format!("{}_edit", table_id);
    if total == 0 {
        empty_state_add_button(ui, &edit_salt);
    } else {
        let page_ticks: Vec<u32> = page_items.iter().map(|(t, _)| *t).collect();
        build_table(
            ui,
            table_id,
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                (label, 200.0),
            ],
            page_items.len(),
            |i, row, click_key| {
                let (tick, text) = &page_items[i];
                cell_row_header(
                    row,
                    state,
                    &edit_salt,
                    i,
                    page_start,
                    *tick,
                    &page_ticks,
                    click_key,
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
        handle_delete_key(ui, &edit_salt, !state.selected_ticks.is_empty());
    }
    apply_text_popups(ui, doc, &edit_salt, bar_lookup);
    // 分派删除/插入操作
    match kind {
        TextEventKind::Marker => apply_marker_ops(ui, doc, state, &edit_salt),
        TextEventKind::ConductorLyrics => apply_conductor_lyrics_ops(ui, doc, state, &edit_salt),
        TextEventKind::ConductorChord => apply_conductor_chord_ops(ui, doc, state, &edit_salt),
        TextEventKind::Lyrics { track } => apply_lyrics_ops(ui, doc, state, &edit_salt, track),
        TextEventKind::Chord { track } => apply_chord_ops(ui, doc, state, &edit_salt, track),
    }
    take_row_click(ui, table_id).map(|i| JumpRequest {
        tick: page_items[i].0,
        note: None,
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
    let model = &doc.data.model;
    let track_count = model
        .track_note_count
        .get(track as usize)
        .copied()
        .unwrap_or(0) as usize;
    let mut notes: Vec<(yinhe_core::NoteEvent, u8, u16)> = Vec::with_capacity(track_count);
    for (key, bucket) in model.notes.iter().enumerate() {
        if !model.bucket_track_stats[key].contains_key(&track) {
            continue;
        }
        for n in bucket.iter().filter(|n| n.track == track) {
            notes.push((
                yinhe_core::NoteEvent {
                    id: n.id,
                    start_tick: n.start_tick,
                    end_tick: n.end_tick,
                    key: key as u8,
                    velocity: n.velocity,
                },
                key as u8,
                track,
            ));
        }
    }
    notes.sort_by_key(|(n, _, _)| n.start_tick);
    let (page, page_start, page_notes) = paginate(state, &notes);
    let total = notes.len();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("音符 {} 个", total))
                .size(12.0)
                .strong(),
        );
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(2.0);
    if total == 0 {
        empty_state_add_button(ui, "eb_notes_edit");
    } else {
        let page_ticks: Vec<u32> = page_notes.iter().map(|(n, _, _)| n.start_tick).collect();
        build_table(
            ui,
            "eb_notes",
            &[
                ("#", 40.0),
                ("id", 70.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                ("gate", 60.0),
                (t!("event_browser.header.end_tick").as_ref(), 80.0),
                ("结束位置", 90.0),
                ("键位", 50.0),
                ("力度", 50.0),
            ],
            page_notes.len(),
            |i, row, click_key| {
                let (n, _key, _trk) = &page_notes[i];
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
                    &page_ticks,
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
        handle_delete_key(ui, "eb_notes_edit", !state.selected_ticks.is_empty());
    }
    apply_note_popups(ui, doc, "eb_notes_edit", bar_lookup);
    apply_notes_ops(ui, doc, state, "eb_notes_edit", track);
    // 音符：切到音符所在 track
    take_row_click(ui, "eb_notes").map(|i| {
        let (n, _key, _trk) = &page_notes[i];
        JumpRequest {
            tick: n.start_tick,
            note: Some((track, n.key)),
        }
    })
}

// ── Program Change ──

fn show_pc_detail(
    ui: &mut egui::Ui,
    doc: &mut Document,
    bar_lookup: &BarLookup,
    state: &mut EventBrowserState,
    track: u16,
) -> Option<JumpRequest> {
    let t = track as usize;
    let mut events: Vec<yinhe_types::PcEvent> = doc
        .data
        .model
        .tracks
        .get(t)
        .map(|td| td.program_change.to_vec())
        .unwrap_or_default();
    events.sort_by_key(|e| e.tick);
    let (page, page_start, page_items) = paginate(state, &events);
    let total = events.len();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("音色变更 {} 个", total))
                .size(12.0)
                .strong(),
        );
        if let Some(np) = render_pager(ui, page, total_pages(total)) {
            state.event_page = np;
        }
    });
    ui.add_space(2.0);
    if total == 0 {
        empty_state_add_button(ui, "eb_pc_edit");
    } else {
        let page_ticks: Vec<u32> = page_items.iter().map(|e| e.tick).collect();
        build_table(
            ui,
            "eb_pc",
            &[
                ("#", 40.0),
                (t!("event_browser.header.tick").as_ref(), 70.0),
                (t!("event_browser.header.position").as_ref(), 80.0),
                ("音色", 50.0),
            ],
            page_items.len(),
            |i, row, click_key| {
                let e = page_items[i];
                cell_row_header(
                    row,
                    state,
                    "eb_pc_edit",
                    i,
                    page_start,
                    e.tick,
                    &page_ticks,
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
        handle_delete_key(ui, "eb_pc_edit", !state.selected_ticks.is_empty());
    }
    apply_pc_popups(ui, doc, "eb_pc_edit", track, bar_lookup);
    apply_pc_ops(ui, doc, state, "eb_pc_edit", track);
    // PC：切到所在 track，仅跳转不闪烁
    take_row_click(ui, "eb_pc").map(|i| JumpRequest {
        tick: page_items[i].tick,
        note: Some((track, 0)),
    })
}

// ── project.json / mapping.json ──

fn show_project_json(ui: &mut egui::Ui, doc: &Document) {
    let pf = &doc.data.project_file;
    ui.add_space(4.0);
    ui.label(egui::RichText::new("project.json").size(13.0).strong());
    ui.add_space(6.0);

    let kv = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(k).size(11.0).color(egui::Color32::GRAY));
            ui.label(
                egui::RichText::new(v)
                    .size(11.0)
                    .monospace()
                    .color(egui::Color32::from_gray(220)),
            );
        });
    };

    kv(ui, "version", format!("{}", pf.version));
    kv(ui, "name", pf.name.clone());
    kv(ui, "artist", pf.artist.clone());
    kv(ui, "description", pf.description.clone());
    kv(ui, "ppq", format!("{}", pf.ppq));
    kv(ui, "compression_level", format!("{}", pf.compression_level));
    kv(
        ui,
        "soundfont_project_mode",
        format!("{}", pf.soundfont_project_mode),
    );

    if !pf.soundfont_overrides.is_empty() {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("soundfont_overrides")
                .size(11.0)
                .strong(),
        );
        for po in &pf.soundfont_overrides {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!("port {}:", po.port))
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                );
            });
            for entry in &po.entries {
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    let status = if entry.enabled {
                        "\u{2705}"
                    } else {
                        "\u{274c}"
                    };
                    ui.label(
                        egui::RichText::new(format!("{} {} ({})", status, entry.name, entry.path))
                            .size(10.0)
                            .monospace()
                            .color(egui::Color32::from_gray(180)),
                    );
                });
            }
        }
    }
}

fn show_mapping_json(ui: &mut egui::Ui, doc: &Document) {
    let mf = &doc.data.mapping_file;
    ui.add_space(4.0);
    ui.label(egui::RichText::new("mapping.json").size(13.0).strong());
    ui.add_space(6.0);

    let kv = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(k).size(11.0).color(egui::Color32::GRAY));
            ui.label(
                egui::RichText::new(v)
                    .size(11.0)
                    .monospace()
                    .color(egui::Color32::from_gray(220)),
            );
        });
    };

    kv(ui, "version", format!("{}", mf.version));

    ui.add_space(6.0);
    ui.label(egui::RichText::new("ports").size(11.0).strong());
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

    if !mf.soundfonts.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("soundfonts").size(11.0).strong());
        for (port, paths) in &mf.soundfonts {
            kv(ui, &format!("port {}", port), paths.join(", "));
        }
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("view").size(11.0).strong());
    kv(ui, "zoom_x", format!("{:.2}", mf.view.zoom_x));
    kv(ui, "zoom_y", format!("{:.2}", mf.view.zoom_y));
    kv(ui, "scroll_tick", format!("{}", mf.view.scroll_tick));
    kv(ui, "scroll_key", format!("{}", mf.view.scroll_key));
    if let Some(ref uuid) = mf.view.active_track_uuid {
        kv(ui, "active_track_uuid", uuid.clone());
    }
}

// ── Overview / Track detail ──

pub(super) fn show_overview(ui: &mut egui::Ui, model: &yinhe_core::YinModel) {
    ui.label(egui::RichText::new("工程概览").size(14.0).strong());
    ui.add_space(4.0);
    let name = if model.meta.name.is_empty() {
        "(未命名)"
    } else {
        &model.meta.name
    };
    let artist = if model.meta.artist.is_empty() {
        "(未填)"
    } else {
        &model.meta.artist
    };
    ui.colored_label(egui::Color32::from_gray(120), format!("名称: {}", name));
    ui.colored_label(egui::Color32::from_gray(120), format!("作者: {}", artist));
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("PPQ: {}", model.meta.ppq),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("zstd 等级: {}", model.meta.compression_level),
    );
    let groups = super::group_tracks_by_port_channel(model, None);
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("活跃 port 数: {}", groups.len()),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("轨道: {} 个", model.tracks.len()),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("音符: {} 个", model.note_count),
    );
    let mut cc = 0usize;
    let mut pb = 0usize;
    let mut pc = 0usize;
    for t in &model.tracks {
        for lane in &t.automation_lanes {
            match &lane.target {
                AutomationTarget::CC { .. } => cc += lane.events.len(),
                AutomationTarget::PitchBend => pb += lane.events.len(),
                _ => {}
            }
        }
        pc += t.program_change.len();
    }
    ui.colored_label(egui::Color32::from_gray(120), format!("CC: {} 个", cc));
    ui.colored_label(egui::Color32::from_gray(120), format!("弯音: {} 个", pb));
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("音色变更: {} 个", pc),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("Tempo: {} 个", model.conductor.tempo.events.len()),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("拍号: {} 个", model.conductor.time_sig.len()),
    );
    if !model.conductor.key_sig.is_empty() {
        ui.colored_label(
            egui::Color32::from_gray(120),
            format!("调号: {} 个", model.conductor.key_sig.len()),
        );
    }
    if !model.conductor.markers.is_empty() {
        ui.colored_label(
            egui::Color32::from_gray(120),
            format!("标记: {} 个", model.conductor.markers.len()),
        );
    }
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧条目查看详情");
}

pub(super) fn show_track_detail(
    ui: &mut egui::Ui,
    idx: u16,
    track: &yinhe_core::TrackData,
    model: &yinhe_core::YinModel,
) {
    ui.add_space(4.0);
    let header = if track.name.is_empty() {
        t!("event_browser.track_unnamed", n = idx).to_string()
    } else {
        t!("event_browser.track_named", n = idx, name = &track.name).to_string()
    };
    ui.label(egui::RichText::new(header).size(13.0).strong());
    ui.add_space(4.0);

    let kv = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(k).size(11.0).color(egui::Color32::GRAY));
            ui.label(
                egui::RichText::new(v)
                    .size(11.0)
                    .monospace()
                    .color(egui::Color32::from_gray(220)),
            );
        });
    };

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
    ui.add_space(6.0);
    ui.label(egui::RichText::new("事件计数").size(12.0).strong());
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
    // 按 automation target 类型汇总
    let mut cc_total = 0usize;
    let mut cc_controllers: Vec<u8> = Vec::new();
    let mut cc_counts: Vec<usize> = Vec::new();
    let mut pb_total = 0usize;
    let mut rpn_total = 0usize;
    for lane in &track.automation_lanes {
        match &lane.target {
            AutomationTarget::CC { controller } => {
                cc_total += lane.events.len();
                if let Some(pos) = cc_controllers.iter().position(|c| c == controller) {
                    cc_counts[pos] += lane.events.len();
                } else {
                    cc_controllers.push(*controller);
                    cc_counts.push(lane.events.len());
                }
            }
            AutomationTarget::PitchBend => pb_total += lane.events.len(),
            AutomationTarget::Rpn { .. } | AutomationTarget::Nrpn { .. } => {
                rpn_total += lane.events.len()
            }
            AutomationTarget::Tempo => {}
        }
    }
    if !cc_controllers.is_empty() {
        kv(
            ui,
            "CC",
            t!(
                "event_browser.cc_summary",
                controllers = cc_controllers.len(),
                events = cc_total
            )
            .to_string(),
        );
        for (i, ctrl) in cc_controllers.iter().enumerate() {
            kv(
                ui,
                &format!("  CC {} {}", ctrl, super::cc_label(*ctrl)),
                t!("event_browser.cc_count", n = cc_counts[i]).to_string(),
            );
        }
    }
    kv(ui, "Pitch Bend", format!("{}", pb_total));
    kv(
        ui,
        "Program Change",
        format!("{}", track.program_change.len()),
    );
    if rpn_total > 0 {
        kv(
            ui,
            "RPN/NRPN",
            t!("event_browser.rpn_summary", n = rpn_total).to_string(),
        );
    }
}
