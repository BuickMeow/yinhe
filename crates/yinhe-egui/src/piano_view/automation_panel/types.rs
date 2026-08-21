use std::sync::Arc;

use eframe::egui;

use yinhe_core::Selection;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::NoteSource;
use yinhe_types::{
    AutomationEdit, AutomationLane, AutomationPanelView, TimeSigEvent, VelocityEdit,
};
use yinhe_wgpu::{AutomationGhost, InstanceRenderer};

use crate::app::layout::SelHintInfo;
use crate::render_context::RenderContext;
use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;

use super::interaction::SelOp;
use super::velocity::VelocityPreview;

/// 交互上下文：打包 `show_panels` 处理编辑所需的全部外部信息.
///
/// `None` 时（如未选中唯一 track）跳过所有编辑交互，仅渲染。
pub struct AutomationEditCtx<'a> {
    pub active_tool: Tool,
    pub active_track: Option<u16>,
    pub quantize: QuantizePreset,
    pub ppq: u32,
    pub bar_line_data: Option<(u32, u8, u8, &'a [TimeSigEvent])>,
}

/// 面板集合渲染状态（panels 与 renderers 一一对应）。
pub(crate) struct PanelsState<'a> {
    pub panels: &'a mut Vec<AutomationPanelView>,
    pub renderers: &'a mut Vec<(InstanceRenderer, RenderContext)>,
    pub wgpu_state: &'a Arc<eframe::egui_wgpu::RenderState>,
    pub show_panels: &'a mut bool,
}

/// 面板布局几何。
#[derive(Clone, Copy)]
pub(crate) struct PanelsLayout {
    pub combo_width: f32,
    pub content_rect_right: f32,
    pub content_top_y: f32,
    pub panels_visible_h: f32,
}

/// 面板渲染/联动配置。
#[derive(Clone, Copy)]
pub(crate) struct PanelsCfg<'a> {
    pub pianoroll_scroll_x: f32,
    pub pianoroll_ppt: f32,
    pub scroll_mode: u32,
    pub min_border_width: f32,
    pub revision: u64,
    /// 状态栏讲解行格式化位置所需（拍号事件）。
    pub bar_line_data: Option<(u32, u8, u8, &'a [TimeSigEvent])>,
    /// 讲解行选框统计（AM 选框命中时显示）。
    pub sel_hint: Option<&'a SelHintInfo>,
    /// 当前编辑目标是否为 Conductor 轨。Conductor 下 AM 面板只可编辑 Tempo
    /// （下拉菜单仅显示 Tempo，非 Tempo 编辑被 dispatch 层禁用）。
    pub editing_is_conductor: bool,
}

/// 面板模型只读数据。
pub(crate) struct PanelsData<'a> {
    pub automation_lanes: &'a [AutomationLane],
    pub render_lanes: &'a [&'a AutomationLane],
    pub tempo_lane: &'a AutomationLane,
    pub midi: Option<&'a dyn NoteSource>,
    pub track_visible: &'a [bool],
    pub track_colors: &'a [[f32; 4]],
}

/// 面板编辑状态。
pub(crate) struct PanelsEdit<'a> {
    pub selected: &'a mut Selection,
    pub info_content: &'a mut Option<InfoContent>,
    pub right_tab: &'a mut Option<RightTab>,
}

/// `show_panels` 的返回：总高度 + 编辑动作列表 + 联动反馈 + (拖拽锚点 tick, value)。
pub(crate) type PanelsOutput = (
    f32,
    Vec<AutomationEdit>,
    Vec<VelocityEdit>,
    PanelPianorollFeedback,
    Option<(u32, f32)>,
);

/// automation 面板交互产生的 pianoroll 联动反馈.
///
/// `show_panels` 返回，由 `piano_view::show` 应用到 pianoroll view。
pub struct PanelPianorollFeedback {
    /// 水平滚动 delta（像素）。非零时 piano_view 会调整 `scroll_x`。
    pub scroll_x_delta: f32,
    /// 水平缩放因子（1.0 = 无缩放）。
    pub zoom_factor: f32,
    /// 缩放中心（pianoroll content 局部 x 坐标，已减去 rect.min.x）。
    pub zoom_center_x: f32,
    /// 状态栏讲解行：鼠标悬停在面板 grid 区时的提示（位置 + 值）。
    pub status_hint: Option<String>,
}

impl Default for PanelPianorollFeedback {
    fn default() -> Self {
        Self {
            scroll_x_delta: 0.0,
            zoom_factor: 1.0,
            zoom_center_x: 0.0,
            status_hint: None,
        }
    }
}

/// 当帧交互产生的临时 overlay 数据。
pub(crate) struct PanelOverlayData {
    pub(crate) marquee_rect: Option<egui::Rect>,
    pub(crate) velocity_preview: Option<VelocityPreview>,
}

/// 单个面板的编辑交互输出。
pub(crate) struct PanelInteractionOut {
    pub(crate) automation_edits: Vec<AutomationEdit>,
    pub(crate) velocity_edits: Vec<VelocityEdit>,
    /// wgpu Layer 3 的 lane ghost（仅 lane 编辑）
    pub(crate) ghost: Option<AutomationGhost>,
    /// velocity 笔划预览（仅 velocity 模式）
    pub(crate) preview: Option<VelocityPreview>,
    /// 锚点拖拽的实时 (tick, value)，供 InfoPanel 显示
    pub(crate) anchor_drag: Option<(u32, f32)>,
    /// Select 工具框选矩形（egui painter 绘制 + 渲染层高亮预览）
    pub(crate) marquee_rect: Option<egui::Rect>,
    /// Select 工具选区变更操作（应用到 panel.anchor_sel_rects）
    pub(crate) sel_op: Option<SelOp>,
}
