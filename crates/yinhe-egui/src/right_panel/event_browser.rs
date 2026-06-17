use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_material_icons::icons::*;
use std::collections::BTreeMap;
use yinhe_midi::MidiControlEvent;
use yinhe_project::{ArchiveEntry, ProjectArchive};
use yinhe_types::TimeSigEvent as TypesTimeSigEvent;

use yinhe_editor_core::document::Document;
use crate::widgets::split_handle;
use crate::theme;

// ── Tab / selection state ──

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewTab {
    Realtime,
    Archive,
}

pub struct EventBrowserState {
    pub active_tab: ViewTab,
    // Realtime view state
    pub expanded_tracks: std::collections::HashSet<u16>,
    pub selected_item: Option<SelectedItem>,
    midi_fingerprint: Option<u64>,
    // Archive view state
    pub expanded_archive_paths: std::collections::HashSet<String>,
    pub selected_archive_path: Option<String>,
    archive_fingerprint: Option<usize>,
    // Shared
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

impl Default for EventBrowserState {
    fn default() -> Self {
        Self {
            active_tab: ViewTab::Realtime,
            expanded_tracks: Default::default(),
            selected_item: None,
            midi_fingerprint: None,
            expanded_archive_paths: Default::default(),
            selected_archive_path: None,
            archive_fingerprint: None,
            split_ratio: 0.45,
        }
    }
}

// ── Bar lookup (shared by both views) ──

struct BarLookup {
    segs: Vec<BarSeg>,
}

struct BarSeg {
    tick_start: u32,
    bar_start: u32,
    ticks_per_bar: u32,
}

impl BarLookup {
    fn build(ppq: u32, default_num: u8, ts_events: &[TypesTimeSigEvent]) -> Self {
        let mut points: Vec<(u32, u8)> = Vec::new();
        if ts_events.first().map(|e| e.tick).unwrap_or(u32::MAX) != 0 {
            points.push((0, default_num));
        }
        for e in ts_events {
            points.push((e.tick, e.numerator));
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

// ═══════════════════════════════════════════════════════════════
//  Main entry
// ═══════════════════════════════════════════════════════════════

pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>, state: &mut EventBrowserState) {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未打开文档）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    };

    // ── Tab bar ──
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let rt_resp = ui.selectable_label(state.active_tab == ViewTab::Realtime, "实时事件");
        let ar_resp = ui.selectable_label(state.active_tab == ViewTab::Archive, "归档结构");
        if rt_resp.clicked() {
            state.active_tab = ViewTab::Realtime;
        }
        if ar_resp.clicked() {
            state.active_tab = ViewTab::Archive;
        }
    });
    ui.separator();

    // ── Dispatch ──
    match state.active_tab {
        ViewTab::Realtime => show_realtime(ui, doc, state),
        ViewTab::Archive => show_archive(ui, doc, state),
    }
}

// ═══════════════════════════════════════════════════════════════
//  Realtime view (reads from MidiFile)
// ═══════════════════════════════════════════════════════════════

fn show_realtime(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState) {
    let midi = doc.midi();
    let ppq = midi.ticks_per_beat;
    let default_num = midi.time_sig_numerator;
    let ts_events: Vec<TypesTimeSigEvent> = midi.time_sig_events.clone();
    let bar_lookup = BarLookup::build(ppq, default_num, &ts_events);

    let fingerprint = midi.tick_length
        ^ (midi.note_count << 16)
        ^ (midi.control_events.len() as u64).wrapping_mul(0x9E3779B9);
    if state.midi_fingerprint != Some(fingerprint) {
        state.expanded_tracks.clear();
        for i in 0..midi.track_ports.len() {
            state.expanded_tracks.insert(i as u16);
        }
        state.midi_fingerprint = Some(fingerprint);
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
            .id_salt("eb_rt_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.vertical(|ui| render_realtime_tree(ui, midi, state));
                });
            });
    });

    let resp = split_handle::horizontal(ui, "__eb_rt_split__", handle_rect);
    if resp.dragged() {
        let new_ratio = ((split_y + resp.drag_delta().y - total_rect.min.y) / total_h)
            .clamp(theme::SPLIT_CLAMP_MIN, theme::SPLIT_CLAMP_MAX);
        state.split_ratio = new_ratio;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(bot_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("eb_rt_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(sel) = &state.selected_item {
                        show_realtime_detail(ui, sel, midi, &bar_lookup);
                    } else {
                        show_realtime_overview(ui, midi);
                    }
                });
            });
    });
}

