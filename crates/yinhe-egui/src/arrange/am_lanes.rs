//! AR（编排视图）展开自动化 lane 的交互与渲染数据收集。
//!
//! 复用 PR 自动化面板的 handle_automation_interaction 与 wgpu 的
//! prepare_arr_automation：每条可见的 AM 行（展开音轨的子 lane +
//! Conductor 主行的 Tempo 直显）独立跑一遍交互，结果汇总给 view_ui。
//!
//! 坐标约定：
//! - 交互层用 lane 局部 y（lview.y_offset = 0，panel_rect = lane 屏幕矩形）；
//! - 渲染层用 AR 纹理坐标（y_offset = 子行顶部 y），Curve ghost 的 y 在此处
//!   从局部平移到纹理坐标。

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui;

use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{ArRow, ArRowLayout, AutomationLane, AutomationPanelView, AutomationTarget};
use yinhe_types::{ArrangementView, AutomationEdit};
use yinhe_wgpu::AutomationGhost;

use crate::piano_view::automation_panel::AutomationEditCtx;
use crate::piano_view::automation_panel::interaction::{self, SelOp, SelRectOp};
use crate::right_panel::{InfoContent, RightTab};

/// 一条需要交互/渲染的 AM 行。
#[derive(Clone, Copy, Debug)]
pub(crate) struct AmRowRef {
    /// 所属音轨索引。
    pub track: usize,
    /// 该轨 automation_lanes 的下标；None = Conductor 主行（Tempo 直显）。
    pub sub: Option<usize>,
    /// 行号（ArRowLayout 行空间）。
    pub row: usize,
}

/// 收集可视范围内的 AM 行（展开 lane + Conductor 主行）。
pub(crate) fn visible_am_rows(
    layout: &ArRowLayout,
    first_row: usize,
    last_row: usize,
    conductor: Option<u16>,
) -> Vec<AmRowRef> {
    let mut out = Vec::new();
    for row in first_row..last_row.min(layout.total_rows()) {
        match layout.row_hit(row) {
            Some(ArRow::Automation(track, sub)) => out.push(AmRowRef {
                track,
                sub: Some(sub),
                row,
            }),
            Some(ArRow::Track(track)) if conductor == Some(track as u16) => out.push(AmRowRef {
                track,
                sub: None,
                row,
            }),
            _ => {}
        }
    }
    out
}

/// AM lane 的值域上限：Tempo 由实际事件动态计算（与 PR 的 panel_max_val 一致），
/// 其他 target 用 max_value()。
pub(crate) fn lane_max_val(lane: &AutomationLane) -> f32 {
    if lane.target == AutomationTarget::Tempo {
        lane.events
            .iter()
            .map(|e| e.value)
            .fold(0.0_f32, f32::max)
            .max(1.0)
    } else {
        lane.target.max_value()
    }
}

/// 高亮锚点 tick 集合（Select 框选的锚点 + Pencil 点选的 info_content 锚点）。
/// 与 PR automation_panel.rs 的 highlight 计算逻辑一致。
pub(crate) fn lane_highlight_ticks(
    lane: &AutomationLane,
    track: u16,
    sel_rects: &[yinhe_types::AnchorSelRect],
    info_content: &Option<InfoContent>,
) -> Box<[u32]> {
    let mut out: Vec<u32> = lane
        .events
        .iter()
        .filter(|e| sel_rects.iter().any(|r| r.contains(e.tick, e.value)))
        .map(|e| e.tick)
        .collect();
    if let Some(InfoContent::Anchor {
        target,
        track_idx,
        event_idx,
        ..
    }) = info_content
        && *target == lane.target
        && (lane.target == AutomationTarget::Tempo || *track_idx == track)
        && let Some(tick) = lane.events.get(*event_idx).map(|e| e.tick)
        && !out.contains(&tick)
    {
        out.push(tick);
    }
    out.into_boxed_slice()
}

/// 交互所需的外部状态（view_ui 每帧组装）。
pub(crate) struct AmLanesIo<'a> {
    pub tracks: &'a [Arc<yinhe_core::TrackData>],
    pub tempo_lane: &'a AutomationLane,
    pub track_colors: &'a [[f32; 4]],
    pub selected: &'a mut yinhe_core::Selection,
    pub info_content: &'a mut Option<InfoContent>,
    pub right_tab: &'a mut Option<RightTab>,
    /// 每 lane 持久视图状态（锚点选框等），key = (音轨, target)。
    pub am_views: &'a mut HashMap<(u16, AutomationTarget), AutomationPanelView>,
    /// 收集到的编辑（由 arrange.rs 应用到 Document）。
    pub edits: &'a mut Vec<AutomationEdit>,
}

/// interact_all 的返回：拖拽 ghost（纹理坐标）+ Select/Eraser 框选矩形。
#[derive(Default)]
pub(crate) struct AmInteractionOut {
    /// (ghost, 所在行顶部纹理 y, 行高, max_val)。
    pub ghost: Option<(AutomationGhost, f32, f32, f32)>,
    pub marquee: Option<egui::Rect>,
}

