use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_material_icons::icons::*;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::AutomationTarget;
use crate::widgets::split_handle;
use crate::theme;

// ── Jump request (event browser → App → piano_view) ──

/// 事件浏览器表格行点击时产生的跳转请求。
///
/// Step 1：音符/TimeSig 携带 PulseKind，App 据此启动闪烁动画；
/// automation 类（CC/PB/PC/Tempo）只跳转不闪烁（PulseKind = None）。
#[derive(Clone, Debug)]
pub struct JumpRequest {
    pub tick: u32,
    /// 音符事件：Some((track, key))；其他事件：None。
    pub note: Option<(u16, u8)>,
    /// 闪烁高亮类型：None = 仅跳转不闪烁；Some = Step 1 支持的闪烁形状。
    pub pulse: Option<PulseKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PulseKind {
    /// 音符矩形闪烁（piano roll 内画白色描边矩形）
    NoteRect,
    /// TimeSig 竖线闪烁（贯穿 piano roll 高度的白色竖线）
    TimesigLine,
}

// ── State types ──

pub struct EventBrowserState {
    pub expanded_keys: std::collections::HashSet<ArchiveKey>,
    pub selected_item: Option<SelectedItem>,
    pub selected_track: Option<u16>,
    /// 事件列表当前页码（0-based）。切换 selected_item 时重置为 0。
    pub event_page: usize,
    fingerprint: Option<u64>,
    split_ratio: f32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SelectedItem {
    ProjectJson,
    MappingJson,
    Tempo,
    TimeSig,
    Notes { track: u16 },
    Cc { track: u16, controller: u8 },
    PitchBend { track: u16 },
    ProgramChange { track: u16 },
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArchiveKey {
    Conductor,
    Port(u8),
    Channel(u8, u8),
    Track(u16),
}

impl Default for EventBrowserState {
    fn default() -> Self {
        Self {
            expanded_keys: Default::default(),
            selected_item: None,
            selected_track: None,
            event_page: 0,
            fingerprint: None,
            split_ratio: 0.45,
        }
    }
}

// ── Bar lookup ──

struct BarLookup {
    segs: Vec<BarSeg>,
}

struct BarSeg {
    tick_start: u32,
    bar_start: u32,
    ticks_per_bar: u32,
}

impl BarLookup {
    fn build(ppq: u32, default_num: u8, ts_changes: &[(u32, u8)]) -> Self {
        let mut points: Vec<(u32, u8)> = Vec::new();
        if ts_changes.first().map(|e| e.0).unwrap_or(u32::MAX) != 0 {
            points.push((0, default_num));
        }
        for &(tick, num) in ts_changes {
            points.push((tick, num));
        }
        let mut segs = Vec::with_capacity(points.len());
        let mut cum_bars: u32 = 0;
        for (i, &(tick, num)) in points.iter().enumerate() {
            let ticks_per_bar = ppq.saturating_mul(num.max(1) as u32);
            segs.push(BarSeg {
                tick_start: tick,
                bar_start: cum_bars,
                ticks_per_bar,
            });
            if let Some(&(next_tick, _)) = points.get(i + 1) {
                let span = next_tick.saturating_sub(tick);
                cum_bars = cum_bars.saturating_add(span / ticks_per_bar.max(1));
            }
        }
        if segs.is_empty() {
            segs.push(BarSeg {
                tick_start: 0,
                bar_start: 0,
                ticks_per_bar: ppq.saturating_mul(4),
            });
        }
        BarLookup { segs }
    }

    fn format(&self, tick: u32) -> String {
        if self.segs.is_empty() {
            return "?".into();
        }
        let idx = match self.segs.binary_search_by_key(&tick, |s| s.tick_start) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let seg = &self.segs[idx];
        let local = tick.saturating_sub(seg.tick_start);
        let tpb = seg.ticks_per_bar.max(1);
        let bar_offset = local / tpb;
        let tick_in_bar = local % tpb;
        let bar_1 = seg.bar_start + bar_offset + 1;
        format!("{}/{}", bar_1, tick_in_bar)
    }
}

fn ts_changes(model: &yinhe_core::YinModel) -> Vec<(u32, u8)> {
    model
        .conductor
        .time_sig
        .iter()
        .map(|e| (e.tick, e.numerator))
        .collect()
}

// ═══════════════════════════════════════════════════════════════
//  Main entry — unified view
// ═══════════════════════════════════════════════════════════════

pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>, state: &mut EventBrowserState) -> Option<JumpRequest> {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("\u{ff08}\u{672a}\u{6253}\u{5f00}\u{6587}\u{6863}\u{ff09}")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return None;
    };

    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let default_num = model.tempo_map.time_sig_default.0;
    let ts = ts_changes(model);
    let bar_lookup = BarLookup::build(ppq, default_num, &ts);

    let fingerprint = doc.data.revision;
    if state.fingerprint != Some(fingerprint) {
        if state.fingerprint.is_none() {
            state.expanded_keys.clear();
            for t in &model.tracks {
                state.expanded_keys.insert(ArchiveKey::Port(t.port));
            }
        }
        if let Some(idx) = state.selected_track {
            if idx as usize >= model.tracks.len() {
                state.selected_track = None;
            }
        }
        state.fingerprint = Some(fingerprint);
    }

