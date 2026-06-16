use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_material_icons::icons::*;
use std::collections::BTreeMap;
use yinhe_project::*;
use yinhe_types::TimeSigEvent as TypesTimeSigEvent;

use yinhe_editor_core::document::Document;
use crate::widgets::split_handle;
use crate::theme;

pub struct EventBrowserState {
    pub expanded_paths: std::collections::HashSet<String>,
    pub selected_path: Option<String>,
    archive_fingerprint: Option<usize>,
    /// Top pane height as fraction of total. Memory-persistent only.
    split_ratio: f32,
}

impl Default for EventBrowserState {
    fn default() -> Self {
        Self {
            expanded_paths: Default::default(),
            selected_path: None,
            archive_fingerprint: None,
            split_ratio: 0.45,
        }
    }
}

enum TreeNode {
    Dir {
        children: BTreeMap<String, TreeNode>,
    },
    Leaf {
        full_path: String,
    },
}

impl TreeNode {
    fn new_dir() -> Self {
        TreeNode::Dir {
            children: BTreeMap::new(),
        }
    }

    fn insert(&mut self, segments: &[&str], full_path: &str) {
        let TreeNode::Dir { children } = self else {
            return;
        };
        match segments {
            [] => {}
            [name] => {
                children.insert(
                    (*name).to_string(),
                    TreeNode::Leaf {
                        full_path: full_path.to_string(),
                    },
                );
            }
            [head, rest @ ..] => {
                let entry = children
                    .entry((*head).to_string())
                    .or_insert_with(TreeNode::new_dir);
                entry.insert(rest, full_path);
            }
        }
    }

    fn leaf_count(&self) -> usize {
        match self {
            TreeNode::Leaf { .. } => 1,
            TreeNode::Dir { children } => children.values().map(|c| c.leaf_count()).sum(),
        }
    }
}

fn build_tree(entries: &[(&String, &ArchiveEntry)]) -> TreeNode {
    let mut root = TreeNode::new_dir();
    for (path, _) in entries {
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        root.insert(&segs, path);
    }
    root
}

/// Precomputed bar-counting table for fast tick -> "{bar}/{tick_in_bar}".
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

    let ppq = doc.midi().ticks_per_beat;
    let default_num = doc.midi().time_sig_numerator;
    let ts_events: Vec<TypesTimeSigEvent> = doc.midi().time_sig_events.clone();
    let bar_lookup = BarLookup::build(ppq, default_num, &ts_events);

    let Some(archive) = &doc.archive else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（无工程文件归档）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    };

    let mut entries: Vec<(&String, &ArchiveEntry)> = archive.entries.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let fingerprint = entries.len()
        ^ entries.iter().map(|(_, e)| e.data.len()).sum::<usize>();
    if state.archive_fingerprint != Some(fingerprint) {
        state.expanded_paths.clear();
        let tree_init = build_tree(&entries);
        if let TreeNode::Dir { children } = &tree_init {
            for (name, node) in children {
                if matches!(node, TreeNode::Dir { .. }) {
                    state.expanded_paths.insert(name.clone());
                }
            }
        }
        state.archive_fingerprint = Some(fingerprint);
    }

    let tree = build_tree(&entries);

    let frame_bg = egui::Frame::NONE
        .fill(egui::Color32::from_gray(16))
        .inner_margin(egui::Margin::symmetric(4, 2));

    let total_rect = ui.available_rect_before_wrap();
    let total_h = total_rect.height();
    let gap = theme::SPLIT_GAP;
    let split_y = total_rect.min.y + (total_h * state.split_ratio).round();

    let top_rect = egui::Rect::from_min_max(
        total_rect.min,
        egui::pos2(total_rect.max.x, split_y),
    );
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(total_rect.min.x, split_y),
        egui::pos2(total_rect.max.x, split_y + gap),
    );
    let bot_rect = egui::Rect::from_min_max(
        egui::pos2(total_rect.min.x, split_y + gap),
        total_rect.max,
    );

    // Top: tree
    ui.scope_builder(egui::UiBuilder::new().max_rect(top_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("event_browser_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.vertical(|ui| {
                        if let TreeNode::Dir { children } = &tree {
                            for (name, node) in children {
                                render_node(ui, name, node, "", 0, archive, state);
                            }
                        }
                    });
                });
            });
    });

    // Splitter handle
    let resp = split_handle::horizontal(ui, "__event_browser_split__", handle_rect);
    if resp.dragged() {
        let new_ratio = ((split_y + resp.drag_delta().y - total_rect.min.y) / total_h)
            .clamp(theme::SPLIT_CLAMP_MIN, theme::SPLIT_CLAMP_MAX);
        state.split_ratio = new_ratio;
    }

    // Bottom: detail
    ui.scope_builder(egui::UiBuilder::new().max_rect(bot_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("event_browser_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(sel) = &state.selected_path {
                        if let Some(entry) = archive.entries.get(sel) {
                            show_entry_detail(ui, sel, entry, &bar_lookup);
                        } else {
                            ui.colored_label(egui::Color32::GRAY, "（条目不存在）");
                        }
                    } else {
                        show_root_overview(ui, &entries, archive);
                    }
                });
            });
    });
}

