//! project.json / mapping.json 详情。

use egui_material_icons::icons::{ICON_CANCEL, ICON_CHECK_CIRCLE};

use super::*;

// ── project.json / mapping.json ──

/// 键值行（project.json / mapping.json / track_detail 共用）。
fn kv(ui: &mut egui::Ui, k: &str, v: String) {
    crate::widgets::rows::value_row(ui, k, v);
}

pub(super) fn show_project_json(ui: &mut egui::Ui, doc: &Document) {
    let pf = &doc.data.project_file;
    crate::widgets::rows::section_header(ui, "project.json");

    kv(ui, "version", format!("{}", pf.version));
    kv(ui, "name", pf.name.clone());
    kv(ui, "artist", pf.artist.clone());
    kv(ui, "description", pf.description.clone());
    kv(ui, "ppq", format!("{}", pf.ppq));
    kv(ui, "compression_level", format!("{}", pf.compression_level));
    kv(
        ui,
        "soundfont_channels",
        format!("{}", pf.sf_channel_overrides.len()),
    );

    if !pf.sf_channel_overrides.is_empty() {
        ui.add_space(crate::theme::GAP_SM);
        ui.label(
            egui::RichText::new("soundfont_overrides")
                .strong()
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(crate::theme::text_bright()),
        );
        for po in &pf.sf_channel_overrides {
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::INDENT_STEP);
                ui.label(
                    egui::RichText::new(format!("channel {}:", po.channel))
                        .size(crate::scaling::scaled_font(
                            ui.ctx(),
                            crate::theme::SMALL_FONT,
                        ))
                        .color(crate::theme::text_label()),
                );
            });
            for entry in &po.entries {
                ui.horizontal(|ui| {
                    ui.add_space(crate::theme::INDENT_STEP * 2.0);
                    let (icon, color) = if entry.enabled {
                        (ICON_CHECK_CIRCLE, crate::theme::accent_active())
                    } else {
                        (ICON_CANCEL, crate::theme::text_disabled())
                    };
                    ui.label(crate::widgets::icon_text::icon_text(
                        icon,
                        &format!("{} ({})", entry.name, entry.path),
                        crate::scaling::scaled_font(ui.ctx(), crate::theme::SMALL_LABEL_FONT),
                        color,
                    ));
                });
            }
        }
    }
}

pub(super) fn show_mapping_json(ui: &mut egui::Ui, doc: &Document) {
    let mf = &doc.data.mapping_file;
    crate::widgets::rows::section_header(ui, "mapping.json");

    kv(ui, "version", format!("{}", mf.version));

    ui.add_space(crate::theme::GAP_SM);
    ui.label(
        egui::RichText::new("ports")
            .strong()
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::SMALL_FONT,
            ))
            .color(crate::theme::text_bright()),
    );
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
}