    let frame_bg = egui::Frame::NONE
        .fill(theme::APP_BG)
        .inner_margin(egui::Margin::symmetric(4, 2));

    let total_rect = ui.available_rect_before_wrap();
    let total_h = total_rect.height();
    let gap = theme::SPLIT_GAP;
    let split_y = total_rect.min.y + (total_h * state.split_ratio).round();

    let top_rect = egui::Rect::from_min_max(total_rect.min, egui::pos2(total_rect.max.x, split_y));
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(total_rect.min.x, split_y),
        egui::pos2(total_rect.max.x, split_y + gap),
    );
    let bot_rect = egui::Rect::from_min_max(
        egui::pos2(total_rect.min.x, split_y + gap),
        total_rect.max,
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(top_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("eb_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.vertical(|ui| render_tree(ui, doc, state));
                });
            });
    });

    let resp = split_handle::horizontal(ui, "__eb_split__", handle_rect);
    if resp.dragged() {
        let new_ratio = ((split_y + resp.drag_delta().y - total_rect.min.y) / total_h)
            .clamp(theme::SPLIT_CLAMP_MIN, theme::SPLIT_CLAMP_MAX);
        state.split_ratio = new_ratio;
    }

    let mut jump_request: Option<JumpRequest> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(bot_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("eb_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    // 先 clone selected_item，避免在调用 show_event_detail 时
                    // 同时持有 state 的不可变借用和可变借用
                    let sel = state.selected_item.clone();
                    let track_idx = state.selected_track;
                    if let Some(ref sel) = sel {
                        jump_request = show_event_detail(ui, sel, doc, &bar_lookup, state);
                    } else if let Some(idx) = track_idx {
                        if let Some(track) = model.tracks.get(idx as usize) {
                            show_track_detail(ui, idx, track, model);
                        } else {
                            show_overview(ui, model);
                        }
                    } else {
                        show_overview(ui, model);
                    }
                });
            });
    });
    jump_request
}

// ═══════════════════════════════════════════════════════════════
//  Unified tree
// ═══════════════════════════════════════════════════════════════

fn render_tree(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &mut EventBrowserState,
) {
    let model = &doc.data.model;
    let conductor_idx = doc.edit.conductor_track_idx;
    let groups = group_tracks_by_port_channel(model, conductor_idx);

    render_leaf_item(ui, "project.json", ICON_DESCRIPTION, 0, SelectedItem::ProjectJson, state);
    render_leaf_item(ui, "mapping.json", ICON_DESCRIPTION, 0, SelectedItem::MappingJson, state);

    let has_tempo = !model.conductor.tempo.events.is_empty();
    let has_ts = !model.conductor.time_sig.is_empty();
    if has_tempo || has_ts {
        let cond_expanded = state.expanded_keys.contains(&ArchiveKey::Conductor);
        let child_count = has_tempo as usize + has_ts as usize;
        if render_dir_row(ui, "Conductor", 0, cond_expanded, child_count) {
            toggle_key(state, ArchiveKey::Conductor);
        }
        if cond_expanded {
            if has_tempo {
                render_leaf_item(ui, &format!("Tempo ({})", model.conductor.tempo.events.len()), ICON_SPEED, 1, SelectedItem::Tempo, state);
            }
            if has_ts {
                render_leaf_item(ui, &format!("TimeSig ({})", model.conductor.time_sig.len()), ICON_SCHEDULE, 1, SelectedItem::TimeSig, state);
            }
        }
    }

    for (&port, channels) in &groups {
        let port_key = ArchiveKey::Port(port);
        let port_expanded = state.expanded_keys.contains(&port_key);
        let port_track_count: usize = channels.values().map(|v| v.len()).sum();
        let port_label = t!("event_browser.port_tracks", port = port_letter(port), n = port_track_count).to_string();
        if render_dir_row(ui, &port_label, 0, port_expanded, channels.len()) {
            toggle_key(state, port_key);
        }
        if !port_expanded {
            continue;
        }

        for (&channel, track_indices) in channels {
            let ch_key = ArchiveKey::Channel(port, channel);
            let ch_expanded = state.expanded_keys.contains(&ch_key);
            let ch_label = t!("event_browser.channel_tracks", ch = channel + 1, n = track_indices.len()).to_string();
            if render_dir_row(ui, &ch_label, 1, ch_expanded, track_indices.len()) {
                toggle_key(state, ch_key);
            }
            if !ch_expanded {
                continue;
            }

            for &track_idx in track_indices {
                render_track_row(ui, model, track_idx, state);
            }
        }
    }
}

