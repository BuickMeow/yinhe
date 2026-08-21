//! 钢琴卷帘基础类型：事件、自动化上下文、预览请求与反馈通道。

use std::sync::Arc;

use yinhe_types::{AutomationLane, AutomationPanelView, VelocityEdit};

use crate::widgets::selection_actions::SelectionAction;

/// 钢琴卷帘视图对外发出的事件。
pub enum PianoViewEvent {
    SelectionAction(SelectionAction),
    AddNote {
        track: u16,
        note: yinhe_core::NoteEvent,
    },
    EraserDelete {
        t_start: u32,
        t_end: u32,
        key_lo: u8,
        key_hi: u8,
        track_lo: u16,
        track_hi: u16,
    },
    QuickDelete {
        track: u16,
        start_tick: u32,
        key: u8,
    },
}

/// Automation panel 上下文（all-or-nothing：要么全 Some 要么全 None）。
/// 合并 5 个 auto_* 参数，减少 piano_view::show 的参数数量。
pub struct AutomationPanelsCtx<'a> {
    pub panels: &'a mut Vec<AutomationPanelView>,
    pub renderers: &'a mut Vec<(
        yinhe_wgpu::InstanceRenderer,
        crate::render_context::RenderContext,
    )>,
    pub lanes: &'a [AutomationLane],
    /// 渲染用 lanes：所有 PR 可见音轨的 lanes（与音符显示逻辑一致）。
    /// `lanes` 仅为主音轨的编辑目标，渲染不受其限制。
    pub render_lanes: &'a [&'a AutomationLane],
    pub show: &'a mut bool,
    pub wgpu_state: &'a Arc<eframe::egui_wgpu::RenderState>,
}

/// 音符听觉预览请求（UI 交互 → App → AudioCommand）。
/// 预览音从目标音轨的通道发出，通道状态按目标位置（target_tick）的自动化。
pub(crate) enum PreviewReq {
    /// 播放/重触发一个音符预览。`duration_ticks == 0` 表示持续音（直到 `Stop`）。
    Note(NotePreview),
    /// 停止持续音预览。
    Stop,
}

/// 单个音符的预览参数。
pub(crate) struct NotePreview {
    pub track: u16,
    pub key: u8,
    /// `None` = 用该音轨最近修改力度（default_velocity）。
    pub velocity: Option<u8>,
    /// 目标位置 tick：自动化状态采样点（音符起点）。
    pub target_tick: u32,
    /// 预览时长（tick），0 = 持续音（配合 `PreviewReq::Stop`）。
    pub duration_ticks: u32,
}

/// piano_view 给外部的反馈通道（合并多个 &mut 出参）。
pub struct PianoViewFeedback<'a> {
    pub auto_edit_events: &'a mut Vec<super::automation_panel::AutomationEdit>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
    pub right_tab: &'a mut Option<crate::right_panel::RightTab>,
    pub automation_drag_ghost: &'a mut Option<(u32, f32)>,
    pub note_drag_delta: &'a mut Option<(i64, i32, bool)>,
    pub pencil_note_drag: &'a mut Option<yinhe_types::PencilNoteDrag>,
    /// 选框边缘拖动伸缩：(side, delta_ticks)。dt 按量化对齐。
    pub note_resize_delta: &'a mut Option<(yinhe_editor_core::ResizeSide, i64)>,
    pub velocity_edits: &'a mut Vec<VelocityEdit>,
    /// 音符听觉预览请求（铅笔新建/拖拽、选框拖拽触发）。
    pub preview_reqs: &'a mut Vec<PreviewReq>,
    /// 状态栏讲解行：钢琴卷帘悬停提示（位置 + 音高）。
    pub status_hint: &'a mut Option<String>,
    /// 控制栏事件（量化/切换主音轨/显示音轨勾选），由 layout 应用。
    pub bar_events: &'a mut Vec<super::control_bar::PrBarEvent>,
}

/// 钢琴卷帘顶部时间标尺高度（占位常量，实际值来自 theme）。
pub const RULER_H: f32 = crate::theme::RULER_H;
