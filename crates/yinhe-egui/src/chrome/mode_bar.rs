use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::right_panel::RightTab;

#[derive(PartialEq)]
pub enum ViewMode {
    Arrange,
    Mix,
    Edit,
}

impl ViewMode {
    /// 是否显示 arrange（transport）区域。
    #[inline]
    pub fn show_transport(&self) -> bool {
        matches!(self, ViewMode::Arrange)
    }

    /// 是否显示 piano roll 区域。
    /// `show_pianoroll_in_arrange` 是用户偏好：Arrange 模式下是否同时显示 PR。
    #[inline]
    pub fn show_pianoroll(&self, show_pianoroll_in_arrange: bool) -> bool {
        match self {
            ViewMode::Arrange => show_pianoroll_in_arrange,
            ViewMode::Mix => false,
            ViewMode::Edit => true,
        }
    }
}

fn mode_button(ui: &mut egui::Ui, label: &str, is_selected: bool, on_click: impl FnOnce()) {
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::theme::MODE_LABEL_FONT)
                .color(if is_selected {
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
        &resp,
        label,
        egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
        is_selected,
    );
    if resp.clicked() {
        on_click();
    }
}

fn right_icon_button(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    is_active: bool,
    on_click: impl FnOnce(),
) {
    let color = if is_active {
        crate::theme::ACCENT_ACTIVE
    } else {
        egui::Color32::GRAY
    };
    let resp = ui.add(
        egui::Label::new(icon.rich_text().size(14.0).color(color))
            .sense(egui::Sense::click())
            .selectable(false),
    );
    crate::widgets::hover::hover_highlight(
        ui,
        &resp,
        icon.codepoint,
        egui::FontId::new(14.0, icon.font_family()),
        is_active,
    );
    if resp.clicked() {
        on_click();
    }
}

/// A compact "LABEL value" readout. Both label and value at `MODE_LABEL_FONT`.
fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    metric_with_value_sz(ui, label, value, crate::theme::MODE_LABEL_FONT);
}

fn metric_with_value_sz(ui: &mut egui::Ui, label: &str, value: &str, value_sz: f32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::theme::MODE_LABEL_FONT)
                .color(egui::Color32::GRAY),
        )
        .selectable(false),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .size(value_sz)
                .color(crate::theme::ACCENT_ACTIVE),
        )
        .selectable(false),
    );
}

/// Like [`metric`], but the value is clickable (e.g. to open a detail popup).
fn metric_clickable(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    on_click: impl FnOnce(),
) {
    metric_clickable_with_value_sz(ui, label, value, crate::theme::MODE_LABEL_FONT, on_click);
}

fn metric_clickable_with_value_sz(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    value_sz: f32,
    on_click: impl FnOnce(),
) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::theme::MODE_LABEL_FONT)
                .color(egui::Color32::GRAY),
        )
        .selectable(false),
    );
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .size(value_sz)
                .color(crate::theme::ACCENT_ACTIVE),
        )
        .sense(egui::Sense::click())
        .selectable(false),
    );
    let resp = resp.on_hover_text("点击打开内存占用详情");
    if resp.clicked() {
        on_click();
    }
}