fn render_track_row(
    ui: &mut egui::Ui,
    model: &yinhe_core::YinModel,
    idx: u16,
    state: &mut EventBrowserState,
) {
    let track = &model.tracks[idx as usize];
    let track_key = ArchiveKey::Track(idx);
    let expanded = state.expanded_keys.contains(&track_key);
    let is_selected = state.selected_track == Some(idx);

    let note_count = *model.track_note_count.get(idx as usize).unwrap_or(&0) as usize;
    // Build CC map and PB count from automation_lanes
    let mut cc_map: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    let mut pb_count: usize = 0;
    for lane in &track.automation_lanes {
        match &lane.target {
            AutomationTarget::CC { controller } => {
                *cc_map.entry(*controller).or_insert(0) += lane.events.len();
            }
            AutomationTarget::PitchBend => {
                pb_count += lane.events.len();
            }
            _ => {}
        }
    }
    let pc_count = track.program_change.len();
    let _child_count = (note_count > 0) as usize
        + cc_map.len()
        + (pb_count > 0) as usize
        + (pc_count > 0) as usize;

    let label_text = if track.name.is_empty() {
        format!("(track #{})", idx)
    } else {
        track.name.clone()
    };
    let summary = format!(
        "{} notes \u{00b7} {} CC \u{00b7} {} PB \u{00b7} {} PC",
        note_count,
        cc_map.values().sum::<usize>(),
        pb_count,
        pc_count,
    );

    let row_bg = if is_selected {
        egui::Color32::from_rgb(40, 50, 70)
    } else {
        egui::Color32::TRANSPARENT
    };

    let mut toggled = false;
    let mut selected = false;

    egui::Frame::NONE
        .fill(row_bg)
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(2.0 * 14.0);

                let chev = if expanded { ICON_EXPAND_MORE } else { ICON_CHEVRON_RIGHT };
                if ui.add(egui::Label::new(chev.rich_text().size(13.0).color(egui::Color32::from_gray(190))).sense(egui::Sense::click())).clicked() {
                    toggled = true;
                }

                ui.label(
                    ICON_AUDIOTRACK
                        .rich_text()
                        .size(12.0)
                        .color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(160) }),
                );

                let name_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&label_text)
                            .size(11.0)
                            .monospace()
                            .color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(220) }),
                    )
                    .sense(egui::Sense::click()),
                );
                if name_resp.clicked() {
                    selected = true;
                }

                ui.label(
                    egui::RichText::new(format!("[{}]", summary))
                        .size(10.0)
                        .color(egui::Color32::from_gray(110)),
                );
            });
        });

    if toggled {
        toggle_key(state, track_key);
    }
    if selected {
        state.selected_track = Some(idx);
        state.selected_item = None;
    }

    if expanded {
        if note_count > 0 {
            render_leaf_item(ui, &format!("Notes ({})", note_count), ICON_MUSIC_NOTE, 3, SelectedItem::Notes { track: idx }, state);
        }
        for (&ctrl, &cnt) in &cc_map {
            render_leaf_item(ui, &format!("CC {} {} ({})", ctrl, cc_label(ctrl), cnt), ICON_SETTINGS, 3, SelectedItem::Cc { track: idx, controller: ctrl }, state);
        }
        if pb_count > 0 {
            render_leaf_item(ui, &format!("Pitch Bend ({})", pb_count), ICON_EDIT_AUDIO, 3, SelectedItem::PitchBend { track: idx }, state);
        }
        if pc_count > 0 {
            render_leaf_item(ui, &format!("Program Change ({})", pc_count), ICON_PALETTE, 3, SelectedItem::ProgramChange { track: idx }, state);
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Detail panels
// ═══════════════════════════════════════════════════════════════

fn show_event_detail(ui: &mut egui::Ui, item: &SelectedItem, doc: &Document, bar_lookup: &BarLookup, state: &mut EventBrowserState) -> Option<JumpRequest> {
    let model = &doc.data.model;
    match item {
        SelectedItem::ProjectJson => {
            show_project_json(ui, doc);
            return None;
        }
        SelectedItem::MappingJson => {
            show_mapping_json(ui, doc);
            return None;
        }
        SelectedItem::Tempo => {
            let mut sorted: Vec<&yinhe_types::AutomationEvent> = model.conductor.tempo.events.iter().collect();
            sorted.sort_by_key(|e| e.tick);
            let (page, page_start, page_items) = paginate(state, &sorted);
            let total = sorted.len();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Tempo {} 个", total)).size(12.0).strong());
                if let Some(np) = render_pager(ui, page, total_pages(total)) {
                    state.event_page = np;
                }
            });
            ui.add_space(2.0);
            build_table(ui, "eb_tempo", &[("#", 40.0), (t!("event_browser.header.tick").as_ref(), 70.0), (t!("event_browser.header.position").as_ref(), 80.0), ("BPM", 70.0)], page_items.len(), |i, row| {
                let s = page_items[i];
                cell_text(row, format!("{}", page_start + i + 1));
                cell_text(row, format!("{}", s.tick));
                cell_text(row, bar_lookup.format(s.tick as u32));
                cell_text(row, format!("{:.2}", s.value));
            });
            // Tempo：仅跳转，不闪烁（Step 2 再做 automation panel 圆点闪烁）
            return take_row_click(ui, "eb_tempo").map(|i| JumpRequest {
                tick: page_items[i].tick,
                note: None,
                pulse: None,
            });
        }
        SelectedItem::TimeSig => {
            let mut sorted: Vec<&yinhe_types::TimeSigEvent> = model.conductor.time_sig.iter().collect();
            sorted.sort_by_key(|e| e.tick);
            let (page, page_start, page_items) = paginate(state, &sorted);
            let total = sorted.len();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("拍号 {} 个", total)).size(12.0).strong());
                if let Some(np) = render_pager(ui, page, total_pages(total)) {
                    state.event_page = np;
                }
            });
            ui.add_space(2.0);
            build_table(ui, "eb_ts", &[("#", 40.0), (t!("event_browser.header.tick").as_ref(), 70.0), (t!("event_browser.header.position").as_ref(), 80.0), ("拍号", 80.0)], page_items.len(), |i, row| {
                let e = page_items[i];
                let denom = 1u32 << e.denominator as u32;
                cell_text(row, format!("{}", page_start + i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}/{}", e.numerator, denom));
            });
            // TimeSig：竖线闪烁，无需跳转音轨
            return take_row_click(ui, "eb_ts").map(|i| JumpRequest {
                tick: page_items[i].tick,
                note: None,
                pulse: Some(PulseKind::TimesigLine),
            });
        }
        SelectedItem::Notes { track } => {
            let model = &doc.data.model;
            // 优化：用 bucket_track_stats 跳过没有该 track 音符的 bucket，
            // 避免全量扫描 128 个桶。预分配容量减少 realloc。
            let track_count = model.track_note_count
                .get(*track as usize)
                .copied()
                .unwrap_or(0) as usize;
            let mut notes: Vec<(yinhe_core::NoteEvent, u8, u16)> = Vec::with_capacity(track_count);
            for (key, bucket) in model.notes.iter().enumerate() {
                if !model.bucket_track_stats[key].contains_key(track) {
                    continue;
                }
                for n in bucket.iter().filter(|n| n.track == *track) {
                    notes.push((
                        yinhe_core::NoteEvent {
                            id: n.id,
                            start_tick: n.start_tick,
                            end_tick: n.end_tick,
                            key: key as u8,
                            velocity: n.velocity,
                        },
                        key as u8,
                        *track,
                    ));
                }
            }
            notes.sort_by_key(|(n, _, _)| n.start_tick);
            let (page, page_start, page_notes) = paginate(state, &notes);
            let total = notes.len();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("音符 {} 个", total)).size(12.0).strong());
                if let Some(np) = render_pager(ui, page, total_pages(total)) {
                    state.event_page = np;
                }
            });
            ui.add_space(2.0);
            build_table(ui, "eb_notes", &[("#", 40.0), ("id", 70.0), (t!("event_browser.header.tick").as_ref(), 70.0), (t!("event_browser.header.position").as_ref(), 80.0), (t!("event_browser.header.end_tick").as_ref(), 80.0), ("结束位置", 90.0), ("键位", 50.0), ("力度", 50.0)], page_notes.len(), |i, row| {
                let (n, _key, _trk) = &page_notes[i];
                cell_text(row, format!("{}", page_start + i + 1));
                cell_text(row, format!("#{}", n.id));
                cell_text(row, format!("{}", n.start_tick));
                cell_text(row, bar_lookup.format(n.start_tick));
                cell_text(row, format!("{}", n.end_tick));
                cell_text(row, bar_lookup.format(n.end_tick));
                cell_text(row, format!("{}", n.key));
                cell_text(row, format!("{}", n.velocity));
            });
            // 音符：矩形闪烁 + 切到音符所在 track
            return take_row_click(ui, "eb_notes").map(|i| {
                let (n, _key, _trk) = &page_notes[i];
                JumpRequest {
                    tick: n.start_tick,
                    note: Some((*track, n.key)),
                    pulse: Some(PulseKind::NoteRect),
                }
            });
        }
        SelectedItem::Cc { track, controller } => {
            let t = *track as usize;
            let mut events: Vec<&yinhe_types::AutomationEvent> = Vec::new();
            if let Some(td) = model.tracks.get(t) {
                for lane in &td.automation_lanes {
                    if matches!(lane.target, AutomationTarget::CC { controller: c } if c == *controller) {
                        events.extend(lane.events.iter());
                    }
                }
            }
            events.sort_by_key(|e| e.tick);
            let (page, page_start, page_items) = paginate(state, &events);
            let total = events.len();
            let title = format!("CC {} {} {} 个", controller, cc_label(*controller), total);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).size(12.0).strong());
                if let Some(np) = render_pager(ui, page, total_pages(total)) {
                    state.event_page = np;
                }
            });
            ui.add_space(2.0);
            build_table(ui, "eb_cc", &[("#", 40.0), (t!("event_browser.header.tick").as_ref(), 70.0), (t!("event_browser.header.position").as_ref(), 80.0), ("值", 60.0)], page_items.len(), |i, row| {
                let e = page_items[i];
                cell_text(row, format!("{}", page_start + i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}", e.value.round() as i32));
            });
            // CC：切到所在 track，仅跳转不闪烁（Step 2 再做圆点闪烁）
            return take_row_click(ui, "eb_cc").map(|i| JumpRequest {
                tick: page_items[i].tick,
                note: Some((*track, 0)),
                pulse: None,
            });
        }
        SelectedItem::PitchBend { track } => {
            let t = *track as usize;
            let mut events: Vec<&yinhe_types::AutomationEvent> = Vec::new();
            if let Some(td) = model.tracks.get(t) {
                for lane in &td.automation_lanes {
                    if lane.target == AutomationTarget::PitchBend {
                        events.extend(lane.events.iter());
                    }
                }
            }
            events.sort_by_key(|e| e.tick);
            let (page, page_start, page_items) = paginate(state, &events);
            let total = events.len();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("弯音事件 {} 个", total)).size(12.0).strong());
                if let Some(np) = render_pager(ui, page, total_pages(total)) {
                    state.event_page = np;
                }
            });
            ui.add_space(2.0);
            build_table(ui, "eb_pb", &[("#", 40.0), (t!("event_browser.header.tick").as_ref(), 70.0), (t!("event_browser.header.position").as_ref(), 80.0), ("值", 70.0)], page_items.len(), |i, row| {
                let e = page_items[i];
                cell_text(row, format!("{}", page_start + i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}", e.value.round() as i32));
            });
            // PB：切到所在 track，仅跳转不闪烁
            return take_row_click(ui, "eb_pb").map(|i| JumpRequest {
                tick: page_items[i].tick,
                note: Some((*track, 0)),
                pulse: None,
            });
        }
        SelectedItem::ProgramChange { track } => {
            let t = *track as usize;
            let mut events: Vec<&yinhe_core::PcEvent> = model.tracks.get(t)
                .map(|td| td.program_change.iter().collect())
                .unwrap_or_default();
            events.sort_by_key(|e| e.tick);
            let (page, page_start, page_items) = paginate(state, &events);
            let total = events.len();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("音色变更 {} 个", total)).size(12.0).strong());
                if let Some(np) = render_pager(ui, page, total_pages(total)) {
                    state.event_page = np;
                }
            });
            ui.add_space(2.0);
            build_table(ui, "eb_pc", &[("#", 40.0), (t!("event_browser.header.tick").as_ref(), 70.0), (t!("event_browser.header.position").as_ref(), 80.0), ("音色", 50.0)], page_items.len(), |i, row| {
                let e = page_items[i];
                cell_text(row, format!("{}", page_start + i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}", e.program));
            });
            // PC：切到所在 track，仅跳转不闪烁
            return take_row_click(ui, "eb_pc").map(|i| JumpRequest {
                tick: page_items[i].tick,
                note: Some((*track, 0)),
                pulse: None,
            });
        }
    }
}

