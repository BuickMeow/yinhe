use serde::{Deserialize, Serialize};

use crate::right_panel_layout::RightPanelLayout;

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
    /// MIX 视图底部是否显示混音台面板（钢琴卷帘/设备链/混音台三选一之一）。
    pub show_mixer_panel: bool,
    /// MIX 视图底部混音台面板高度（px）。
    pub mix_height: f32,
    /// 右栏多栏选项卡布局。
    pub right_panel_layout: RightPanelLayout,
    /// PR 视图方向是否为纵向瀑布流（跨会话持久化，打开新文档时沿用）。
    pub pr_vertical: bool,
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
            show_mixer_panel: true,
            mix_height: 240.0,
            right_panel_layout: RightPanelLayout::default(),
            pr_vertical: false,
        }
    }
}
