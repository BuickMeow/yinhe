use egui_material_icons::icons::*;

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

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "选择",
            Tool::Pan => "手形",
            Tool::Pencil => "铅笔",
            Tool::Curve => "曲线",
            Tool::Scissors => "剪刀",
            Tool::Eraser => "橡皮擦",
        }
    }
}