/// 每页行数。100 行在常规字体下约填满半屏~一屏，翻页频率适中。
const EVENT_PAGE_SIZE: usize = 100;

/// 计算总页数（至少 1 页）。
fn total_pages(total: usize) -> usize {
    total.div_ceil(EVENT_PAGE_SIZE).max(1)
}

/// 根据当前 `state.event_page` 切片出当前页。
///
/// 返回 `(page, page_start, page_slice)`：
/// - `page`：0-based 页码（已做越界保护，删除数据后自动夹回末页）
/// - `page_start`：当前页起始索引（`page * EVENT_PAGE_SIZE`）
/// - `page_slice`：当前页的切片
fn paginate<'a, T>(state: &mut EventBrowserState, items: &'a [T]) -> (usize, usize, &'a [T]) {
    let total = items.len();
    let tp = total_pages(total);
    if state.event_page >= tp {
        state.event_page = tp - 1;
    }
    let page = state.event_page;
    let start = page * EVENT_PAGE_SIZE;
    let end = (start + EVENT_PAGE_SIZE).min(total);
    (page, start, &items[start..end])
}

/// 渲染翻页控件（右对齐），返回 `Some(新页码)` 如果用户改变了页码（0-based）。
///
/// 用 `TextEdit` + chevron 图标按钮：点击输入页码，左右箭头翻页。
/// 所有元素用自然高度，靠 right_to_left 布局的 Center 对齐垂直居中。
/// 输入框用 `ui.memory()` 存临时文本，失焦时解析；按钮翻页时输入框下一帧自动同步。
fn render_pager(ui: &mut egui::Ui, page: usize, total_pages: usize) -> Option<usize> {
    let mut new_page = None;
    let mem_key = ui.id().with("eb_page_input");
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // right_to_left：先放的在最右
        // 下一页按钮
        let next_enabled = page + 1 < total_pages;
        if ui.add_enabled(next_enabled, egui::Label::new(ICON_CHEVRON_RIGHT.rich_text().size(14.0).color(egui::Color32::from_gray(200))).sense(egui::Sense::click())).clicked() {
            new_page = Some(page + 1);
        }
        // 总页数
        ui.label(egui::RichText::new(format!("/ {}", total_pages)).size(11.0).color(egui::Color32::from_gray(140)));
        // 页码输入框（1-based 显示）
        // 关键：只在 TextEdit 有焦点时才写 mem_key，没焦点时不写。
        // 这样 chevron 翻页后 state.event_page 更新，下一帧 mem_key 为 None，
        // fallback 到 (page+1).to_string() 显示新页码；用户输入时 mem_key 保存临时文本不被覆盖。
        let buf: String = ui.memory(|m| m.data.get_temp(mem_key).unwrap_or_else(|| (page + 1).to_string()));
        let mut buf = buf;
        let resp = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(28.0)
                .font(egui::FontId::proportional(11.0))
                .horizontal_align(egui::Align::Center),
        );
        let edited_buf = buf.clone();
        if resp.has_focus() {
            // 用户正在输入，保存临时文本
            ui.memory_mut(|m| m.data.insert_temp(mem_key, buf));
        }
        if resp.lost_focus() {
            // 失焦：解析页码并清掉临时文本
            if let Ok(n) = edited_buf.trim().parse::<usize>() {
                if n >= 1 && n <= total_pages && n - 1 != page {
                    new_page = Some(n - 1);
                }
            }
            ui.memory_mut(|m| m.data.remove::<String>(mem_key));
        }
        // 上一页按钮
        let prev_enabled = page > 0;
        if ui.add_enabled(prev_enabled, egui::Label::new(ICON_CHEVRON_LEFT.rich_text().size(14.0).color(egui::Color32::from_gray(200))).sense(egui::Sense::click())).clicked() {
            new_page = Some(page - 1);
        }
    });
    new_page
}

