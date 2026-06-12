use eframe::egui;
use egui_material_icons::icons::*;
use std::collections::BTreeMap;
use yinhe_project::*;

use crate::document::Document;

#[derive(Default)]
pub struct EventBrowserState {
    pub expanded_paths: std::collections::HashSet<String>,
    pub selected_path: Option<String>,
    archive_fingerprint: Option<usize>,
}

enum TreeNode {
    Dir { children: BTreeMap<String, TreeNode> },
    Leaf { full_path: String },
}

impl TreeNode {
    fn new_dir() -> Self {
        TreeNode::Dir { children: BTreeMap::new() }
    }

    fn insert(&mut self, segments: &[&str], full_path: &str) {
        let TreeNode::Dir { children } = self else { return };
        match segments {
            [] => {}
            [name] => {
                children.insert((*name).to_string(), TreeNode::Leaf { full_path: full_path.to_string() });
            }
            [head, rest @ ..] => {
                let entry = children.entry((*head).to_string()).or_insert_with(TreeNode::new_dir);
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

pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>, state: &mut EventBrowserState) {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("（未打开文档）").color(egui::Color32::from_gray(100)).size(12.0));
        return;
    };

    let Some(archive) = &doc.archive else {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("（无工程文件归档）").color(egui::Color32::from_gray(100)).size(12.0));
        return;
    };

    let mut entries: Vec<(&String, &ArchiveEntry)> = archive.entries.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let fingerprint = entries.len();
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

    let frame_bg = egui::Frame::none()
        .fill(egui::Color32::from_gray(16))
        .inner_margin(egui::Margin::symmetric(4, 2));

    let total_h = ui.available_height();
    let top_h = (total_h * 0.45).max(120.0).min(total_h - 80.0);