fn render_node(
    ui: &mut egui::Ui,
    name: &str,
    node: &TreeNode,
    parent_path: &str,
    depth: usize,
    archive: &ProjectArchive,
    state: &mut EventBrowserState,
) {
    let cur_path = if parent_path.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent_path, name)
    };
    match node {
        TreeNode::Dir { children } => {
            let expanded = state.expanded_paths.contains(&cur_path);
            let count = node.leaf_count();
            if render_dir_row(ui, name, depth, expanded, count) {
                if expanded {
                    state.expanded_paths.remove(&cur_path);
                } else {
                    state.expanded_paths.insert(cur_path.clone());
                }
            }
            if state.expanded_paths.contains(&cur_path) {
                for (cname, cnode) in children {
                    render_node(ui, cname, cnode, &cur_path, depth + 1, archive, state);
                }
            }
        }
        TreeNode::Leaf { full_path } => {
            let entry = match archive.entries.get(full_path) {
                Some(e) => e,
                None => return,
            };
            render_leaf_row(ui, name, full_path, entry, depth, state);
        }
    }
}

fn render_dir_row(
    ui: &mut egui::Ui,
    name: &str,
    depth: usize,
    expanded: bool,
    child_count: usize,
) -> bool {
    let mut toggled = false;
        egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(depth as f32 * 14.0);

                let chev = if expanded {
                    ICON_EXPAND_MORE
                } else {
                    ICON_CHEVRON_RIGHT
                };
                let chev_resp = ui.add(
                    egui::Label::new(
                        chev.rich_text()
                            .size(13.0)
                            .color(egui::Color32::from_gray(190)),
                    )
                    .sense(egui::Sense::click()),
                );
                if chev_resp.clicked() {
                    toggled = true;
                }

                let folder_icon = if expanded {
                    ICON_FOLDER_OPEN
                } else {
                    ICON_FOLDER
                };
                let folder_resp = ui.add(
                    egui::Label::new(
                        folder_icon
                            .rich_text()
                            .size(13.0)
                            .color(egui::Color32::from_rgb(220, 180, 90)),
                    )
                    .sense(egui::Sense::click()),
                );
                if folder_resp.clicked() {
                    toggled = true;
                }

                ui.label(
                    egui::RichText::new(name)
                        .size(11.0)
                        .color(egui::Color32::from_gray(220)),
                );
                ui.label(
                    egui::RichText::new(format!("({})", child_count))
                        .size(10.0)
                        .color(egui::Color32::from_gray(110)),
                );
            });
        });
    toggled
}

