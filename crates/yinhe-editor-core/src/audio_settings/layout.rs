use serde::{Deserialize, Serialize};

/// 用户可拖拽调整的布局状态（跨会话持久化）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    pub right_panel_width: f32,
    pub arr_split: f32,
    pub transport_panel_width: f32,
    pub show_pianoroll_in_arrange: bool,
    /// 三视图通用底部设备栏是否展开。
    pub show_bottom_dock: bool,
    /// 底部设备栏高度（px）。
    pub bottom_dock_height: f32,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            right_panel_width: 320.0,
            arr_split: 0.3,
            transport_panel_width: 200.0,
            show_pianoroll_in_arrange: false,
            show_bottom_dock: false,
            bottom_dock_height: 200.0,
        }
    }
}