    ui.vertical(|ui| {
        egui::ScrollArea::both()
            .id_salt("event_browser_tree")
            .max_height(top_h)
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

        ui.add_space(4.0);

        egui::ScrollArea::both()
            .id_salt("event_browser_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                frame_bg.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(sel) = &state.selected_path {
                        if let Some(entry) = archive.entries.get(sel) {
                            show_entry_detail(ui, sel, entry);
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
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(depth as f32 * 14.0);

                let chev = if expanded { ICON_EXPAND_MORE } else { ICON_CHEVRON_RIGHT };
                let chev_resp = ui.add(
                    egui::Label::new(chev.rich_text().size(13.0).color(egui::Color32::from_gray(190)))
                        .sense(egui::Sense::click()),
                );
                if chev_resp.clicked() {
                    toggled = true;
                }

                let folder_icon = if expanded { ICON_FOLDER_OPEN } else { ICON_FOLDER };
                let folder_resp = ui.add(
                    egui::Label::new(folder_icon.rich_text().size(13.0).color(egui::Color32::from_rgb(220, 180, 90)))
                        .sense(egui::Sense::click()),
                );
                if folder_resp.clicked() {
                    toggled = true;
                }

                ui.label(egui::RichText::new(name).size(11.0).color(egui::Color32::from_gray(220)));
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
        b if b == *b"YHPB" => ICON_CENTER_FOCUS_STRONG,
        b if b == *b"YHPC" => ICON_DESCRIPTION,
        b if b == *b"YHMP" || b == *b"YHPR" || b == *b"YHTM" || b == *b"YHTS" => ICON_AUDIO_FILE,
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

    let frame_r = egui::Frame::none()
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
                ui.label(
                    egui::RichText::new(name)
                        .size(11.0)
                        .monospace()
                        .color(if is_selected {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_gray(200)
                        }),
                );
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
fn show_entry_detail(ui: &mut egui::Ui, path: &str, entry: &ArchiveEntry) {
    let h = entry.header;

    ui.label(egui::RichText::new("文件头").size(12.0).strong());
    ui.add_space(2.0);
    header_row(ui, "magic", &format!("{}{}{}{}", h.magic[0] as char, h.magic[1] as char, h.magic[2] as char, h.magic[3] as char));
    header_row(ui, "version", &format!("{}", h.version));
    header_row(ui, "port", &format!("{}", h.port));
    header_row(ui, "channel", &format!("{}", h.channel));
    header_row(ui, "extra", &format!("{}", h.extra));

    ui.add_space(6.0);

    if entry.data.len() >= 3 {
        if let Some((inner, _rest)) = InnerHeader::read(&entry.data) {
            ui.label(egui::RichText::new("内头").size(12.0).strong());
            ui.add_space(2.0);
            header_row(ui, "track_index", &format!("{}", inner.track_index));
            header_row(ui, "channel", &format!("{} (port={}, raw_ch={})", inner.channel, inner.port(), inner.raw_channel()));
            ui.add_space(6.0);
        }
    }

    match h.magic {
        b if b == *b"YHTK" => render_notes_table(ui, &entry.data),
        b if b == *b"YHCC" => render_cc_table(ui, &entry.data),
        b if b == *b"YHPB" => render_pitch_table(ui, &entry.data),
        b if b == *b"YHPC" => render_pc_table(ui, &entry.data),
        b if b == *b"YHMP" || b == *b"YHPR" || b == *b"YHTM" || b == *b"YHTS" => render_json_text(ui, &entry.data),
        _ => render_hexdump(ui, &entry.data),
    }
}

fn header_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(10.0).color(egui::Color32::GRAY));
        ui.label(egui::RichText::new(value).size(10.0).color(egui::Color32::from_gray(200)));
    });
}

fn render_json_text(ui: &mut egui::Ui, data: &[u8]) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("JSON / 数据").size(12.0).strong());
    ui.add_space(2.0);
    if let Ok(obj) = serde_json::from_slice::<serde_json::Value>(data) {
        let pretty = serde_json::to_string_pretty(&obj).unwrap_or_default();
        let mut text = pretty;
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
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
    let max = data.len().min(512);
    let mut hex = String::new();
    for (i, chunk) in data[..max].chunks(16).enumerate() {
        hex.push_str(&format!("{:04x}  ", i * 16));
        for b in chunk {
            hex.push_str(&format!("{:02x} ", b));
        }
        hex.push_str("  ");
        for b in chunk {
            let c = *b as char;
            hex.push(if c.is_ascii_graphic() || c == ' ' { c } else { '.' });
        }
        hex.push('\n');
    }
    if data.len() > max {
        hex.push_str(&format!("... ({} bytes total)", data.len()));
    }
    let mut clone = hex;
    ui.add(
        egui::TextEdit::multiline(&mut clone)
            .desired_rows(15)
            .font(egui::TextStyle::Monospace)
            .code_editor(),
    );
}

fn render_notes_table(ui: &mut egui::Ui, data: &[u8]) {
    let Some((_inner, notes)) = (|| -> Option<_> {
        let entry = data;
        let (inner, rest) = InnerHeader::read(entry)?;
        Some((inner, decode_notes_delta_gate(rest)))
    })() else {
        render_hexdump(ui, data);
        return;
    };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("音符 ({} 个)", notes.len())).size(12.0).strong());
    ui.add_space(2.0);
    let mut table_text = String::from("tick\t结束\t键位\t力度\n");
    for n in &notes {
        table_text.push_str(&format!("{}\t{}\t{}\t{}\n", n.start_tick, n.end_tick, n.key, n.velocity));
    }
    let mut clone = table_text;
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.add_sized(
            egui::vec2(ui.available_width(), ui.available_height().max(300.0)),
            egui::TextEdit::multiline(&mut clone)
                .desired_rows(notes.len().min(30))
                .font(egui::TextStyle::Monospace)
                .code_editor(),
        );
    });
}