fn show_project_json(ui: &mut egui::Ui, doc: &Document) {
    let pf = &doc.data.project_file;
    ui.add_space(4.0);
    ui.label(egui::RichText::new("project.json").size(13.0).strong());
    ui.add_space(6.0);

    let kv = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(k).size(11.0).color(egui::Color32::GRAY));
            ui.label(egui::RichText::new(v).size(11.0).monospace().color(egui::Color32::from_gray(220)));
        });
    };

    kv(ui, "version", format!("{}", pf.version));
    kv(ui, "name", pf.name.clone());
    kv(ui, "artist", pf.artist.clone());
    kv(ui, "description", pf.description.clone());
    kv(ui, "ppq", format!("{}", pf.ppq));
    kv(ui, "compression_level", format!("{}", pf.compression_level));
    kv(ui, "soundfont_project_mode", format!("{}", pf.soundfont_project_mode));

    if !pf.soundfont_overrides.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("soundfont_overrides").size(11.0).strong());
        for po in &pf.soundfont_overrides {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(egui::RichText::new(format!("port {}:", po.port)).size(11.0).color(egui::Color32::GRAY));
            });
            for entry in &po.entries {
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    let status = if entry.enabled { "\u{2705}" } else { "\u{274c}" };
                    ui.label(egui::RichText::new(format!("{} {} ({})", status, entry.name, entry.path)).size(10.0).monospace().color(egui::Color32::from_gray(180)));
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
            ui.label(egui::RichText::new(v).size(11.0).monospace().color(egui::Color32::from_gray(220)));
        });
    };

    kv(ui, "version", format!("{}", mf.version));

    ui.add_space(6.0);
    ui.label(egui::RichText::new("ports").size(11.0).strong());
    for p in &mf.ports {
        for ch in &p.channels {
            for t in &ch.tracks {
                let muted = if t.muted { t!("event_browser.muted_badge").to_string() } else { String::new() };
                let soloed = if t.soloed { t!("event_browser.solo_badge").to_string() } else { String::new() };
                kv(ui, &format!("P{} Ch{}", p.port, ch.channel + 1),
                   format!("{} ({}){}{}", t.name, &t.uuid[..8], muted, soloed));
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

fn show_overview(ui: &mut egui::Ui, model: &yinhe_core::YinModel) {
    ui.label(egui::RichText::new("工程概览").size(14.0).strong());
    ui.add_space(4.0);
    let name = if model.meta.name.is_empty() { "(未命名)" } else { &model.meta.name };
    let artist = if model.meta.artist.is_empty() { "(未填)" } else { &model.meta.artist };
    ui.colored_label(egui::Color32::from_gray(120), format!("名称: {}", name));
    ui.colored_label(egui::Color32::from_gray(120), format!("作者: {}", artist));
    ui.colored_label(egui::Color32::from_gray(120), format!("PPQ: {}", model.meta.ppq));
    ui.colored_label(egui::Color32::from_gray(120), format!("zstd 等级: {}", model.meta.compression_level));
    let groups = group_tracks_by_port_channel(model, None);
    ui.colored_label(egui::Color32::from_gray(120), format!("活跃 port 数: {}", groups.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("轨道: {} 个", model.tracks.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("音符: {} 个", model.note_count));
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
    ui.colored_label(egui::Color32::from_gray(120), format!("音色变更: {} 个", pc));
    ui.colored_label(egui::Color32::from_gray(120), format!("Tempo: {} 个", model.conductor.tempo.events.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("拍号: {} 个", model.conductor.time_sig.len()));
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧条目查看详情");
}

fn show_track_detail(ui: &mut egui::Ui, idx: u16, track: &yinhe_core::TrackData, model: &yinhe_core::YinModel) {
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
            ui.label(egui::RichText::new(v).size(11.0).monospace().color(egui::Color32::from_gray(220)));
        });
    };

    kv(ui, "UUID", track.uuid.clone());
    kv(ui, "Port / Channel", format!("{} / {}", port_letter(track.port), track.channel + 1));
    kv(ui, t!("event_browser.channel_prefix").as_ref(), match track.channel_prefix {
        Some(c) => format!("{}", c),
        None => t!("common.none").to_string(),
    });
    kv(ui, t!("event_browser.color").as_ref(), format!("[{:.2}, {:.2}, {:.2}]", track.color[0], track.color[1], track.color[2]));
    kv(ui, t!("event_browser.muted_soloed").as_ref(), format!("{} / {}", track.muted, track.soloed));
    ui.add_space(6.0);
    ui.label(egui::RichText::new("事件计数").size(12.0).strong());
    kv(ui, "Notes", format!("{}", model.track_note_count.get(idx as usize).copied().unwrap_or(0)));
    // Count CC/PB/RPN events from automation_lanes
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
            AutomationTarget::Rpn { .. } | AutomationTarget::Nrpn { .. } => rpn_total += lane.events.len(),
            // Tempo 在 conductor.tempo，不出现在 track.automation_lanes 里。
            AutomationTarget::Tempo => {}
        }
    }
    if !cc_controllers.is_empty() {
        kv(ui, "CC", t!("event_browser.cc_summary", controllers = cc_controllers.len(), events = cc_total).to_string());
        for (i, ctrl) in cc_controllers.iter().enumerate() {
            kv(ui, &format!("  CC {} {}", ctrl, cc_label(*ctrl)), t!("event_browser.cc_count", n = cc_counts[i]).to_string());
        }
    }
    kv(ui, "Pitch Bend", format!("{}", pb_total));
    kv(ui, "Program Change", format!("{}", track.program_change.len()));
    if rpn_total > 0 {
        kv(ui, "RPN/NRPN", t!("event_browser.rpn_summary", n = rpn_total).to_string());
    }
}

// ═══════════════════════════════════════════════════════════════
//  Shared helpers
// ═══════════════════════════════════════════════════════════════

fn cc_label(controller: u8) -> &'static str {
    match controller {
        0 => "Bank Select MSB", 1 => "Modulation", 7 => "Volume",
        10 => "Pan", 11 => "Expression", 64 => "Sustain",
        91 => "Reverb", 93 => "Chorus", _ => "",
    }
}

