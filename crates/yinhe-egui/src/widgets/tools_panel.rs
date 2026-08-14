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

    /// 工具切换快捷键的动作 id（与 `shortcuts::ACTION_TOOL_*` 对应）。
    pub fn action_id(self) -> &'static str {
        use yinhe_editor_core::shortcuts as sc;
        match self {
            Tool::Select => sc::ACTION_TOOL_SELECT,
            Tool::SelectVertical => sc::ACTION_TOOL_SELECT_VERTICAL,
            Tool::Pan => sc::ACTION_TOOL_PAN,
            Tool::Pencil => sc::ACTION_TOOL_PENCIL,
            Tool::Curve => sc::ACTION_TOOL_CURVE,
            Tool::Scissors => sc::ACTION_TOOL_SCISSORS,
            Tool::Eraser => sc::ACTION_TOOL_ERASER,
        }
    }
}