/// 对所有可见 AM 行跑一遍编辑交互（Pencil/Select/SelectVertical/Curve/Eraser）。
///
/// 必须在 AR 的音符选框/橡皮擦处理之外按行区域互斥（view_ui 负责 gating）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn interact_all(
    ui: &mut egui::Ui,
    rows: &[AmRowRef],
    view: &ArrangementView,
    rect: egui::Rect,
    music_rect: egui::Rect,
    am_ctx: &AutomationEditCtx<'_>,
    io: &mut AmLanesIo<'_>,
) -> AmInteractionOut {
    let mut out = AmInteractionOut::default();
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;

    for r in rows {
        let (lanes, tempo_lane, target): (
            &[AutomationLane],
            Option<&AutomationLane>,
            AutomationTarget,
        ) = match r.sub {
            Some(sub) => {
                let Some(track) = io.tracks.get(r.track) else {
                    continue;
                };
                let Some(lane) = track.automation_lanes.get(sub) else {
                    continue;
                };
                (track.automation_lanes.as_slice(), None, lane.target.clone())
            }
            None => (
                &[] as &[AutomationLane],
                Some(io.tempo_lane),
                AutomationTarget::Tempo,
            ),
        };

        let y_top = r.row as f32 * lh - scroll_y; // 纹理坐标
        let sy = rect.min.y + y_top; // 屏幕坐标
        let lane_rect = egui::Rect::from_min_max(
            egui::pos2(music_rect.min.x, sy),
            egui::pos2(music_rect.max.x, sy + lh),
        );

        // 每 lane 持久视图状态：anchor_sel_rects 跨帧保留，其余字段每帧同步。
        let key = (r.track as u16, target.clone());
        let lview = io.am_views.entry(key.clone()).or_default();
        lview.base.scroll_x = view.base.scroll_x;
        lview.base.pixels_per_tick = view.base.pixels_per_tick;
        lview.base.left_panel_width = view.base.left_panel_width;
        lview.panel_height = lh;
        lview.y_offset = 0.0; // 交互用 panel 局部坐标
        lview.value_zoom = 1.0;
        lview.value_scroll = 0.0;
        lview.selected_target = target.clone();
        lview.show_velocity = false;

        let id_base = ui.id().with(("arr_am_lane", r.track, r.sub));
        let (edits, ghost, drag_info, hover_info, marquee, sel_op) =
            interaction::handle_automation_interaction(
                ui,
                lane_rect,
                lane_rect,
                lview,
                lanes,
                tempo_lane,
                r.track as u16,
                am_ctx,
                id_base,
                io.track_colors,
                io.info_content,
                io.right_tab,
            );
        io.edits.extend(edits);

        // 应用 Select 工具的选区变更（与 PR show_panels 一致）
        if let Some(op) = sel_op {
            let lview = io.am_views.entry(key).or_default();
            match op {
                SelOp::Set(rect_op) => match rect_op {
                    SelRectOp::Set(r) => lview.anchor_sel_rects = vec![r],
                    SelRectOp::Append(r) => lview.anchor_sel_rects.push(r),
                    SelRectOp::ReplaceAll(rects) => lview.anchor_sel_rects = rects,
                    SelRectOp::Keep => {}
                },
                SelOp::Clear => lview.anchor_sel_rects.clear(),
                SelOp::ClearNoteSelection => {
                    // 三视图选框互斥：开始新框选时清空共享音符选区
                    io.selected.clear();
                }
            }
        }

        if let Some(mr) = marquee {
            out.marquee = Some(mr);
        }

        if let Some(g) = ghost {
            // Curve ghost 的 y 是 lane 局部坐标，平移到纹理坐标；
            // Move ghost 携带整条 lane，渲染时按 y_offset 重建，无需平移。
            let g = match g {
                AutomationGhost::Curve {
                    start_x,
                    start_y,
                    cur_x,
                    cur_y,
                    color,
                } => AutomationGhost::Curve {
                    start_x,
                    start_y: start_y + y_top,
                    cur_x,
                    cur_y: cur_y + y_top,
                    color,
                },
                other => other,
            };
            let max_val = match r.sub {
                Some(sub) => io
                    .tracks
                    .get(r.track)
                    .and_then(|t| t.automation_lanes.get(sub))
                    .map(lane_max_val)
                    .unwrap_or(1.0),
                None => lane_max_val(io.tempo_lane),
            };
            out.ghost = Some((g, y_top, lh, max_val));
        }

        // tooltip：拖拽中显示 drag_info，否则 hover 超时显示 hover_info（与 PR 一致）。
        if let Some(tip) = drag_info.or(hover_info) {
            let (lines, x, y): (Vec<String>, f32, f32) = match tip {
                interaction::HoverTooltip::Anchor { tick, value, pos } => {
                    let pos_str = if let Some((ppq, num, den, ts_events)) = am_ctx.bar_line_data {
                        format_tick_bar_beat_with_time_sig(tick as f64, ppq, ts_events, num, den)
                    } else {
                        format!("{}", tick)
                    };
                    let val_str = if target == AutomationTarget::Tempo {
                        format!("{:.2} BPM", value)
                    } else {
                        format!("{:.2}", value)
                    };
                    (vec![pos_str, val_str], pos.x, pos.y)
                }
                interaction::HoverTooltip::ControlPoint {
                    x1,
                    y1,
                    x2,
                    y2,
                    pos,
                } => (
                    vec![
                        format!("X1: {:.2}", x1),
                        format!("Y1: {:.2}", y1),
                        format!("X2: {:.2}", x2),
                        format!("Y2: {:.2}", y2),
                    ],
                    pos.x,
                    pos.y,
                ),
            };
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, x, y);
        }
    }

    out
}

/// lane 的菜单/标签名（子行绘制与右键菜单共用）。
pub(crate) fn lane_label(target: &AutomationTarget) -> String {
    match target {
        AutomationTarget::CC { controller } => format!("CC {:03}", controller),
        other => other.display_name(),
    }
}
