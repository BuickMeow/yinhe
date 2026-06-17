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
    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let default_num = model.tempo_map.time_sig_default.0;
    let ts_events: Vec<TypesTimeSigEvent> = model.tempo_map.time_sig_events.iter().map(|e| TypesTimeSigEvent { tick: e.tick, numerator: e.numerator, denominator: e.denominator }).collect();
    let bar_lookup = BarLookup::build(ppq, default_num, &ts_events);

    let fingerprint = model.tick_length
        ^ (model.note_count << 16)
        ^ (model.tracks.len() as u64).wrapping_mul(0x9E3779B9);
    if state.midi_fingerprint != Some(fingerprint) {
        state.expanded_tracks.clear();
        for i in 0..model.tracks.len() {
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
                    ui.vertical(|ui| render_realtime_tree(ui, model, state));
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
                        show_realtime_detail(ui, sel, model, &bar_lookup);
                    } else {
                        show_realtime_overview(ui, model);
                    }
                });
            });
    });
}

// ── Realtime tree ──

fn render_realtime_tree(ui: &mut egui::Ui, model: &yinhe_core::YinModel, state: &mut EventBrowserState) {
    let num_tracks = model.tracks.len();

    if !model.conductor.tempo.is_empty() || !model.conductor.time_sig.is_empty() {
        let expanded = state.expanded_tracks.contains(&u16::MAX)
            || state.selected_item.as_ref().map(|s| matches!(s, SelectedItem::Tempo | SelectedItem::TimeSig)).unwrap_or(false);
        let tempo_count = model.conductor.tempo.len();
        let ts_count = model.conductor.time_sig.len();
        let child_count = tempo_count + ts_count;

        if render_dir_row(ui, "Conductor", 0, expanded, child_count) {
            if expanded { state.expanded_tracks.remove(&u16::MAX); }
            else { state.expanded_tracks.insert(u16::MAX); }
        }
        if expanded {
            if !model.conductor.tempo.is_empty() {
                render_leaf_item(ui, &format!("Tempo ({})", tempo_count), ICON_SPEED, 1, SelectedItem::Tempo, state);
            }
            if !model.conductor.time_sig.is_empty() {
                render_leaf_item(ui, &format!("TimeSig ({})", ts_count), ICON_SCHEDULE, 1, SelectedItem::TimeSig, state);
            }
        }
    }

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let t = track_idx as u16;
        let name = track.name.clone();
        let ch = track.channel;
        let note_count = track.notes.len();
        let cc_map: std::collections::BTreeMap<u8, usize> = track.cc.iter().map(|(&c, v)| (c, v.len())).collect();
        let pb = track.pitch_bend.len();
        let pc = track.program_change.len();
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

fn show_realtime_detail(ui: &mut egui::Ui, item: &SelectedItem, model: &yinhe_core::YinModel, bar_lookup: &BarLookup) {
    match item {
        SelectedItem::Tempo => {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Tempo ({} 个)", model.conductor.tempo.len())).size(12.0).strong());
            ui.add_space(2.0);
            build_table(ui, "eb_tempo", &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("BPM", 70.0)], model.conductor.tempo.len(), |i, row| {
                let s = &model.conductor.tempo[i];
                let bpm = s.bpm;
                cell_text(row, format!("{}", i + 1));
                cell_text(row, format!("{}", s.tick));
                cell_text(row, bar_lookup.format(s.tick as u32));
                cell_text(row, format!("{:.2}", bpm));
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

fn show_realtime_overview(ui: &mut egui::Ui, model: &yinhe_core::YinModel) {
    ui.label(egui::RichText::new("工程概览").size(14.0).strong());
    ui.add_space(4.0);
    let mut cc = 0usize;
    let mut pb = 0usize;
    let mut pc = 0usize;
    for t in &model.tracks {
        cc += t.cc.values().map(|v| v.len()).sum::<usize>();
        pb += t.pitch_bend.len();
        pc += t.program_change.len();
    }
    ui.colored_label(egui::Color32::from_gray(120), format!("轨道: {} 个", model.tracks.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("音符: {} 个", model.note_count));
    ui.colored_label(egui::Color32::from_gray(120), format!("CC: {} 个", cc));
    ui.colored_label(egui::Color32::from_gray(120), format!("弯音: {} 个", pb));
    ui.colored_label(egui::Color32::from_gray(120), format!("音色变更: {} 个", pc));
    ui.colored_label(egui::Color32::from_gray(120), format!("Tempo: {} 个", model.conductor.tempo.len()));
    ui.colored_label(egui::Color32::from_gray(120), format!("拍号: {} 个", model.conductor.time_sig.len()));
    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧条目查看详情");
}

// ═══════════════════════════════════════════════════════════════
//  Archive view (reads from ProjectArchive)
// ═══════════════════════════════════════════════════════════════

fn show_archive(ui: &mut egui::Ui, doc: &mut Document, state: &mut EventBrowserState) {
    // Display YinModel structure directly as a tree view.
    ui.heading("YinModel Structure");
    ui.separator();
    
    let model = &doc.data.model;
    ui.label(format!("Tracks: {}", model.tracks.len()));
    ui.label(format!("Conductor: {} tempo, {} time-sig",
        model.conductor.tempo.len(),
        model.conductor.time_sig.len()
    ));
    ui.label(format!("Note count: {}", model.note_count));
    ui.label(format!("Tick length: {}", model.tick_length));
    
    ui.separator();
    ui.label("Track list:");
    
    for (i, track) in model.tracks.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("  [{}] {} (ch {}, port {})",
                i, track.name, track.channel, track.port));
            ui.label(format!("{} notes", track.notes.len()));
            ui.label(format!("{} CCs", track.cc.len()));
            ui.label(format!("{} PB", track.pitch_bend.len()));
            ui.label(format!("{} PC", track.program_change.len()));
            ui.label(format!("{} RPN", track.rpn.len()));
        });
    }
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 { format!("{}B", bytes) }
    else if bytes < 1024 * 1024 { format!("{:.1}K", bytes as f64 / 1024.0) }
    else { format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0)) }
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