// ── Realtime tree ──

fn render_realtime_tree(ui: &mut egui::Ui, midi: &yinhe_midi::MidiFile, state: &mut EventBrowserState) {
    let num_tracks = midi.track_ports.len();

    if !midi.tempo_segments.is_empty() || !midi.time_sig_events.is_empty() {
        let expanded = state.expanded_tracks.contains(&u16::MAX)
            || state.selected_item.as_ref().map(|s| matches!(s, SelectedItem::Tempo | SelectedItem::TimeSig)).unwrap_or(false);
        let tempo_count = midi.tempo_segments.len();
        let ts_count = midi.time_sig_events.len();
        let child_count = tempo_count + ts_count;

        if render_dir_row(ui, "Conductor", 0, expanded, child_count) {
            if expanded { state.expanded_tracks.remove(&u16::MAX); }
            else { state.expanded_tracks.insert(u16::MAX); }
        }
        if expanded {
            if !midi.tempo_segments.is_empty() {
                render_leaf_item(ui, &format!("Tempo ({})", tempo_count), ICON_SPEED, 1, SelectedItem::Tempo, state);
            }
            if !midi.time_sig_events.is_empty() {
                render_leaf_item(ui, &format!("TimeSig ({})", ts_count), ICON_SCHEDULE, 1, SelectedItem::TimeSig, state);
            }
        }
    }

    for track_idx in 0..num_tracks {
        let t = track_idx as u16;
        let name = midi.track_names.get(track_idx).cloned().unwrap_or_else(|| format!("Track {}", track_idx + 1));
        let ch = midi.track_channels.get(track_idx).copied().unwrap_or(0);
        let note_count = count_notes_for_track(midi, t);
        let cc_map = count_cc_for_track(midi, t);
        let pb = count_pitch_bend(midi, t);
        let pc = count_pc(midi, t);
        let child_count = if note_count > 0 { 1 } else { 0 } + cc_map.len() + if pb > 0 { 1 } else { 0 } + if pc > 0 { 1 } else { 0 };
        if child_count == 0 { continue; }

        let expanded = state.expanded_tracks.contains(&t);
        let label = format!("{} [ch {}]", name, ch);
        if render_dir_row(ui, &label, 0, expanded, child_count) {
            if expanded { state.expanded_tracks.remove(&t); } else { state.expanded_tracks.insert(t); }
        }
        if expanded {
            if note_count > 0 { render_leaf_item(ui, &format!("Notes ({})", note_count), ICON_MUSIC_NOTE, 1, SelectedItem::Notes { track: t }, state); }
            for (&ctrl, &cnt) in &cc_map {
                render_leaf_item(ui, &format!("CC {} {} ({})", ctrl, cc_label(ctrl), cnt), ICON_SETTINGS, 1, SelectedItem::Cc { track: t, controller: ctrl }, state);
            }
            if pb > 0 { render_leaf_item(ui, &format!("Pitch Bend ({})", pb), ICON_EDIT_AUDIO, 1, SelectedItem::PitchBend { track: t }, state); }
            if pc > 0 { render_leaf_item(ui, &format!("Program Change ({})", pc), ICON_PALETTE, 1, SelectedItem::ProgramChange { track: t }, state); }
        }
    }
}

// ── Realtime detail ──