fn port_letter(port: u8) -> char {
    if port < 26 {
        (b'A' + port) as char
    } else {
        '?'
    }
}

fn toggle_key(state: &mut EventBrowserState, key: ArchiveKey) {
    if state.expanded_keys.contains(&key) {
        state.expanded_keys.remove(&key);
    } else {
        state.expanded_keys.insert(key);
    }
}

// ── Tree row renderers ──

fn render_dir_row(ui: &mut egui::Ui, name: &str, depth: usize, expanded: bool, child_count: usize) -> bool {
    let mut toggled = false;
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(2, 1)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(depth as f32 * 14.0);
            let chev = if expanded { ICON_EXPAND_MORE } else { ICON_CHEVRON_RIGHT };
            if ui.add(egui::Label::new(chev.rich_text().size(13.0).color(egui::Color32::from_gray(190))).sense(egui::Sense::click())).clicked() { toggled = true; }
            let folder = if expanded { ICON_FOLDER_OPEN } else { ICON_FOLDER };
            if ui.add(egui::Label::new(folder.rich_text().size(13.0).color(egui::Color32::from_rgb(220, 180, 90))).sense(egui::Sense::click())).clicked() { toggled = true; }
            ui.label(egui::RichText::new(name).size(11.0).color(egui::Color32::from_gray(220)));
            ui.label(egui::RichText::new(format!("({})", child_count)).size(10.0).color(egui::Color32::from_gray(110)));
        });
    });
    toggled
}

