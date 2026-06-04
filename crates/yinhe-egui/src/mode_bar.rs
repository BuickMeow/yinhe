use eframe::egui;
use egui_material_icons::icons::*;

#[derive(PartialEq)]
pub enum ViewMode {
    Arrange,
    Mix,
    Edit,
}

pub fn show(
    ui: &mut egui::Ui,
    view_mode: &mut ViewMode,
    show_pianoroll_in_arrange: &mut bool,
    show_transport: &mut bool,
    show_pianoroll: &mut bool,
) {
    egui::Panel::bottom("bottom_bar")
        .frame(egui::Frame {
            inner_margin: egui::Margin::symmetric(8, 6),
            fill: egui::Color32::from_rgb(30, 30, 30),
            ..Default::default()
        })
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let active_color = egui::Color32::from_rgb(100, 180, 255);
                let inactive_color = egui::Color32::GRAY;
                let font_size = 9.5;

                ui.add_space(2.0);

                // ── ARRANGE ──
                let arrange_sel = *view_mode == ViewMode::Arrange;
                let arrange_resp = ui.add(
                    egui::Label::new(egui::RichText::new("ARRANGE").size(font_size).color(
                        if arrange_sel {
                            active_color
                        } else {
                            inactive_color
                        },
                    ))
                    .sense(egui::Sense::click())
                    .selectable(false),
                );
                if !arrange_sel && arrange_resp.hovered() {
                    ui.painter().text(
                        arrange_resp.rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "ARRANGE",
                        egui::FontId::proportional(font_size),
                        egui::Color32::WHITE,
                    );
                }
                if arrange_resp.clicked() {
                    *view_mode = ViewMode::Arrange;
                    *show_transport = true;
                    *show_pianoroll = *show_pianoroll_in_arrange;
                }

                ui.add_space(2.0);

                // ── MIX ──
                let mix_sel = *view_mode == ViewMode::Mix;
                let mix_resp = ui.add(
                    egui::Label::new(egui::RichText::new("MIX").size(font_size).color(
                        if mix_sel {
                            active_color
                        } else {
                            inactive_color
                        },
                    ))
                    .sense(egui::Sense::click())
                    .selectable(false),
                );
                if !mix_sel && mix_resp.hovered() {
                    ui.painter().text(
                        mix_resp.rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "MIX",
                        egui::FontId::proportional(font_size),
                        egui::Color32::WHITE,
                    );
                }
                if mix_resp.clicked() {
                    *view_mode = ViewMode::Mix;
                    *show_transport = false;
                    *show_pianoroll = false;
                }

                ui.add_space(2.0);

                // ── EDIT ──
                let edit_sel = *view_mode == ViewMode::Edit;
                let edit_resp = ui.add(
                    egui::Label::new(egui::RichText::new("EDIT").size(font_size).color(
                        if edit_sel {
                            active_color
                        } else {
                            inactive_color
                        },
                    ))
                    .sense(egui::Sense::click())
                    .selectable(false),
                );
                if !edit_sel && edit_resp.hovered() {
                    ui.painter().text(
                        edit_resp.rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "EDIT",
                        egui::FontId::proportional(font_size),
                        egui::Color32::WHITE,
                    );
                }
                if edit_resp.clicked() {
                    *view_mode = ViewMode::Edit;
                    *show_transport = false;
                    *show_pianoroll = true;
                }

                // ── Piano roll toggle ──
                if *view_mode == ViewMode::Arrange {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let piano_color = if *show_pianoroll_in_arrange {
                        active_color
                    } else {
                        inactive_color
                    };
                    let piano_resp = ui.add(
                        egui::Label::new(ICON_PIANO.rich_text().size(14.0).color(piano_color))
                            .sense(egui::Sense::click())
                            .selectable(false),
                    );
                    if !*show_pianoroll_in_arrange && piano_resp.hovered() {
                        ui.painter().text(
                            piano_resp.rect.center(),
                            egui::Align2::CENTER_CENTER,
                            ICON_PIANO.codepoint,
                            egui::FontId::new(14.0, ICON_PIANO.font_family()),
                            egui::Color32::WHITE,
                        );
                    }
                    if piano_resp.clicked() {
                        *show_pianoroll_in_arrange = !*show_pianoroll_in_arrange;
                        *show_pianoroll = *show_pianoroll_in_arrange;
                    }
                }
            });
        });
}