fn render_leaf_row(
    ui: &mut egui::Ui,
    name: &str,
    full_path: &str,
    entry: &ArchiveEntry,
    depth: usize,
    state: &mut EventBrowserState,
) {
    let is_selected = state.selected_path.as_deref() == Some(full_path);
    let icon = match entry.header.magic {
        b if b == *b"YHTK" => ICON_MUSIC_NOTE,
        b if b == *b"YHCC" => ICON_SETTINGS,
        b if b == *b"YHPB" => ICON_EDIT_AUDIO,
        b if b == *b"YHPC" => ICON_PALETTE,
        b if b == *b"YHTM" => ICON_SPEED,
        b if b == *b"YHTS" => ICON_SCHEDULE,
        b if b == *b"YHMP" || b == *b"YHPR" => ICON_AUDIO_FILE,
        _ => ICON_AUDIO_FILE,
    };
    let bg = if is_selected {
        egui::Color32::from_rgb(40, 50, 70)
    } else {
        egui::Color32::TRANSPARENT
    };
    let size_label = if entry.data.len() < 1024 {
        format!("{}B", entry.data.len())
    } else if entry.data.len() < 1024 * 1024 {
        format!("{:.1}K", entry.data.len() as f64 / 1024.0)
    } else {
        format!("{:.1}M", entry.data.len() as f64 / (1024.0 * 1024.0))
    };

    let frame_r =
    egui::Frame::NONE
            .fill(bg)
            .inner_margin(egui::Margin::symmetric(2, 1))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_space(depth as f32 * 14.0);
                    ui.add_space(14.0);
                    ui.label(icon.rich_text().size(12.0).color(if is_selected {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_gray(160)
                    }));
                    ui.label(egui::RichText::new(name).size(11.0).monospace().color(
                        if is_selected {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_gray(200)
                        },
                    ));
                    ui.label(
                        egui::RichText::new(format!("({})", size_label))
                            .size(10.0)
                            .color(egui::Color32::from_gray(110)),
                    );
                });
            });
    let resp = frame_r.response.interact(egui::Sense::click());
    if resp.clicked() {
        state.selected_path = Some(full_path.to_string());
    }
}

/// Returns true if the magic indicates a track-scoped file with InnerHeader.
fn magic_has_inner_header(magic: [u8; 4]) -> bool {
    matches!(&magic, b"YHTK" | b"YHCC" | b"YHPB" | b"YHPC" | b"YHRP")
}

fn show_entry_detail(ui: &mut egui::Ui, _path: &str, entry: &ArchiveEntry, bar_lookup: &BarLookup) {
    let h = entry.header;

    ui.label(egui::RichText::new("文件头").size(12.0).strong());
    ui.add_space(2.0);
    header_row(
        ui,
        "magic",
        &format!(
            "{}{}{}{}",
            h.magic[0] as char, h.magic[1] as char, h.magic[2] as char, h.magic[3] as char
        ),
    );
    header_row(ui, "version", &format!("{}", h.version));
    header_row(ui, "port", &format!("{}", h.port));
    header_row(ui, "channel", &format!("{}", h.channel));
    header_row(ui, "extra", &format!("{}", h.extra));

    ui.add_space(6.0);

    let payload: &[u8] = if magic_has_inner_header(h.magic) && entry.data.len() >= InnerHeader::SIZE
    {
        if let Some((inner, rest)) = InnerHeader::read(&entry.data) {
            ui.label(egui::RichText::new("内头").size(12.0).strong());
            ui.add_space(2.0);
            header_row(ui, "track_index", &format!("{}", inner.track_index));
            header_row(
                ui,
                "channel",
                &format!(
                    "{} (port={}, raw_ch={}, label={})",
                    inner.channel,
                    inner.port(),
                    inner.raw_channel(),
                    channel_label(inner.channel),
                ),
            );
            ui.add_space(6.0);
            rest
        } else {
            &entry.data[..]
        }
    } else {
        &entry.data[..]
    };

    match h.magic {
        b if b == *b"YHTK" => render_notes_table(ui, payload, bar_lookup),
        b if b == *b"YHCC" => render_cc_table(ui, payload, bar_lookup),
        b if b == *b"YHPB" => render_pitch_table(ui, payload, bar_lookup),
        b if b == *b"YHPC" => render_pc_table(ui, payload, bar_lookup),
        b if b == *b"YHTM" => render_tempo_table(ui, payload, bar_lookup),
        b if b == *b"YHTS" => render_time_sig_table(ui, payload, bar_lookup),
        b if b == *b"YHMP" || b == *b"YHPR" => render_json_text(ui, payload),
        b if b == *b"YHRP" => render_rpn_table(ui, payload, bar_lookup, h.extra),
        _ => render_hexdump(ui, payload),
    }
}

fn header_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(10.0)
                .color(egui::Color32::GRAY),
        );
        ui.label(
            egui::RichText::new(value)
                .size(10.0)
                .color(egui::Color32::from_gray(200)),
        );
    });
}