fn render_cc_table(ui: &mut egui::Ui, data: &[u8]) {
    let Some((_inner, events)) = (|| -> Option<_> {
        let (inner, rest) = InnerHeader::read(data)?;
        let ev: Vec<CcEvent> = bincode::deserialize(rest).ok()?;
        Some((inner, ev))
    })() else {
        render_hexdump(ui, data);
        return;
    };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("CC 事件 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    let mut table_text = String::from("tick\t值\n");
    for e in &events {
        table_text.push_str(&format!("{}\t{}\n", e.tick, e.value));
    }
    let mut clone = table_text;
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.add_sized(
            egui::vec2(ui.available_width(), ui.available_height().max(300.0)),
            egui::TextEdit::multiline(&mut clone)
                .desired_rows(events.len().min(30))
                .font(egui::TextStyle::Monospace)
                .code_editor(),
        );
    });
}

fn render_pitch_table(ui: &mut egui::Ui, data: &[u8]) {
    let Some((_inner, events)) = (|| -> Option<_> {
        let (inner, rest) = InnerHeader::read(data)?;
        let ev: Vec<PitchBendEvent> = bincode::deserialize(rest).ok()?;
        Some((inner, ev))
    })() else {
        render_hexdump(ui, data);
        return;
    };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("弯音事件 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    let mut table_text = String::from("tick\t值\n");
    for e in &events {
        table_text.push_str(&format!("{}\t{}\n", e.tick, e.value));
    }
    let mut clone = table_text;
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.add_sized(
            egui::vec2(ui.available_width(), ui.available_height().max(300.0)),
            egui::TextEdit::multiline(&mut clone)
                .desired_rows(events.len().min(30))
                .font(egui::TextStyle::Monospace)
                .code_editor(),
        );
    });
}

fn render_pc_table(ui: &mut egui::Ui, data: &[u8]) {
    let Some((_inner, events)) = (|| -> Option<_> {
        let (inner, rest) = InnerHeader::read(data)?;
        let ev: Vec<PcEvent> = bincode::deserialize(rest).ok()?;
        Some((inner, ev))
    })() else {
        render_hexdump(ui, data);
        return;
    };
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("音色变更 ({} 个)", events.len())).size(12.0).strong());
    ui.add_space(2.0);
    let mut table_text = String::from("tick\t音色\n");
    for e in &events {
        table_text.push_str(&format!("{}\t{}\n", e.tick, e.program));
    }
    let mut clone = table_text;
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.add_sized(
            egui::vec2(ui.available_width(), ui.available_height().max(300.0)),
            egui::TextEdit::multiline(&mut clone)
                .desired_rows(events.len().min(30))
                .font(egui::TextStyle::Monospace)
                .code_editor(),
        );
    });
}

fn show_root_overview(ui: &mut egui::Ui, entries: &[(&String, &ArchiveEntry)], _archive: &ProjectArchive) {
    ui.label(egui::RichText::new("工程文件结构").size(14.0).strong());
    ui.add_space(4.0);
    let total_bytes: usize = entries.iter().map(|(_, e)| e.data.len()).sum();
    ui.colored_label(egui::Color32::GRAY, format!("{} 个条目, 共 {} 字节", entries.len(), total_bytes));
    ui.add_space(4.0);

    // Summary by category
    let track_count = entries.iter().filter(|(p, _)| p.contains("/notes.zst")).count();
    let cc_count = entries.iter().filter(|(p, _)| p.contains("/cc_")).count();
    let conductor_count = entries.iter().filter(|(p, _)| p.starts_with("conductor/")).count();
    ui.colored_label(egui::Color32::from_gray(120), format!("◉ 音轨: {} 个", track_count));
    ui.colored_label(egui::Color32::from_gray(120), format!("CC: {} 个", cc_count));
    ui.colored_label(egui::Color32::from_gray(120), format!("指挥: {} 个", conductor_count));

    ui.add_space(8.0);
    ui.colored_label(egui::Color32::from_gray(100), "← 点击左侧文件查看详情");
}