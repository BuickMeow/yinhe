use eframe::egui;
use egui_material_icons::icons::ICON_ADD;

use crate::audio_settings::AudioSettings;
use rust_i18n::t;
use yinhe_editor_core::document::Document;

use yinhe_editor_core::config::SfEntry;

/// 构造带 Material Icon 的 "添加" 按钮文本。
/// 图标码点用 material-icons 家族渲染（否则被 Pretendard/MiSans 的
/// PUA 私有字形抢占，显示成奇怪的方框/数字），文字走 Proportional。
fn add_button_text() -> egui::WidgetText {
    let label = t!("common.add");
    crate::widgets::icon_text::icon_text(ICON_ADD, label.as_ref(), 12.0, egui::Color32::PLACEHOLDER)
}

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
    // 上方留 8px 与面板顶/顶栏分隔线拉开，下方 separator 前不额外加宽，
    // 避免上窄下宽；字号与面板其他文字（12px）一致。
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);

        let is_global = settings.global_sf_config.global_enabled;

        // "全局音色库" button
        let resp_g = crate::widgets::hover::hover_button(
            ui,
            t!("soundfont.global").as_ref(),
            egui::FontId::proportional(crate::theme::BODY_FONT),
            crate::theme::text_label(),
            is_global,
        );
        if resp_g.clicked() && !is_global {
            settings.global_sf_config.global_enabled = true;
            changed = true;
        }

        ui.add_space(16.0);

        // "歌曲音色库" button
        let resp_p = crate::widgets::hover::hover_button(
            ui,
            t!("soundfont.project").as_ref(),
            egui::FontId::proportional(crate::theme::BODY_FONT),
            crate::theme::text_label(),
            !is_global,
        );
        if resp_p.clicked() && is_global {
            settings.global_sf_config.global_enabled = false;
            changed = true;
        }
    });

    ui.separator();
    ui.add_space(4.0);

    // ── Panel content: only one visible at a time ──
    if settings.global_sf_config.global_enabled {
        ui.label(
            egui::RichText::new(t!("soundfont.global_hint").as_ref())
                .color(crate::theme::text_faint())
                .size(crate::theme::BODY_FONT),
        );
        ui.add_space(4.0);
        changed |= global_panel(ui, settings);
    } else {
        if let Some(ref mut doc) = doc {
            changed |= project_panel(ui, doc);
        } else {
            ui.label(t!("common.no_document").as_ref());
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
            ui.label(t!("soundfont.global_status", total = total, enabled = enabled).to_string());
        } else if let Some(ref doc) = doc {
            let proj_total: usize = doc
                .edit
                .project_sf
                .overrides
                .iter()
                .map(|(_, e)| e.len())
                .sum();
            let proj_enabled: usize = doc
                .edit
                .project_sf
                .overrides
                .iter()
                .flat_map(|(_, e)| e.iter())
                .filter(|e| e.enabled)
                .count();
            ui.label(
                t!(
                    "soundfont.project_status",
                    total = proj_total,
                    enabled = proj_enabled
                )
                .to_string(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(t!("soundfont.reload_audio").as_ref()).clicked() {
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

    // Toolbar 必须在列表上方：sf_list 的 ScrollArea 占满全部可用高度，
    // 放在后面的按钮会被挤出可视区（空列表时无法添加第一个音色库）。
    ui.horizontal(|ui| {
        let entries = &mut settings.global_sf_config.ports[0];
        if ui.button(add_button_text()).clicked()
            && let Some(paths) = rfd::FileDialog::new()
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
        if ui.button(t!("common.clear").as_ref()).clicked() {
            entries.clear();
            changed = true;
        }
    });
    ui.add_space(4.0);

    // SF list — always edit ports[0]
    let entries = &mut settings.global_sf_config.ports[0];
    changed |= super::sf_list::sf_list(ui, entries, "global");

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
        .edit
        .project_sf
        .overrides
        .iter()
        .position(|(p, _)| *p == port)
    {
        let entries = &mut doc.edit.project_sf.overrides[idx].1;

        // Toolbar 必须在列表上方（sf_list 的 ScrollArea 占满剩余高度，
        // 后面的按钮会被挤出可视区，空列表时无法添加第一个音色库）。
        ui.horizontal(|ui| {
            if ui.button(add_button_text()).clicked()
                && let Some(paths) = rfd::FileDialog::new()
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
            if ui.button(t!("soundfont.clear_port").as_ref()).clicked() {
                entries.clear();
                changed = true;
            }
        });
        ui.add_space(4.0);

        changed |= super::sf_list::sf_list(ui, entries, &format!("port_{port}"));
    } else {
        crate::widgets::hint::empty_hint(ui, t!("soundfont.not_configured").as_ref());
        if ui.button(t!("soundfont.add_for_port").as_ref()).clicked() {
            doc.edit.project_sf.overrides.push((port, Vec::new()));
            changed = true;
        }
    }

    changed
}
