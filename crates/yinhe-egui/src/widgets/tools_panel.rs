use egui_material_icons::icons::*;
use rust_i18n::t;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tool {
    Select,
    Pan,
    Pencil,
    Curve,
    Scissors,
    Eraser,
}

/// All currently available tools — shown on the transport bar (right of the timecode).
pub const ALL_TOOLS: [Tool; 6] = [
    Tool::Select,
    Tool::Pan,
    Tool::Pencil,
    Tool::Curve,
    Tool::Scissors,
    Tool::Eraser,
];

impl Tool {
    pub fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            Tool::Select => ICON_SELECT,
            Tool::Pan => ICON_PAN_TOOL,
            Tool::Pencil => ICON_EDIT,
            Tool::Curve => ICON_DRAW,
            Tool::Scissors => ICON_CONTENT_CUT,
            Tool::Eraser => ICON_INK_ERASER,
        }
    }

    pub fn label(self) -> String {
        match self {
            Tool::Select => t!("tool.select").to_string(),
            Tool::Pan => t!("tool.pan").to_string(),
            Tool::Pencil => t!("tool.pencil").to_string(),
            Tool::Curve => t!("tool.curve").to_string(),
            Tool::Scissors => t!("tool.scissors").to_string(),
            Tool::Eraser => t!("tool.eraser").to_string(),
        }
    }
}
