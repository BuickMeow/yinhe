//! PR 钢琴卷帘页：顶栏（返回 + 走带 + 工具）+ GPU 音符视图。

use eframe::egui;

use crate::app::{Page, Tool, YinheApp};
use crate::pages::transport;
use crate::ui_common::{fill_page_background, icon_text, show_toolbar};

impl YinheApp {
    /// PR 钢琴卷帘页：顶部工具条（返回 + 走带控制 + 工具）+ 视图。
    pub(crate) fn ui_pr(&mut self, ui: &mut egui::Ui) {
        transport::update(self);
        show_toolbar(ui, "pr_toolbar", self.safe_insets, |ui| {
            use egui_material_icons::icons::ICON_ARROW_BACK;
            if ui
                .button(icon_text(ICON_ARROW_BACK))
                .on_hover_text("返回")
                .clicked()
            {
                self.page = Page::Ar;
            }
            ui.label(egui::RichText::new("钢琴卷帘").strong());
            transport::bar(self, ui);
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                fill_page_background(ui);
                self.pr_view.ui(ui, self.safe_insets);
            });
        // 工具选择弹窗：屏幕中央、横向排列（选择/铅笔/橡皮）。
        if self.tool_picker_open {
            egui::Window::new("工具")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        for t in Tool::ALL {
                            let icon = t.icon();
                            let text =
                                egui::RichText::new(format!("{}\n{}", icon.codepoint, t.name()))
                                    .family(icon.font_family())
                                    .size(26.0)
                                    .text_style(egui::TextStyle::Body);
                            if ui.selectable_label(self.tool == t, text).clicked() {
                                self.tool = t;
                                self.tool_picker_open = false;
                            }
                        }
                    });
                });
        }
    }
}
