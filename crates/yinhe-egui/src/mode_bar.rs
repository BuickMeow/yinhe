use eframe::egui;

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

                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new("ARRANGE")
                                .size(font_size)
                                .strong()
                                .color(if *view_mode == ViewMode::Arrange {
                                    active_color
                                } else {
                                    inactive_color
                                }),
                        )
                        .sense(egui::Sense::click())
                        .selectable(false),
                    )
                    .clicked()
                {
                    *view_mode = ViewMode::Arrange;
                    *show_transport = true;
                    *show_pianoroll = *show_pianoroll_in_arrange;
                }

                ui.add_space(2.0);

                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new("MIX")
                                .size(font_size)
                                .strong()
                                .color(if *view_mode == ViewMode::Mix {
                                    active_color
                                } else {
                                    inactive_color
                                }),
                        )
                        .sense(egui::Sense::click())
                        .selectable(false),
                    )
                    .clicked()
                {
                    *view_mode = ViewMode::Mix;
                    *show_transport = false;
                    *show_pianoroll = false;
                }

                ui.add_space(2.0);

                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new("EDIT")
                                .size(font_size)
                                .strong()
                                .color(if *view_mode == ViewMode::Edit {
                                    active_color
                                } else {
                                    inactive_color
                                }),
                        )
                        .sense(egui::Sense::click())
                        .selectable(false),
                    )
                    .clicked()
                {
                    *view_mode = ViewMode::Edit;
                    *show_transport = false;
                    *show_pianoroll = true;
                }

                if *view_mode == ViewMode::Arrange {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    let emoji_color = if *show_pianoroll_in_arrange {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_gray(80)
                    };
                    if ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new("🎹")
                                    .size(font_size)
                                    .color(emoji_color),
                            )
                            .sense(egui::Sense::click())
                            .selectable(false),
                        )
                        .clicked()
                    {
                        *show_pianoroll_in_arrange = !*show_pianoroll_in_arrange;
                        *show_pianoroll = *show_pianoroll_in_arrange;
                    }
                }
            });
        });
}