fn render_json_text(ui: &mut egui::Ui, data: &[u8]) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("JSON / 数据").size(12.0).strong());
    ui.add_space(2.0);
    if let Ok(obj) = serde_json::from_slice::<serde_json::Value>(data) {
        let pretty = serde_json::to_string_pretty(&obj).unwrap_or_default();
        let mut text = pretty;
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_sized(
                    egui::vec2(ui.available_width(), ui.available_height().max(200.0)),
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(20)
                        .font(egui::TextStyle::Monospace)
                        .code_editor(),
                );
            });
    } else {
        render_hexdump(ui, data);
    }
}

fn render_hexdump(ui: &mut egui::Ui, data: &[u8]) {
    let mut hex = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        hex.push_str(&format!("{:04x}  ", i * 16));
        for b in chunk {
            hex.push_str(&format!("{:02x} ", b));
        }
        hex.push_str("  ");
        for b in chunk {
            let c = *b as char;
            hex.push(if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '.'
            });
        }
        hex.push('\n');
    }
    let mut clone = hex;
    ui.add(
        egui::TextEdit::multiline(&mut clone)
            .desired_rows(15)
            .font(egui::TextStyle::Monospace)
            .code_editor(),
    );
}

/// Common table builder: header + virtualised body rows.
/// `rows`: total row count. `row_cb(row_idx, row)`: render one row's cells in order.
fn build_table<F>(
    ui: &mut egui::Ui,
    id_salt: &str,
    headers: &[(&str, f32)], // (label, min_width)
    rows: usize,
    mut row_cb: F,
) where
    F: FnMut(usize, &mut egui_extras::TableRow),
{
    let mut tb = TableBuilder::new(ui)
        .id_salt(id_salt)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for (_, min_w) in headers {
        tb = tb.column(Column::initial(*min_w).at_least(40.0).clip(true));
    }
    tb.header(20.0, |mut h| {
        for (label, _) in headers {
            h.col(|ui| {
                ui.label(egui::RichText::new(*label).strong().size(11.0));
            });
        }
    })
    .body(|body| {
        body.rows(18.0, rows, |mut row| {
            let i = row.index();
            row_cb(i, &mut row);
        });
    });
}

fn cell_text(row: &mut egui_extras::TableRow, text: impl Into<String>) {
    let s: String = text.into();
    row.col(|ui| {
        ui.label(egui::RichText::new(s).size(11.0).monospace());
    });
}