pub fn show(
    ui: &mut egui::Ui,
    view_mode: &mut ViewMode,
    show_pianoroll_in_arrange: &mut bool,
    right_tab: &mut Option<RightTab>,
    cpu_usage: f32,
    mem_mb: f64,
    fps: f32,
    show_mem_breakdown: &mut bool,
) {
    egui::Panel::bottom("bottom_bar")
        .frame(egui::Frame {
            inner_margin: egui::Margin::symmetric(8, 6),
            fill: crate::theme::APP_BG,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(2.0);

                mode_button(ui, t!("mode.arrange").as_ref(), *view_mode == ViewMode::Arrange, || {
                    *view_mode = ViewMode::Arrange;
                });

                ui.add_space(2.0);

                mode_button(ui, t!("mode.mix").as_ref(), *view_mode == ViewMode::Mix, || {
                    *view_mode = ViewMode::Mix;
                });

                ui.add_space(2.0);

                mode_button(ui, t!("mode.edit").as_ref(), *view_mode == ViewMode::Edit, || {
                    *view_mode = ViewMode::Edit;
                });

                // ── Piano roll toggle ──
                if *view_mode == ViewMode::Arrange {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let piano_color = if *show_pianoroll_in_arrange {
                        crate::theme::ACCENT_ACTIVE
                    } else {
                        egui::Color32::GRAY
                    };
                    let piano_resp = ui.add(
                        egui::Label::new(ICON_PIANO.rich_text().size(14.0).color(piano_color))
                            .sense(egui::Sense::click())
                            .selectable(false),
                    );
                    crate::widgets::hover::hover_highlight(
                        ui,
                        &piano_resp,
                        ICON_PIANO.codepoint,
                        egui::FontId::new(14.0, ICON_PIANO.font_family()),
                        *show_pianoroll_in_arrange,
                    );
                    if piano_resp.clicked() {
                        *show_pianoroll_in_arrange = !*show_pianoroll_in_arrange;
                    }
                }

                // ── Spacer: push right icons to the right edge ──
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Right-most first (from right to left):
                    //  1. ICON_INFO
                    //  2. ICON_ALBUM
                    //  3. ICON_MUSIC_CAST
                    //  4. ICON_SHUFFLE
                    //  5. ICON_AUTO_STORIES (event browser)

                    right_icon_button(ui, ICON_INFO, *right_tab == Some(RightTab::Info), || {
                        *right_tab = if *right_tab == Some(RightTab::Info) {
                            None
                        } else {
                            Some(RightTab::Info)
                        };
                    });

                    ui.add_space(4.0);

                    right_icon_button(
                        ui,
                        ICON_ALBUM,
                        *right_tab == Some(RightTab::Project),
                        || {
                            *right_tab = if *right_tab == Some(RightTab::Project) {
                                None
                            } else {
                                Some(RightTab::Project)
                            };
                        },
                    );

                    ui.add_space(4.0);

                    right_icon_button(
                        ui,
                        ICON_MUSIC_CAST,
                        *right_tab == Some(RightTab::SoundFont),
                        || {
                            *right_tab = if *right_tab == Some(RightTab::SoundFont) {
                                None
                            } else {
                                Some(RightTab::SoundFont)
                            };
                        },
                    );

                    ui.add_space(4.0);

                    right_icon_button(
                        ui,
                        ICON_SHUFFLE,
                        *right_tab == Some(RightTab::Channels),
                        || {
                            *right_tab = if *right_tab == Some(RightTab::Channels) {
                                None
                            } else {
                                Some(RightTab::Channels)
                            };
                        },
                    );

                    ui.add_space(4.0);

                    right_icon_button(
                        ui,
                        ICON_FOLDER_ZIP,
                        *right_tab == Some(RightTab::EventBrowser),
                        || {
                            *right_tab = if *right_tab == Some(RightTab::EventBrowser) {
                                None
                            } else {
                                Some(RightTab::EventBrowser)
                            };
                        },
                    );

                    // ── Resource metrics (CPU / MEM / FPS) — left of the right icons ──
                    ui.separator();
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        metric(ui, "CPU", &format!("{:.1}%", cpu_usage));
                        ui.add_space(12.0);
                        let ctx_clone = ui.ctx().clone();
                        metric_clickable(ui, "MEM", &format!("{:.1} MB", mem_mb), || {
                            *show_mem_breakdown = true;
                            crate::chrome::dialog::raise_viewport(
                                &ctx_clone,
                                egui::ViewportId::from_hash_of("memory_breakdown_dialog"),
                            );
                        });
                        ui.add_space(12.0);
                        metric(ui, "FPS", &format!("{:.1}", fps));
                    });
                });
            });
        });
}
