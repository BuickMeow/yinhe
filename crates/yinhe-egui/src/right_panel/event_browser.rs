use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_material_icons::icons::*;

use yinhe_editor_core::document::Document;
use crate::widgets::split_handle;
use crate::theme;

// ── State types ──

pub struct EventBrowserState {
    pub expanded_keys: std::collections::HashSet<ArchiveKey>,
    pub selected_item: Option<SelectedItem>,
    pub selected_track: Option<u16>,
    fingerprint: Option<u64>,
    split_ratio: f32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SelectedItem {
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

pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>, state: &mut EventBrowserState) {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("\u{ff08}\u{672a}\u{6253}\u{5f00}\u{6587}\u{6863}\u{ff09}")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    };

    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let default_num = model.tempo_map.time_sig_default.0;
    let ts = ts_changes(model);
    let bar_lookup = BarLookup::build(ppq, default_num, &ts);

    let fingerprint = doc.data.midi_version;
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
        .fill(egui::Color32::from_gray(16))
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
                    ui.vertical(|ui| render_tree(ui, model, state));
                });
            });
    });

    let resp = split_handle::horizontal(ui, "__eb_split__", handle_rect);
    if resp.dragged() {
        let new_ratio = ((split_y + resp.drag_delta().y - total_rect.min.y) / total_h)
            .clamp(theme::SPLIT_CLAMP_MIN, theme::SPLIT_CLAMP_MAX);
        state.split_ratio = new_ratio;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(bot_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("eb_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(sel) = &state.selected_item {
                        show_event_detail(ui, sel, model, &bar_lookup);
                    } else if let Some(idx) = state.selected_track {
                        if let Some(track) = model.tracks.get(idx as usize) {
                            show_track_detail(ui, idx, track);
                        } else {
                            show_overview(ui, model);
                        }
                    } else {
                        show_overview(ui, model);
                    }
                });
            });
    });
}

// ═══════════════════════════════════════════════════════════════
//  Unified tree
// ═══════════════════════════════════════════════════════════════