fn render_notes_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup) {
    let notes: Vec<Note> = decode_delta_events(payload);
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("音符 ({} 个)", notes.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "notes_table",
        &[
            ("#", 40.0),
            ("tick", 70.0),
            ("位置", 80.0),
            ("结束 tick", 80.0),
            ("结束位置", 90.0),
            ("键位", 50.0),
            ("力度", 50.0),
        ],
        notes.len(),
        |i, row| {
            let n = &notes[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", n.start_tick));
            cell_text(row, bar_lookup.format(n.start_tick));
            cell_text(row, format!("{}", n.end_tick));
            cell_text(row, bar_lookup.format(n.end_tick));
            cell_text(row, format!("{}", n.key));
            cell_text(row, format!("{}", n.velocity));
        },
    );
}

fn render_cc_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup) {
    let events: Vec<CcEvent> = {
        let v: Vec<CcEvent> = decode_delta_events(payload);
        if v.is_empty() && !payload.is_empty() {
            render_hexdump(ui, payload);
            return;
        }
        v
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("CC 事件 ({} 个)", events.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "cc_table",
        &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("值", 60.0)],
        events.len(),
        |i, row| {
            let e = &events[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", e.tick));
            cell_text(row, bar_lookup.format(e.tick));
            cell_text(row, format!("{}", e.value));
        },
    );
}

fn render_pitch_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup) {
    let events: Vec<PitchBendEvent> = {
        let v: Vec<PitchBendEvent> = decode_delta_events(payload);
        if v.is_empty() && !payload.is_empty() {
            render_hexdump(ui, payload);
            return;
        }
        v
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("弯音事件 ({} 个)", events.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "pb_table",
        &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("值", 70.0)],
        events.len(),
        |i, row| {
            let e = &events[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", e.tick));
            cell_text(row, bar_lookup.format(e.tick));
            cell_text(row, format!("{}", e.value));
        },
    );
}

fn render_pc_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup) {
    let events: Vec<PcEvent> = {
        let v: Vec<PcEvent> = decode_delta_events(payload);
        if v.is_empty() && !payload.is_empty() {
            render_hexdump(ui, payload);
            return;
        }
        v
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("音色变更 ({} 个)", events.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "pc_table",
        &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("音色", 50.0), ("Bank MSB", 70.0), ("Bank LSB", 70.0)],
        events.len(),
        |i, row| {
            let e = &events[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", e.tick));
            cell_text(row, bar_lookup.format(e.tick));
            cell_text(row, format!("{}", e.program));
            cell_text(row, if e.bank_msb == 0xFF { "-".into() } else { format!("{}", e.bank_msb) });
            cell_text(row, if e.bank_lsb == 0xFF { "-".into() } else { format!("{}", e.bank_lsb) });
        },
    );
}

fn render_tempo_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup) {
    let events: Vec<TempoEvent> = {
        let v: Vec<TempoEvent> = decode_delta_events(payload);
        if v.is_empty() && !payload.is_empty() {
            render_hexdump(ui, payload);
            return;
        }
        v
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("Tempo ({} 个)", events.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "tempo_table",
        &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("BPM", 70.0)],
        events.len(),
        |i, row| {
            let e = &events[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", e.tick));
            cell_text(row, bar_lookup.format(e.tick));
            cell_text(row, format!("{:.2}", e.bpm));
        },
    );
}

fn render_time_sig_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup) {
    let events: Vec<TimeSigEvent> = {
        let v: Vec<TimeSigEvent> = decode_delta_events(payload);
        if v.is_empty() && !payload.is_empty() {
            render_hexdump(ui, payload);
            return;
        }
        v
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("拍号 ({} 个)", events.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "ts_table",
        &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("拍号", 80.0)],
        events.len(),
        |i, row| {
            let e = &events[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", e.tick));
            cell_text(row, bar_lookup.format(e.tick));
            let denom = 1u32 << e.denominator_power as u32;
            cell_text(row, format!("{}/{}", e.numerator, denom));
        },
    );
}

fn show_root_overview(
    ui: &mut egui::Ui,
    entries: &[(&String, &ArchiveEntry)],
    _archive: &ProjectArchive,
) {
    ui.label(egui::RichText::new("工程文件结构").size(14.0).strong());
    ui.add_space(4.0);
    let total_bytes: usize = entries.iter().map(|(_, e)| e.data.len()).sum();
    ui.colored_label(
        egui::Color32::GRAY,
        format!("{} 个条目, 共 {} 字节", entries.len(), total_bytes),
    );
    ui.add_space(4.0);

    let track_count = entries
        .iter()
        .filter(|(p, _)| p.contains("/notes.zst"))
        .count();
    let cc_count = entries.iter().filter(|(p, _)| p.contains("/cc_")).count();
    let conductor_count = entries
        .iter()
        .filter(|(p, _)| p.starts_with("conductor/"))
        .count();
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("音轨: {} 个", track_count),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("CC: {} 个", cc_count),
    );
    ui.colored_label(
        egui::Color32::from_gray(120),
        format!("指挥: {} 个", conductor_count),
    );

    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧文件查看详情");
}

fn render_rpn_table(ui: &mut egui::Ui, payload: &[u8], bar_lookup: &BarLookup, rpn_num: u8) {
    let events: Vec<RpnEvent> = {
        let v: Vec<RpnEvent> = decode_delta_events(payload);
        if v.is_empty() && !payload.is_empty() {
            render_hexdump(ui, payload);
            return;
        }
        v
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("RPN {} ({} 个)", rpn_num, events.len()))
            .size(12.0)
            .strong(),
    );
    ui.add_space(2.0);
    build_table(
        ui,
        "rpn_table",
        &[("#", 40.0), ("tick", 70.0), ("位置", 80.0), ("RPN", 50.0), ("值", 60.0), ("名称", 160.0)],
        events.len(),
        |i, row| {
            let e = &events[i];
            cell_text(row, format!("{}", i + 1));
            cell_text(row, format!("{}", e.tick));
            cell_text(row, bar_lookup.format(e.tick));
            cell_text(row, format!("{}", rpn_num));
            cell_text(row, format!("{}", e.value));
            cell_text(row, rpn_name(rpn_num));
        },
    );
}

fn rpn_name(rpn_num: u8) -> &'static str {
    match rpn_num {
        0 => "Pitch Bend Sensitivity",
        1 => "Fine Tune",
        2 => "Coarse Tune",
        _ => "Unknown",
    }
}