fn show_realtime_detail(ui: &mut egui::Ui, item: &SelectedItem, midi: &yinhe_midi::MidiFile, bar_lookup: &BarLookup) {
    match item {
        SelectedItem::Tempo => {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Tempo ({} 个)", midi.tempo_segments.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_tempo", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("BPM", 70.0)], midi.tempo_segments.len(), |i, row| {
                let s = &midi.tempo_segments[i];
                let bpm = yinhe_midi::bpm_from_mpq(s.micros_per_quarter);
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", s.start_tick));
                cell_text(row, bar_lookup.format(s.start_tick as u32));
                cell_text(row, format!("{:.2}", bpm));
            });
        }
        SelectedItem::TimeSig => {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("拍号 ({} 个)", midi.time_sig_events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_ts", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("拍号", 80.0)], midi.time_sig_events.len(), |i, row| {
                let e = &midi.time_sig_events[i];
                let denom = 1u32 << e.denominator as u32;
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", e.tick));
                cell_text(row, bar_lookup.format(e.tick));
                cell_text(row, format!("{}/{}", e.numerator, denom));
            });
        }
        SelectedItem::Notes { track } => {
            let notes = collect_notes_for_track(midi, *track);
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("音符 ({} 个)", notes.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_notes", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("结束 tick", 80.0), ("结束位置", 90.0), ("键位", 50.0), ("力度", 50.0)], notes.len(), |i, row| {
                let (key, n) = &notes[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", n.start_tick));
                cell_text(row, bar_lookup.format(n.start_tick));
                cell_text(row, format!("{}", n.end_tick));
                cell_text(row, bar_lookup.format(n.end_tick));
                cell_text(row, format!("{}", key));
                cell_text(row, format!("{}", n.velocity));
            });
        }
        SelectedItem::Cc { track, controller } => {
            let events: Vec<&MidiControlEvent> = midi.control_events.iter().filter(|ev| matches!(ev, MidiControlEvent::ControlChange { track: t, controller: c, .. } if *t == *track && *c == *controller)).collect();
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("CC {} {} ({} 个)", controller, cc_label(*controller), events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_cc", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("值", 60.0)], events.len(), |i, row| {
                if let MidiControlEvent::ControlChange { tick, value, .. } = events[i] {
                    cell_text(row, format!("{}", i + 1));
                    cell_text(row, format!("{}", tick));
                    cell_text(row, bar_lookup.format(*tick));
                    cell_text(row, format!("{}", value));
                }
            });
        }
        SelectedItem::PitchBend { track } => {
            let events: Vec<&MidiControlEvent> = midi.control_events.iter().filter(|ev| matches!(ev, MidiControlEvent::PitchBend { track: t, .. } if *t == *track)).collect();
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("弯音事件 ({} 个)", events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_pb", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("值", 70.0)], events.len(), |i, row| {
                if let MidiControlEvent::PitchBend { tick, value, .. } = events[i] {
                    cell_text(row, format!("{}", i + 1));
                    cell_text(row, format!("{}", tick));
                    cell_text(row, bar_lookup.format(*tick));
                    cell_text(row, format!("{}", value));
                }
            });
        }
        SelectedItem::ProgramChange { track } => {
            let events: Vec<&MidiControlEvent> = midi.control_events.iter().filter(|ev| matches!(ev, MidiControlEvent::ProgramChange { track: t, .. } if *t == *track)).collect();
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("音色变更 ({} 个)", events.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_pc", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("音色", 50.0)], events.len(), |i, row| {
                if let MidiControlEvent::ProgramChange { tick, program, .. } = events[i] {
                    cell_text(row, format!("{}", i + 1));
                    cell_text(row, format!("{}", tick));
                    cell_text(row, bar_lookup.format(*tick));
                    cell_text(row, format!("{}", program));
                }
            });
        }
    }
}