fn render_tree(
    ui: &mut egui::Ui,
    model: &yinhe_core::YinModel,
    state: &mut EventBrowserState,
) {
    let groups = group_tracks_by_port_channel(model);

    let has_tempo = !model.conductor.tempo.is_empty();
    let has_ts = !model.conductor.time_sig.is_empty();
    if has_tempo || has_ts {
        let cond_expanded = state.expanded_keys.contains(&ArchiveKey::Conductor);
        let child_count = has_tempo as usize + has_ts as usize;
        if render_dir_row(ui, "Conductor", 0, cond_expanded, child_count) {
            toggle_key(state, ArchiveKey::Conductor);
        }
        if cond_expanded {
            if has_tempo {
                render_leaf_item(ui, &format!("Tempo ({})", model.conductor.tempo.len()), ICON_SPEED, 1, SelectedItem::Tempo, state);
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
        let port_label = format!("Port {} ({} tracks)", port_letter(port), port_track_count);
        if render_dir_row(ui, &port_label, 0, port_expanded, channels.len()) {
            toggle_key(state, port_key);
        }
        if !port_expanded {
            continue;
        }

        for (&channel, track_indices) in channels {
            let ch_key = ArchiveKey::Channel(port, channel);
            let ch_expanded = state.expanded_keys.contains(&ch_key);
            let ch_label = format!("Channel {} ({} tracks)", channel + 1, track_indices.len());
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

    let note_count = track.notes.len();
    let cc_map: std::collections::BTreeMap<u8, usize> =
        track.cc.iter().map(|(&c, v)| (c, v.len())).collect();
    let pb_count = track.pitch_bend.len();
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
        track.cc.values().map(|v| v.len()).sum::<usize>(),
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

fn show_event_detail(ui: &mut egui::Ui, item: &SelectedItem, model: &yinhe_core::YinModel, bar_lookup: &BarLookup) {
    match item {
        SelectedItem::Tempo => {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Tempo ({} 个)", model.conductor.tempo.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_tempo", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("BPM", 70.0)], model.conductor.tempo.len(), |i, row| {
                let s = &model.conductor.tempo[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", s.tick));
                cell_text(row, bar_lookup.format(s.tick as u32));
                cell_text(row, format!("{:.2}", s.bpm));
            });
        }
        SelectedItem::TimeSig => {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("拍号 ({} 个)", model.conductor.time_sig.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_ts", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("拍号", 80.0)], model.conductor.time_sig.len(), |i, row| {
                let e = &model.conductor.time_sig[i];
                let denom = 1u32 << e.denominator as u32;
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}/{}", e.numerator, denom));
            });
        }
        SelectedItem::Notes { track } => {
            let t = *track as usize;
            let track_data = model.tracks.get(t);
            let notes: Vec<(&yinhe_core::NoteEvent, u8, u16)> = if let Some(td) = track_data {
                td.notes.iter().map(|n| (n, n.key, *track)).collect()
            } else { Vec::new() };
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("音符 ({} 个)", notes.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_notes", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("结束 tick", 80.0), ("结束位置", 90.0), ("键位", 50.0), ("力度", 50.0)], notes.len(), |i, row| {
                let (n, _key, _trk) = &notes[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", n.start_tick));
                cell_text(row, bar_lookup.format(n.start_tick));
                cell_text(row, format!("{}", n.end_tick));
                cell_text(row, bar_lookup.format(n.end_tick));
                cell_text(row, format!("{}", n.key));
                cell_text(row, format!("{}", n.velocity));
            });
        }
        SelectedItem::Cc { track, controller } => {
            let t = *track as usize;
            let events: Vec<&yinhe_core::CcEvent> = model.tracks.get(t)
                .and_then(|td| td.cc.get(controller))
                .map(|v| v.iter().collect())
                .unwrap_or_default();
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("CC {} {} ({} 个)", controller, cc_label(*controller), events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_cc", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("值", 60.0)], events.len(), |i, row| {
                let e = events[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}", e.value));
            });
        }
        SelectedItem::PitchBend { track } => {
            let t = *track as usize;
            let events: Vec<&yinhe_core::PitchBendEvent> = model.tracks.get(t)
                .map(|td| td.pitch_bend.iter().collect())
                .unwrap_or_default();
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("弯音事件 ({} 个)", events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_pb", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("值", 70.0)], events.len(), |i, row| {
                let e = events[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}", e.value));
            });
        }
        SelectedItem::ProgramChange { track } => {
            let t = *track as usize;
            let events: Vec<&yinhe_core::PcEvent> = model.tracks.get(t)
                .map(|td| td.program_change.iter().collect())
                .unwrap_or_default();
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("音色变更 ({} 个)", events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_pc", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("音色", 50.0)], events.len(), |i, row| {
                let e = events[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}", e.program));
            });
        }
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
    let groups = group_tracks_by_port_channel(model);
    ui.colored_label(egui::Color32::from_gray(120), format!("活跃 port 数: {}", groups.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("轨道: {} 个", model.tracks.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("音符: {} 个", model.note_count));
    let mut cc = 0usize;
    let mut pb = 0usize;
    let mut pc = 0usize;
    for t in &model.tracks {
        cc += t.cc.values().map(|v| v.len()).sum::<usize>();
        pb += t.pitch_bend.len();
        pc += t.program_change.len();
    }
    ui.colored_label(egui::Color32::from_gray(120), format!("CC: {} 个", cc));
    ui.colored_label(egui::Color32::from_gray(120), format!("弯音: {} 个", pb));
    ui.colored_label(egui::Color32::from_gray(120), format!("音色变更: {} 个", pc));
    ui.colored_label(egui::Color32::from_gray(120), format!("Tempo: {} 个", model.conductor.tempo.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("拍号: {} 个", model.conductor.time_sig.len()));
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧条目查看详情");
}

fn show_track_detail(ui: &mut egui::Ui, idx: u16, track: &yinhe_core::TrackData) {
    ui.add_space(4.0);
    let header = if track.name.is_empty() {
        format!("Track #{} (未命名)", idx)
    } else {
        format!("Track #{} — {}", idx, track.name)
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
    kv(ui, "Channel Prefix", match track.channel_prefix {
        Some(c) => format!("{}", c),
        None => "(none)".to_string(),
    });
    kv(ui, "Color", format!("[{:.2}, {:.2}, {:.2}]", track.color[0], track.color[1], track.color[2]));
    kv(ui, "Muted / Soloed", format!("{} / {}", track.muted, track.soloed));
    ui.add_space(6.0);
    ui.label(egui::RichText::new("事件计数").size(12.0).strong());
    kv(ui, "Notes", format!("{}", track.notes.len()));
    if !track.cc.is_empty() {
        let total_cc: usize = track.cc.values().map(|v| v.len()).sum();
        kv(ui, "CC", format!("{} controllers, {} events total", track.cc.len(), total_cc));
        for (&ctrl, evs) in &track.cc {
            kv(ui, &format!("  CC {} {}", ctrl, cc_label(ctrl)), format!("{} events", evs.len()));
        }
    }
    kv(ui, "Pitch Bend", format!("{}", track.pitch_bend.len()));
    kv(ui, "Program Change", format!("{}", track.program_change.len()));
    if !track.rpn.is_empty() {
        let total_rpn: usize = track.rpn.values().map(|v| v.len()).sum();
        kv(ui, "RPN", format!("{} keys, {} events total", track.rpn.len(), total_rpn));
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
    if frame_r.response.interact(egui::Sense::click()).clicked() { state.selected_item = Some(item); }
}

// ── Table helpers ──

fn build_table<F>(ui: &mut egui::Ui, id_salt: &str, headers: &[(&str, f32)], rows: usize, mut row_cb: F)
where F: FnMut(usize, &mut egui_extras::TableRow) {
    let mut tb = TableBuilder::new(ui).id_salt(id_salt).striped(true).resizable(true).cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for (_, min_w) in headers { tb = tb.column(Column::initial(*min_w).at_least(40.0).clip(true)); }
    tb.header(20.0, |mut h| {
        for (label, _) in headers { h.col(|ui| { ui.label(egui::RichText::new(*label).strong().size(11.0)); }); }
    }).body(|body| {
        body.rows(18.0, rows, |mut row| { let i = row.index(); row_cb(i, &mut row); });
    });
}

fn cell_text(row: &mut egui_extras::TableRow, text: impl Into<String>) {
    let s: String = text.into();
    row.col(|ui| { ui.label(egui::RichText::new(s).size(11.0).monospace()); });
}

// ── Group tracks by port/channel ──

fn group_tracks_by_port_channel(
    model: &yinhe_core::YinModel,
) -> std::collections::BTreeMap<u8, std::collections::BTreeMap<u8, Vec<u16>>> {
    let mut out: std::collections::BTreeMap<u8, std::collections::BTreeMap<u8, Vec<u16>>> =
        std::collections::BTreeMap::new();
    for (i, t) in model.tracks.iter().enumerate() {
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

        let groups = group_tracks_by_port_channel(&model);
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
        use yinhe_core::{ConductorData, TimeSigEvent, YinModel};

        let conductor = ConductorData {
            tempo: vec![],
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
