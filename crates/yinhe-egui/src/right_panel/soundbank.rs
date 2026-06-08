use eframe::egui;

use crate::dialogs::settings::AudioSettings;
use crate::document::Document;

use super::config::SfEntry;

/// Show the sound-bank (SoundFont) panel.
///
/// Returns `true` if audio should be reloaded (SF config changed).
pub fn show(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    mut doc: Option<&mut Document>,
) -> bool {
    let mut changed = false;

    // ── Top: global / project toggle ──
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let glbl = &mut settings.global_sf_config.global_enabled;
        ui.checkbox(glbl, "全局音色库");
        ui.add_space(16.0);
        if let Some(ref mut doc) = doc {
            ui.checkbox(&mut doc.project_sf.project_enabled, "歌曲配置（覆盖）");
        } else {
            ui.label("歌曲配置（覆盖）");
        }
        ui.add_space(8.0);
    });

    ui.add_space(4.0);

    // ── Split: global left, project right ──
    let avail = ui.available_size();
    let half_w = (avail.x - 8.0) / 2.0;

    ui.horizontal(|ui| {
        // ── Left: Global ──
        ui.group(|ui| {
            ui.set_min_width(half_w);
            ui.set_max_width(half_w);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("全局音色库").strong().size(14.0));
                ui.add_space(4.0);
                changed |= global_panel(ui, settings);
            });
        });

        ui.add_space(8.0);

        // ── Right: Project ──
        ui.group(|ui| {
            ui.set_min_width(half_w);
            ui.set_max_width(half_w);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("歌曲配置").strong().size(14.0));
                ui.add_space(4.0);
                if let Some(ref mut doc) = doc {
                    changed |= project_panel(ui, doc);
                } else {
                    ui.label("（未打开文档）");
                }
            });
        });
    });

    // ── Bottom status bar ──
    ui.add_space(8.0);
    ui.separator();
    ui.horizontal(|ui| {
        let total: usize = settings
            .global_sf_config
            .ports
            .iter()
            .map(|p| p.len())
            .sum();
        let enabled: usize = settings
            .global_sf_config
            .ports
            .iter()
            .flat_map(|p| p.iter())
            .filter(|e| e.enabled)
            .count();
        ui.label(format!("全局: {} SF 文件, {} 已启用", total, enabled));
        if let Some(ref doc) = doc {
            let proj_total: usize = doc.project_sf.overrides.iter().map(|(_, e)| e.len()).sum();
            let proj_enabled: usize = doc
                .project_sf
                .overrides
                .iter()
                .flat_map(|(_, e)| e.iter())
                .filter(|e| e.enabled)
                .count();
            ui.label(format!(
                "  歌曲: {} SF 文件, {} 已启用",
                proj_total, proj_enabled
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("重新加载音频").clicked() {
                changed = true;
            }
        });
    });

    changed
}

// ── Global panel ──

fn global_panel(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    let port_count = 16;
    let active_ports: Vec<usize> = (0..port_count)
        .filter(|&p| !settings.global_sf_config.ports[p].is_empty() || p == 0)
        .collect();

    // Port selector
    let port_names: Vec<String> = active_ports
        .iter()
        .map(|&p| format!("Port {}", (b'A' + p as u8) as char))
        .collect();
    let port_idx = {
        let p = settings
            .global_sf_config
            .ports
            .iter()
            .position(|entries| !entries.is_empty())
            .unwrap_or(0);
        active_ports.iter().position(|&ap| ap == p).unwrap_or(0)
    };

    let mut selected_port = port_idx;
    egui::ComboBox::from_id_salt("global_port")
        .selected_text(&port_names[port_idx])
        .show_ui(ui, |ui| {
            for (i, name) in port_names.iter().enumerate() {
                if ui.selectable_label(i == port_idx, name).clicked() {
                    selected_port = i;
                }
            }
        });
    let port = active_ports[selected_port];

    // SF list
    let entries = &mut settings.global_sf_config.ports[port];
    changed |= super::sf_list::sf_list(ui, entries);

    // Toolbar
    ui.horizontal(|ui| {
        if ui.button("＋ 添加").clicked() {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("SoundFont", &["sf2", "sf3", "sfz"])
                .pick_files()
            {
                for path in paths {
                    let name = path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .unwrap_or("SoundFont")
                        .to_string();
                    entries.push(SfEntry {
                        path: path.to_string_lossy().to_string(),
                        name,
                        enabled: true,
                    });
                }
                changed = true;
            }
        }
        if ui.button("清空此 Port").clicked() {
            entries.clear();
            changed = true;
        }
    });

    changed
}

// ── Project panel ──

fn project_panel(ui: &mut egui::Ui, doc: &mut Document) -> bool {
    let mut changed = false;

    // Gather ports actually used by the MIDI file
    let used_ports: Vec<u8> = {
        let mut seen = std::collections::BTreeSet::new();
        for &p in &doc.midi.track_ports {
            if p < 16 {
                seen.insert(p as u8);
            }
        }
        if seen.is_empty() {
            seen.insert(0);
        }
        seen.into_iter().collect()
    };

    if used_ports.is_empty() {
        ui.label("（当前 MIDI 未使用任何端口）");
        return false;
    }

    // Port selector
    let port_names: Vec<String> = used_ports
        .iter()
        .map(|&p| format!("Port {}", (b'A' + p) as char))
        .collect();

    let mut selected_port = 0_usize;
    egui::ComboBox::from_id_salt("project_port")
        .selected_text(&port_names[0])
        .show_ui(ui, |ui| {
            for (i, name) in port_names.iter().enumerate() {
                if ui.selectable_label(i == 0, name).clicked() {
                    selected_port = i;
                }
            }
        });
    let port = used_ports[selected_port];

    let has_override = doc.project_sf.overrides.iter().any(|(p, _)| *p == port);

    if has_override {
        let ov_idx = doc
            .project_sf
            .overrides
            .iter()
            .position(|(p, _)| *p == port)
            .unwrap();
        let entries = &mut doc.project_sf.overrides[ov_idx].1;
        changed |= super::sf_list::sf_list(ui, entries);

        ui.horizontal(|ui| {
            if ui.button("＋ 添加").clicked() {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("SoundFont", &["sf2", "sf3", "sfz"])
                    .pick_files()
                {
                    for path in paths {
                        let name = path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or("SoundFont")
                            .to_string();
                        entries.push(SfEntry {
                            path: path.to_string_lossy().to_string(),
                            name,
                            enabled: true,
                        });
                    }
                    changed = true;
                }
            }
        });

        if ui.button("清除覆盖").clicked() {
            if let Some(idx) = doc
                .project_sf
                .overrides
                .iter()
                .position(|(p, _)| *p == port)
            {
                doc.project_sf.overrides.remove(idx);
                changed = true;
            }
        }
    } else {
        ui.label(
            egui::RichText::new("（继承自全局配置）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        ui.add_space(4.0);
        if ui.button("为此 Port 创建覆盖").clicked() {
            doc.project_sf.overrides.push((port, Vec::new()));
        }
    }

    changed
}