fn show_realtime_overview(ui: &mut egui::Ui, midi: &yinhe_midi::MidiFile) {
    ui.label(egui::RichText::new("工程概览").size(14.0).strong());
    ui.add_space(4.0);
    let cc = midi.control_events.iter().filter(|e| matches!(e, MidiControlEvent::ControlChange { .. })).count();
    let pb = midi.control_events.iter().filter(|e| matches!(e, MidiControlEvent::PitchBend { .. })).count();
    let pc = midi.control_events.iter().filter(|e| matches!(e, MidiControlEvent::ProgramChange { .. })).count();
    ui.colored_label(egui::Color32::from_gray(120), format!("轨道: {} 个", midi.track_ports.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("音符: {} 个", midi.note_count));
    ui.colored_label(egui::Color32::from_gray(120), format!("CC: {} 个", cc));
    ui.colored_label(egui::Color32::from_gray(120), format!("弯音: {} 个", pb));
    ui.colored_label(egui::Color32::from_gray(120), format!("音色变更: {} 个", pc));
    ui.colored_label(egui::Color32::from_gray(120), format!("Tempo: {} 个", midi.tempo_segments.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("拍号: {} 个", midi.time_sig_events.len()));
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧条目查看详情");
}

// ═══════════════════════════════════════════════════════════════
//  Archive view (reads from ProjectArchive)
// ═══════════════════════════════════════════════════════════════

fn show_archive(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState) {
    let Some(archive) = &doc.archive else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（无归档数据 — 仅 .yin 文件支持此视图）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    };

    let mut entries: Vec<(&String, &ArchiveEntry)> = archive.entries.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let fingerprint = entries.len() ^ entries.iter().map(|(_, e)| e.data.len()).sum::<usize>();
    if state.archive_fingerprint != Some(fingerprint) {
        state.expanded_archive_paths.clear();
        // Auto-expand top-level dirs
        for path in entries.iter().map(|(p, _)| *p) {
            if let Some(top) = path.split('/').next() {
                state.expanded_archive_paths.insert(top.to_string());
            }
        }
        state.archive_fingerprint = Some(fingerprint);
    }

    let tree = build_archive_tree(&entries);

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
            .id_salt("eb_ar_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.vertical(|ui| {
                        if let ArchiveTreeNode::Dir { children } = &tree {
                            for (name, node) in children {
                                render_archive_node(ui, name, node, "", 0, archive, state);
                            }
                        }
                    });
                });
            });
    });

    let resp = split_handle::horizontal(ui, "__eb_ar_split__", handle_rect);
    if resp.dragged() {
        let new_ratio = ((split_y + resp.drag_delta().y - total_rect.min.y) / total_h)
            .clamp(theme::SPLIT_CLAMP_MIN, theme::SPLIT_CLAMP_MAX);
        state.split_ratio = new_ratio;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(bot_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("eb_ar_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(sel) = &state.selected_archive_path {
                        if let Some(entry) = archive.entries.get(sel) {
                            show_archive_entry_detail(ui, sel, entry);
                        } else {
                            ui.colored_label(egui::Color32::GRAY, "（条目不存在）");
                        }
                    } else {
                        show_archive_overview(ui, &entries);
                    }
                });
            });
    });
}

// ── Archive tree ──

enum ArchiveTreeNode {
    Dir { children: BTreeMap<String, ArchiveTreeNode> },
    Leaf { full_path: String },
}

fn build_archive_tree(entries: &[(&String, &ArchiveEntry)]) -> ArchiveTreeNode {
    let mut root = ArchiveTreeNode::Dir { children: BTreeMap::new() };
    for (path, _) in entries {
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        insert_archive_node(&mut root, &segs, path);
    }
    root
}

fn insert_archive_node(node: &mut ArchiveTreeNode, segments: &[&str], full_path: &str) {
    let ArchiveTreeNode::Dir { children } = node else { return; };
    match segments {
        [] => {}
        [name] => {
            children.insert((*name).to_string(), ArchiveTreeNode::Leaf { full_path: full_path.to_string() });
        }
        [head, rest @ ..] => {
            let entry = children.entry((*head).to_string()).or_insert_with(|| ArchiveTreeNode::Dir { children: BTreeMap::new() });
            insert_archive_node(entry, rest, full_path);
        }
    }
}

fn archive_node_leaf_count(node: &ArchiveTreeNode) -> usize {
    match node {
        ArchiveTreeNode::Leaf { .. } => 1,
        ArchiveTreeNode::Dir { children } => children.values().map(archive_node_leaf_count).sum(),
    }
}

fn render_archive_node(
    ui: &mut egui::Ui, name: &str, node: &ArchiveTreeNode, parent_path: &str, depth: usize,
    archive: &ProjectArchive, state: &mut EventBrowserState,
) {
    let cur_path = if parent_path.is_empty() { name.to_string() } else { format!("{}/{}", parent_path, name) };
    match node {
        ArchiveTreeNode::Dir { children } => {
            let expanded = state.expanded_archive_paths.contains(&cur_path);
            let count = archive_node_leaf_count(node);
            if render_dir_row(ui, name, depth, expanded, count) {
                if expanded { state.expanded_archive_paths.remove(&cur_path); }
                else { state.expanded_archive_paths.insert(cur_path.clone()); }
            }
            if state.expanded_archive_paths.contains(&cur_path) {
                for (cname, cnode) in children {
                    render_archive_node(ui, cname, cnode, &cur_path, depth + 1, archive, state);
                }
            }
        }
        ArchiveTreeNode::Leaf { full_path } => {
            let entry = match archive.entries.get(full_path) { Some(e) => e, None => return };
            let is_selected = state.selected_archive_path.as_deref() == Some(full_path.as_str());
            let magic_str = format!("{}{}{}{}", entry.header.magic[0] as char, entry.header.magic[1] as char, entry.header.magic[2] as char, entry.header.magic[3] as char);
            let icon = match &entry.header.magic {
                b"YHTK" => ICON_MUSIC_NOTE,
                b"YHCC" => ICON_SETTINGS,
                b"YHPB" => ICON_EDIT_AUDIO,
                b"YHPC" => ICON_PALETTE,
                b"YHTM" => ICON_SPEED,
                b"YHTS" => ICON_SCHEDULE,
                _ => ICON_AUDIO_FILE,
            };
            let bg = if is_selected { egui::Color32::from_rgb(40, 50, 70) } else { egui::Color32::TRANSPARENT };
            let size_label = format_size(entry.data.len());
            let display_name = format!("{} [{}]", name, magic_str);

            let frame_r = egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(2, 1)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_space(depth as f32 * 14.0);
                    ui.add_space(14.0);
                    ui.label(icon.rich_text().size(12.0).color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(160) }));
                    ui.label(egui::RichText::new(display_name).size(11.0).monospace().color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(200) }));
                    ui.label(egui::RichText::new(format!("({})", size_label)).size(10.0).color(egui::Color32::from_gray(110)));
                });
            });
            if frame_r.response.interact(egui::Sense::click()).clicked() {
                state.selected_archive_path = Some(full_path.to_string());
            }
        }
    }
}

