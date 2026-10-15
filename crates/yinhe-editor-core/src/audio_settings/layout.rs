use serde::{Deserialize, Serialize};

use crate::right_panel_layout::RightPanelLayout;

/// 单个底部面板的「浮动窗口」状态（跨会话持久化）。
///
/// 与 `show_*` 停靠开关互斥：`floating = true` 时面板以独立 OS 窗口显示，
/// 停靠形态关闭；`floating = false` 时由 `show_*` 决定停靠/隐藏。
/// 同一面板任意时刻只渲染一份（钢琴卷帘的 GPU 视图只允许单实例）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FloatPanelState {
    /// 是否以独立窗口形态显示。
    pub floating: bool,
    /// 窗口内区矩形 `[x, y, w, h]`（逻辑像素）；`None` = 使用默认尺寸。
    pub rect: Option<[f32; 4]>,
}

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
    /// 钢琴卷帘浮动窗口（泊出为独立窗口）。
    pub float_pianoroll: FloatPanelState,
    /// 底部设备栏浮动窗口。
    pub float_bottom_dock: FloatPanelState,
    /// 混音台浮动窗口。
    pub float_mixer: FloatPanelState,
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
            mix_height: 480.0,
            float_pianoroll: FloatPanelState::default(),
            float_bottom_dock: FloatPanelState::default(),
            float_mixer: FloatPanelState::default(),
            right_panel_layout: RightPanelLayout::default(),
            pr_vertical: false,
        }
    }
}
