use egui_material_icons::icons::*;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tool {
    Select,
    SelectVertical,
    Pan,
    Pencil,
    Scissors,
    Eraser,
    Grid,
    Line,
    Brush,
}

/// All currently available tools — shown on the transport bar (right of the timecode).
pub const ALL_TOOLS: [Tool; 9] = [
    Tool::Select,
    Tool::SelectVertical,
    Tool::Pan,
    Tool::Pencil,
    Tool::Scissors,
    Tool::Eraser,
    Tool::Grid,
    Tool::Line,
    Tool::Brush,
];

impl Tool {
    pub fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            Tool::Select => ICON_SELECT,
            Tool::SelectVertical => ICON_TEXT_SELECT_START,
            Tool::Pan => ICON_PAN_TOOL,
            Tool::Pencil => ICON_EDIT,
            Tool::Scissors => ICON_CONTENT_CUT,
            Tool::Eraser => ICON_INK_ERASER,
            Tool::Grid => ICON_GRID_ON,
            Tool::Line => ICON_PENTAGON,
            Tool::Brush => ICON_BRUSH,
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
            Tool::Scissors => sc::ACTION_TOOL_SCISSORS,
            Tool::Eraser => sc::ACTION_TOOL_ERASER,
            Tool::Grid => sc::ACTION_TOOL_GRID,
            Tool::Line => sc::ACTION_TOOL_LINE,
            Tool::Brush => sc::ACTION_TOOL_BRUSH,
        }
    }

    /// 在 [`ALL_TOOLS`] 中的下标（图钉等按固定下标存储）。
    pub fn pin_index(self) -> usize {
        ALL_TOOLS.iter().position(|&t| t == self).unwrap_or(0)
    }

    /// 工具在工具菜单/提示里显示的名称 i18n key。
    pub fn label_key(self) -> &'static str {
        match self {
            Tool::Select => "tool.select",
            Tool::SelectVertical => "tool.select_vertical",
            Tool::Pan => "tool.pan",
            Tool::Pencil => "tool.pencil",
            Tool::Scissors => "tool.scissors",
            Tool::Eraser => "tool.eraser",
            Tool::Grid => "tool.grid",
            Tool::Line => "tool.line",
            Tool::Brush => "tool.brush",
        }
    }
}
