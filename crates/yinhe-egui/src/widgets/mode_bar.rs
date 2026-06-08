use eframe::egui;
use egui_material_icons::icons::*;

#[derive(PartialEq)]
pub enum ViewMode {
    Arrange,
    Mix,
    Edit,
    Rack,
}

fn mode_button(ui: &mut egui::Ui, label: &str, is_selected: bool, on_click: impl FnOnce()) {
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::widgets::theme::MODE_LABEL_FONT)
                .color(if is_selected {
                    crate::widgets::theme::ACCENT_ACTIVE
                } else {
                    egui::Color32::GRAY
                }),
        )
        .sense(egui::Sense::click())
        .selectable(false),
    );
    if !is_selected && resp.hovered() {
        ui.painter().text(
            resp.rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(crate::widgets::theme::MODE_LABEL_FONT),
            egui::Color32::WHITE,
        );
    }
    if resp.clicked() {
        on_click();
    }
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
            fill: crate::widgets::theme::APP_BG,
            ..Default::default()
        })
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(2.0);

                mode_button(ui, "ARRANGE", *view_mode == ViewMode::Arrange, || {
                    *view_mode = ViewMode::Arrange;
                    *show_transport = true;
                    *show_pianoroll = *show_pianoroll_in_arrange;
                });

                ui.add_space(2.0);

                mode_button(ui, "MIX", *view_mode == ViewMode::Mix, || {
                    *view_mode = ViewMode::Mix;
                    *show_transport = false;
                    *show_pianoroll = false;
                });

                ui.add_space(2.0);

                mode_button(ui, "EDIT", *view_mode == ViewMode::Edit, || {
                    *view_mode = ViewMode::Edit;
                    *show_transport = false;
                    *show_pianoroll = true;
                });

                ui.add_space(2.0);

                mode_button(ui, "RACK", *view_mode == ViewMode::Rack, || {
                    *view_mode = ViewMode::Rack;
                    *show_transport = false;
                    *show_pianoroll = false;
                });

                // ── Piano roll toggle ──
                if *view_mode == ViewMode::Arrange {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let piano_color = if *show_pianoroll_in_arrange {
                        crate::widgets::theme::ACCENT_ACTIVE
                    } else {
                        egui::Color32::GRAY
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
