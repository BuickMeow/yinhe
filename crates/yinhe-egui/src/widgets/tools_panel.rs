use eframe::egui;
use egui_material_icons::icons::*;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tool {
    Select,
    Pencil,
    Scissors,
}

impl Tool {
    pub fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            Tool::Select => ICON_SELECT,
            Tool::Pencil => ICON_EDIT,
            Tool::Scissors => ICON_CONTENT_CUT,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "选择",
            Tool::Pencil => "铅笔",
            Tool::Scissors => "剪刀",
        }
    }
}

/// Tool panel width (icon + padding).
pub const TOOLS_PANEL_W: f32 = 28.0;

/// Show the vertical tool palette.
///
/// `rect` is the full area allocated for the tool buttons.
pub fn show(ui: &mut egui::Ui, rect: egui::Rect, active_tool: &mut Tool) {
    let layout = egui::Layout::top_down(egui::Align::Center);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.set_clip_rect(rect);
        ui.painter()
            .rect_filled(rect, 0.0, crate::widgets::theme::APP_BG);

        ui.add_space(4.0);

        for tool in &[Tool::Select, Tool::Pencil, Tool::Scissors] {
            let is_active = *active_tool == *tool;
            let color = if is_active {
                crate::widgets::theme::ACCENT_ACTIVE
            } else {
                egui::Color32::GRAY
            };
            let resp = ui.add(
                egui::Label::new(tool.icon().rich_text().size(16.0).color(color))
                    .sense(egui::Sense::click())
                    .selectable(false),
            );
            if resp.clicked() {
                *active_tool = *tool;
            }
            // Hover highlight
            if !is_active && resp.hovered() {
                ui.painter().text(
                    resp.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    tool.icon().codepoint,
                    egui::FontId::new(16.0, tool.icon().font_family()),
                    egui::Color32::WHITE,
                );
            }
            resp.on_hover_text(tool.label());

            ui.add_space(2.0);
        }
    });
}
