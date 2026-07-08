use eframe::egui;

use crate::audio_settings::AudioSettings;
use yinhe_editor_core::document::Document;

use yinhe_editor_core::config::SfEntry;

/// Show the sound-bank (SoundFont) panel.
///
/// Returns `true` if audio should be reloaded (SF config changed).
pub fn show(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    mut doc: Option<&mut Document>,
) -> bool {
    let mut changed = false;

    // ── Top: mode toggle (two text buttons, mutually exclusive) ──
    ui.horizontal(|ui| {
        ui.add_space(8.0);

        let is_global = settings.global_sf_config.global_enabled;

        // "全局音色库" button
        let resp_g = ui.add(
            egui::Label::new(
                egui::RichText::new("全局音色库")
                    .size(crate::theme::MODE_LABEL_FONT)
                    .color(if is_global {
                        crate::theme::ACCENT_ACTIVE
                    } else {
                        egui::Color32::GRAY
                    }),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        crate::widgets::hover::hover_highlight(
            ui,
            &resp_g,
            "全局音色库",
            egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
            is_global,
        );
        if resp_g.clicked() && !is_global {
            settings.global_sf_config.global_enabled = true;
            changed = true;
        }

        ui.add_space(16.0);

        // "歌曲音色库" button
        let resp_p = ui.add(
            egui::Label::new(
                egui::RichText::new("歌曲音色库")
                    .size(crate::theme::MODE_LABEL_FONT)
                    .color(if !is_global {
                        crate::theme::ACCENT_ACTIVE
                    } else {
                        egui::Color32::GRAY
                    }),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        crate::widgets::hover::hover_highlight(
            ui,
            &resp_p,
            "歌曲音色库",
            egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
            !is_global,
        );
        if resp_p.clicked() && is_global {
            settings.global_sf_config.global_enabled = false;
            changed = true;
        }
    });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    // ── Panel content: only one visible at a time ──
    if settings.global_sf_config.global_enabled {
        ui.label(
            egui::RichText::new("所有端口共享同一组音色库")
                .color(egui::Color32::from_gray(140))
                .size(12.0),
        );
        ui.add_space(4.0);
        changed |= global_panel(ui, settings);
    } else {
        if let Some(ref mut doc) = doc {
            changed |= project_panel(ui, doc);
        } else {
            ui.label("（未打开文档）");
        }
    }

    // ── Bottom status bar ──
    ui.add_space(8.0);
    ui.separator();
    ui.horizontal(|ui| {
        if settings.global_sf_config.global_enabled {
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
        } else if let Some(ref doc) = doc {
            let proj_total: usize = doc.edit.project_sf.overrides.iter().map(|(_, e)| e.len()).sum();
            let proj_enabled: usize = doc
                .edit.project_sf
                .overrides
                .iter()
                .flat_map(|(_, e)| e.iter())
                .filter(|e| e.enabled)
                .count();
            ui.label(format!(
                "歌曲: {} SF 文件, {} 已启用",
                proj_total, proj_enabled
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("重新加载音频").clicked() {
                changed = true;
            }
        });
    });

    if changed {
        settings.save();
    }

    changed
}

// ── Global panel (no port selector — all ports share ports[0]) ──

fn global_panel(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

    // SF list — always edit ports[0]
    let entries = &mut settings.global_sf_config.ports[0];
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
        if ui.button("清空").clicked() {
            entries.clear();
            changed = true;
        }
    });

    changed
}

// ── Project panel (per-port SF lists) ──

fn project_panel(ui: &mut egui::Ui, doc: &mut Document) -> bool {
    let mut changed = false;

    // Derive used ports directly from track port fields. (Old code looked at
    // track_channels which packed port|channel into a u8.)
    let max_port = {
        let mut max_p = 0u8;
        for t in &doc.data.model.tracks {
            if t.port > max_p {
                max_p = t.port;
            }
        }
        max_p
    };
    let num_ports = (max_port + 1).max(1);
    let used_ports: Vec<u8> = (0..num_ports).collect();

    // Port selector — persist selection in Document so it survives frames.
    let port_names: Vec<String> = used_ports
        .iter()
        .map(|&p| format!("Port {}", (b'A' + p) as char))
        .collect();

    let mut selected_port = doc.edit.soundfont_selected_port as usize;
    selected_port = selected_port.min(port_names.len().saturating_sub(1));
    egui::ComboBox::from_id_salt("project_port")
        .selected_text(&port_names[selected_port])
        .show_ui(ui, |ui| {
            for (i, name) in port_names.iter().enumerate() {
                if ui.selectable_label(i == selected_port, name).clicked() {
                    selected_port = i;
                }
            }
        });
    doc.edit.soundfont_selected_port = selected_port as u8;
    let port = used_ports[selected_port];

    if let Some(idx) = doc
        .edit.project_sf
        .overrides
        .iter()
        .position(|(p, _)| *p == port)
    {
        let entries = &mut doc.edit.project_sf.overrides[idx].1;
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

        if ui.button("清空此 Port").clicked() {
            doc.edit.project_sf.overrides[idx].1.clear();
            changed = true;
        }
    } else {
        ui.label(
            egui::RichText::new("（未配置音色库）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        ui.add_space(4.0);
        if ui.button("为此 Port 添加音色库").clicked() {
            doc.edit.project_sf.overrides.push((port, Vec::new()));
            changed = true;
        }
    }

    changed
}