// ── Archive entry detail ──

fn show_archive_entry_detail(ui: &mut egui::Ui, _path: &str, entry: &ArchiveEntry) {
    let h = entry.header;
    ui.label(egui::RichText::new("文件头").size(12.0).strong());
    ui.add_space(2.0);
    archive_header_row(ui, "magic", &format!("{}{}{}{}", h.magic[0] as char, h.magic[1] as char, h.magic[2] as char, h.magic[3] as char));
    archive_header_row(ui, "version", &format!("{}", h.version));
    archive_header_row(ui, "port", &format!("{}", h.port));
    archive_header_row(ui, "channel", &format!("{}", h.channel));
    archive_header_row(ui, "extra", &format!("{}", h.extra));
    ui.add_space(4.0);
    archive_header_row(ui, "data size", &format_size(entry.data.len()));

    // Try to decode as specific types
    ui.add_space(6.0);
    match &entry.header.magic {
        b"YHTK" => show_archive_notes(ui, entry),
        b"YHCC" => show_archive_cc(ui, entry),
        b"YHPB" => show_archive_pitch(ui, entry),
        b"YHPC" => show_archive_pc(ui, entry),
        b"YHTM" => show_archive_tempo(ui, entry),
        b"YHTS" => show_archive_time_sig(ui, entry),
        b"YHMP" | b"YHPR" => show_archive_json(ui, entry),
        _ => show_archive_hexdump(ui, &entry.data),
    }
}

