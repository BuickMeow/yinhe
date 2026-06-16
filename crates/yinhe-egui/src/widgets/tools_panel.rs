use eframe::egui;
use egui_material_icons::icons::*;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tool {
    Select,
    Pan,
    Pencil,
    Scissors,
    Eraser,
}

/// All currently available tools — shown on both panels.
pub const ALL_TOOLS: [Tool; 5] = [
    Tool::Select,
    Tool::Pan,
    Tool::Pencil,
    Tool::Scissors,
    Tool::Eraser,
];

impl Tool {
    pub fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            Tool::Select => ICON_SELECT,
            Tool::Pan => ICON_PAN_TOOL,
            Tool::Pencil => ICON_EDIT,
            Tool::Scissors => ICON_CONTENT_CUT,
            Tool::Eraser => ICON_INK_ERASER,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "选择",
            Tool::Pan => "手形",
            Tool::Pencil => "铅笔",
            Tool::Scissors => "剪刀",
            Tool::Eraser => "橡皮擦",
        }
    }
}

/// Tool panel width (icon + padding).
pub const TOOLS_PANEL_W: f32 = 28.0;

/// Show a vertical tool palette inside `rect`.
///
/// `available_tools` controls which tools appear and in what order.
/// Pass [`ALL_TOOLS`] to show everything.
pub fn show(ui: &mut egui::Ui, rect: egui::Rect, active_tool: &mut Tool, available_tools: &[Tool]) {
    let layout = egui::Layout::top_down(egui::Align::Center);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.set_clip_rect(rect);
        ui.painter()
            .rect_filled(rect, 0.0, crate::theme::APP_BG);

        ui.add_space(4.0);

        for tool in available_tools {
            let is_active = *active_tool == *tool;
            let color = if is_active {
                crate::theme::ACCENT_ACTIVE
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
            crate::widgets::hover::hover_highlight(
                ui,
                &resp,
                tool.icon().codepoint,
                egui::FontId::new(16.0, tool.icon().font_family()),
                is_active,
            );
            resp.on_hover_text(tool.label());

            ui.add_space(2.0);
        }
    });
}