fn render_leaf_item(ui: &mut egui::Ui, name: &str, icon: egui_material_icons::MaterialIcon, depth: usize, item: SelectedItem, state: &mut EventBrowserState) {
    let is_selected = state.selected_item.as_ref() == Some(&item);
    let bg = if is_selected { egui::Color32::from_rgb(40, 50, 70) } else { egui::Color32::TRANSPARENT };
    let frame_r = egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(2, 1)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(depth as f32 * 14.0);
            ui.add_space(14.0);
            ui.label(icon.rich_text().size(12.0).color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(160) }));
            ui.label(egui::RichText::new(name).size(11.0).monospace().color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(200) }));
        });
    });
    if frame_r.response.interact(egui::Sense::click()).clicked() {
        state.selected_item = Some(item);
        state.selected_track = None;
        state.event_page = 0;
    }
}

// ── Table helpers ──

fn build_table<F>(ui: &mut egui::Ui, id_salt: &str, headers: &[(&str, f32)], rows: usize, mut row_cb: F)
where F: FnMut(usize, &mut egui_extras::TableRow) {
    let ctx = ui.ctx().clone();
    let click_key = ui.id().with(("row_click", id_salt));
    let mut tb = TableBuilder::new(ui)
        .id_salt(id_salt)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for (_, min_w) in headers { tb = tb.column(Column::initial(*min_w).at_least(40.0).clip(true)); }
    tb.header(20.0, |mut h| {
        for (label, _) in headers { h.col(|ui| { ui.label(egui::RichText::new(*label).strong().size(11.0)); }); }
    }).body(move |body| {
        body.rows(18.0, rows, move |mut row| {
            let i = row.index();
            row_cb(i, &mut row);
            // 行点击：sense(click) 让整行可点击，row.response() 是所有 cell 的联合
            if row.response().clicked() {
                ctx.memory_mut(|m| m.data.insert_temp(click_key, i));
            }
        });
    });
}

/// 取出 build_table 写入 memory 的行点击索引（若存在）。
fn take_row_click(ui: &egui::Ui, id_salt: &str) -> Option<usize> {
    let key = ui.id().with(("row_click", id_salt));
    let v = ui.memory(|m| m.data.get_temp::<usize>(key));
    if v.is_some() {
        ui.memory_mut(|m| m.data.remove::<usize>(key));
    }
    v
}