fn show_archive_notes(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    // Notes data starts with InnerHeader (3 bytes) then delta-gate encoded notes
    let min_size = yinhe_project::InnerHeader::SIZE;
    if entry.data.len() >= min_size {
        if let Some((_inner, rest)) = yinhe_project::InnerHeader::read(&entry.data) {
            let notes: Vec<yinhe_project::Note> = yinhe_project::decode_delta_events(rest);
            ui.label(egui::RichText::new(format!("音符 ({} 个)", notes.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_ar_notes", &[("#", 40.0), ("start_tick", 70.0), ("end_tick", 70.0), ("key", 50.0), ("velocity", 50.0)], notes.len(), |i, row| {
                let n = &notes[i];
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", n.start_tick));
                cell_text(row, format!("{}", n.end_tick));
                cell_text(row, format!("{}", n.key));
                cell_text(row, format!("{}", n.velocity));
            });
            return;
        }
    }
    // Fallback: try raw decode
    let notes: Vec<yinhe_project::Note> = yinhe_project::decode_delta_events(&entry.data);
    ui.label(egui::RichText::new(format!("音符 ({} 个)", notes.len())).size(12.0).strong());
    ui.add_space(2.0);
    build_table(ui, "eb_ar_notes", &[("#", 40.0), ("start_tick", 70.0), ("end_tick", 70.0), ("key", 50.0), ("velocity", 50.0)], notes.len(), |i, row| {
        let n = &notes[i];
        cell_text(row, format!("{}", i + 1));
        cell_text(row, format!("{}", n.start_tick));
        cell_text(row, format!("{}", n.end_tick));
        cell_text(row, format!("{}", n.key));
        cell_text(row, format!("{}", n.velocity));
    });
}

fn show_archive_cc(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    // CC data starts with InnerHeader (3 bytes) then delta-varint encoded events
    let payload = skip_inner_header_if_present(&entry.data);
    let events: Vec<yinhe_project::CcEvent> = yinhe_project::decode_delta_events(payload);
    ui.label(egui::RichText::new(format!("CC 事件 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    build_table(ui, "eb_ar_cc", &[("#", 40.0), ("tick", 70.0), ("value", 60.0)], events.len(), |i, row| {
        let e = &events[i];
        cell_text(row, format!("{}", i + 1));
        cell_text(row, format!("{}", e.tick));
        cell_text(row, format!("{}", e.value));
    });
}

fn show_archive_pitch(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    let payload = skip_inner_header_if_present(&entry.data);
    let events: Vec<yinhe_project::PitchBendEvent> = yinhe_project::decode_delta_events(payload);
    ui.label(egui::RichText::new(format!("弯音 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    build_table(ui, "eb_ar_pb", &[("#", 40.0), ("tick", 70.0), ("value", 70.0)], events.len(), |i, row| {
        let e = &events[i];
        cell_text(row, format!("{}", i + 1));
        cell_text(row, format!("{}", e.tick));
        cell_text(row, format!("{}", e.value));
    });
}

fn show_archive_pc(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    let payload = skip_inner_header_if_present(&entry.data);
    let events: Vec<yinhe_project::PcEvent> = yinhe_project::decode_delta_events(payload);
    ui.label(egui::RichText::new(format!("音色变更 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    build_table(ui, "eb_ar_pc", &[("#", 40.0), ("tick", 70.0), ("program", 50.0)], events.len(), |i, row| {
        let e = &events[i];
        cell_text(row, format!("{}", i + 1));
        cell_text(row, format!("{}", e.tick));
        cell_text(row, format!("{}", e.program));
    });
}

fn show_archive_tempo(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    let events: Vec<yinhe_project::TempoEvent> = yinhe_project::decode_delta_events(&entry.data);
    ui.label(egui::RichText::new(format!("Tempo ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    build_table(ui, "eb_ar_tempo", &[("#", 40.0), ("tick", 70.0), ("BPM", 70.0)], events.len(), |i, row| {
        let e = &events[i];
        cell_text(row, format!("{}", i + 1));
        cell_text(row, format!("{}", e.tick));
        cell_text(row, format!("{:.2}", e.bpm));
    });
}

fn show_archive_time_sig(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    let events: Vec<yinhe_project::TimeSigEvent> = yinhe_project::decode_delta_events(&entry.data);
    ui.label(egui::RichText::new(format!("拍号 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    build_table(ui, "eb_ar_ts", &[("#", 40.0), ("tick", 70.0), ("拍号", 80.0)], events.len(), |i, row| {
        let e = &events[i];
        let denom = 1u32 << e.denominator_power as u32;
        cell_text(row, format!("{}", i + 1));
        cell_text(row, format!("{}", e.tick));
        cell_text(row, format!("{}/{}", e.numerator, denom));
    });
}

fn show_archive_json(ui: &mut egui::Ui, entry: &ArchiveEntry) {
    ui.label(egui::RichText::new("JSON 数据").size(12.0).strong());
    ui.add_space(2.0);
    if let Ok(obj) = serde_json::from_slice::<serde_json::Value>(&entry.data) {
        let pretty = serde_json::to_string_pretty(&obj).unwrap_or_default();
        let mut text = pretty;
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            ui.add_sized(
                egui::vec2(ui.available_width(), ui.available_height().max(200.0)),
                egui::TextEdit::multiline(&mut text).desired_rows(20).font(egui::TextStyle::Monospace).code_editor(),
            );
        });
    } else {
        show_archive_hexdump(ui, &entry.data);
    }
}

fn show_archive_hexdump(ui: &mut egui::Ui, data: &[u8]) {
    let mut hex = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        hex.push_str(&format!("{:04x}  ", i * 16));
        for b in chunk { hex.push_str(&format!("{:02x} ", b)); }
        hex.push_str("  ");
        for b in chunk {
            let c = *b as char;
            hex.push(if c.is_ascii_graphic() || c == ' ' { c } else { '.' });
        }
        hex.push('\n');
    }
    let mut clone = hex;
    ui.add(egui::TextEdit::multiline(&mut clone).desired_rows(15).font(egui::TextStyle::Monospace).code_editor());
}

fn show_archive_overview(ui: &mut egui::Ui, entries: &[(&String, &ArchiveEntry)]) {
    ui.label(egui::RichText::new("归档结构").size(14.0).strong());
    ui.add_space(4.0);
    let total_bytes: usize = entries.iter().map(|(_, e)| e.data.len()).sum();
    ui.colored_label(egui::Color32::GRAY, format!("{} 个条目, 共 {}", entries.len(), format_size(total_bytes)));
    ui.add_space(4.0);
    let track_count = entries.iter().filter(|(p, _)| p.contains("notes.zst")).count();
    let cc_count = entries.iter().filter(|(p, _)| p.contains("cc_")).count();
    let json_count = entries.iter().filter(|(p, _)| p.ends_with(".json")).count();
    ui.colored_label(egui::Color32::from_gray(120), format!("音符条目: {} 个", track_count));
    ui.colored_label(egui::Color32::from_gray(120), format!("CC 条目: {} 个", cc_count));
    ui.colored_label(egui::Color32::from_gray(120), format!("JSON 条目: {} 个", json_count));
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧条目查看详情");
}

// ═══════════════════════════════════════════════════════════════
//  Shared helpers
// ═══════════════════════════════════════════════════════════════

fn count_notes_for_track(midi: &yinhe_midi::MidiFile, track: u16) -> usize {
    midi.key_notes.iter().map(|kn| kn.iter().filter(|n| n.track == track).count()).sum()
}

fn count_cc_for_track(midi: &yinhe_midi::MidiFile, track: u16) -> BTreeMap<u8, usize> {
    let mut map = BTreeMap::new();
    for ev in &midi.control_events {
        if let MidiControlEvent::ControlChange { track: t, controller, .. } = ev {
            if *t == track { *map.entry(*controller).or_insert(0) += 1; }
        }
    }
    map
}

fn count_pitch_bend(midi: &yinhe_midi::MidiFile, track: u16) -> usize {
    midi.control_events.iter().filter(|e| matches!(e, MidiControlEvent::PitchBend { track: t, .. } if *t == track)).count()
}

fn count_pc(midi: &yinhe_midi::MidiFile, track: u16) -> usize {
    midi.control_events.iter().filter(|e| matches!(e, MidiControlEvent::ProgramChange { track: t, .. } if *t == track)).count()
}

fn collect_notes_for_track(midi: &yinhe_midi::MidiFile, track: u16) -> Vec<(u8, &yinhe_types::Note)> {
    let mut notes = Vec::new();
    for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
        for note in key_notes {
            if note.track == track { notes.push((key_idx as u8, note)); }
        }
    }
    notes.sort_by_key(|(_, n)| n.start_tick);
    notes
}

fn cc_label(controller: u8) -> &'static str {
    match controller {
        0 => "Bank Select MSB", 1 => "Modulation", 7 => "Volume",
        10 => "Pan", 11 => "Expression", 64 => "Sustain",
        91 => "Reverb", 93 => "Chorus", _ => "",
    }
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 { format!("{}B", bytes) }
    else if bytes < 1024 * 1024 { format!("{:.1}K", bytes as f64 / 1024.0) }
    else { format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0)) }
}

/// Try to skip InnerHeader (3 bytes) for track-scoped archive entries.
fn skip_inner_header_if_present(data: &[u8]) -> &[u8] {
    let min_size = yinhe_project::InnerHeader::SIZE;
    if data.len() >= min_size {
        if let Some((_inner, rest)) = yinhe_project::InnerHeader::read(data) {
            return rest;
        }
    }
    data
}

fn archive_header_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(10.0).color(egui::Color32::GRAY));
        ui.label(egui::RichText::new(value).size(10.0).color(egui::Color32::from_gray(200)));
    });
}

// ── Tree row renderers (shared) ──

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
        let events = vec![TypesTimeSigEvent { tick: 0, numerator: 4, denominator: 2 }];
        let bl = BarLookup::build(480, 4, &events);
        assert_eq!(bl.format(0), "1/0");
        assert_eq!(bl.format(480), "1/480");
    }

    #[test]
    fn bar_lookup_time_sig_change() {
        let events = vec![
            TypesTimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TypesTimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ];
        let bl = BarLookup::build(480, 4, &events);
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
}
