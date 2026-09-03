use serde::{Deserialize, Serialize};

/// 用户可拖拽调整的布局状态（跨会话持久化）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    pub right_panel_width: f32,
    pub arr_split: f32,
    pub transport_panel_width: f32,
    pub show_pianoroll_in_arrange: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            right_panel_width: 320.0,
            arr_split: 0.3,
            transport_panel_width: 200.0,
            show_pianoroll_in_arrange: false,
        }
    }
}
