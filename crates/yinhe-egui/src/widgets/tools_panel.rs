use egui_material_icons::icons::*;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tool {
    Select,
    SelectVertical,
    Pan,
    Pencil,
    Curve,
    Scissors,
    Eraser,
}

/// All currently available tools — shown on the transport bar (right of the timecode).
pub const ALL_TOOLS: [Tool; 7] = [
    Tool::Select,
    Tool::SelectVertical,
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
            Tool::SelectVertical => ICON_TEXT_SELECT_START,
            Tool::Pan => ICON_PAN_TOOL,
            Tool::Pencil => ICON_EDIT,
            Tool::Curve => ICON_DRAW,
            Tool::Scissors => ICON_CONTENT_CUT,
            Tool::Eraser => ICON_INK_ERASER,
        }
    }
}