fn cell_text(row: &mut egui_extras::TableRow, text: impl Into<String>) {
    let s: String = text.into();
    row.col(|ui| {
        // selectable(false)：避免文字选中消费点击事件，让整行点击生效
        ui.add(egui::Label::new(egui::RichText::new(s).size(11.0).monospace()).selectable(false));
    });
}

// ── Group tracks by port/channel ──

fn group_tracks_by_port_channel(
    model: &yinhe_core::YinModel,
    conductor_idx: Option<u16>,
) -> std::collections::BTreeMap<u8, std::collections::BTreeMap<u8, Vec<u16>>> {
    let mut out: std::collections::BTreeMap<u8, std::collections::BTreeMap<u8, Vec<u16>>> =
        std::collections::BTreeMap::new();
    for (i, t) in model.tracks.iter().enumerate() {
        if Some(i as u16) == conductor_idx {
            continue;
        }
        out.entry(t.port)
            .or_default()
            .entry(t.channel)
            .or_default()
            .push(i as u16);
    }
    out
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_lookup_single_segment() {
        let bl = BarLookup::build(480, 4, &[]);
        assert_eq!(bl.format(0), "1/0");
        assert_eq!(bl.format(480), "1/480");
        assert_eq!(bl.format(1920), "2/0");
    }

    #[test]
    fn bar_lookup_with_time_sig() {
        let bl = BarLookup::build(480, 4, &[(0, 4)]);
        assert_eq!(bl.format(0), "1/0");
        assert_eq!(bl.format(480), "1/480");
    }

    #[test]
    fn bar_lookup_time_sig_change() {
        let bl = BarLookup::build(480, 4, &[(0, 4), (1920, 3)]);
        assert_eq!(bl.format(0), "1/0");
        assert_eq!(bl.format(1920), "2/0");
        assert_eq!(bl.format(2400), "2/480");
    }

    #[test]
    fn bar_lookup_format_tick_zero() {
        let bl = BarLookup::build(480, 4, &[]);
        assert_eq!(bl.format(0), "1/0");
    }

    #[test]
    fn bar_lookup_default_time_sig() {
        let bl = BarLookup::build(480, 4, &[]);
        assert_eq!(bl.format(960), "1/960");
    }

    #[test]
    fn bar_lookup_format_bar_start() {
        let bl = BarLookup::build(480, 4, &[]);
        assert_eq!(bl.format(1920), "2/0");
        assert_eq!(bl.format(3840), "3/0");
    }

    #[test]
    fn bar_lookup_first_ts_after_zero_uses_default() {
        let bl = BarLookup::build(480, 4, &[(1920, 3)]);
        assert_eq!(bl.format(0), "1/0");
        assert_eq!(bl.format(1920), "2/0");
    }

    #[test]
    fn port_letter_basic() {
        assert_eq!(port_letter(0), 'A');
        assert_eq!(port_letter(1), 'B');
        assert_eq!(port_letter(15), 'P');
        assert_eq!(port_letter(25), 'Z');
        assert_eq!(port_letter(26), '?');
        assert_eq!(port_letter(255), '?');
    }

    #[test]
    fn toggle_key_inserts_then_removes() {
        let mut state = EventBrowserState::default();
        assert!(!state.expanded_keys.contains(&ArchiveKey::Conductor));
        toggle_key(&mut state, ArchiveKey::Conductor);
        assert!(state.expanded_keys.contains(&ArchiveKey::Conductor));
        toggle_key(&mut state, ArchiveKey::Conductor);
        assert!(!state.expanded_keys.contains(&ArchiveKey::Conductor));
    }

    #[test]
    fn group_tracks_by_port_channel_orders_and_groups() {
        use std::sync::Arc;
        use yinhe_core::{TrackData, YinModel};

        let mut t0 = TrackData::new(0, 0);
        t0.name = "A0c0".into();
        let mut t1 = TrackData::new(0, 1);
        t1.name = "A0c1".into();
        let mut t2 = TrackData::new(1, 0);
        t2.name = "B0c0".into();
        let mut t3 = TrackData::new(0, 0);
        t3.name = "A0c0_dup".into();

        let model = YinModel {
            tracks: vec![Arc::new(t0), Arc::new(t1), Arc::new(t2), Arc::new(t3)],
            ..Default::default()
        };

        let groups = group_tracks_by_port_channel(&model, None);
        assert_eq!(groups.len(), 2);
        let p0 = &groups[&0];
        assert_eq!(p0.len(), 2);
        assert_eq!(p0[&0], vec![0, 3]);
        assert_eq!(p0[&1], vec![1]);
        assert_eq!(groups[&1][&0], vec![2]);
    }

    #[test]
    fn ts_changes_extracts_tick_numerator() {
        use std::sync::Arc;
        use yinhe_core::{ConductorData, YinModel};
        use yinhe_types::TimeSigEvent;

        let conductor = ConductorData {
            tempo: yinhe_types::AutomationLane {
                target: yinhe_types::AutomationTarget::Tempo,
                track: 0,
                events: vec![],
            },
            time_sig: vec![
                TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
                TimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
            ],
        };
        let model = YinModel {
            conductor: Arc::new(conductor),
            ..Default::default()
        };

        let changes = ts_changes(&model);
        assert_eq!(changes, vec![(0, 4), (1920, 3)]);
    }
}
