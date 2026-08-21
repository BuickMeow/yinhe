use eframe::egui;

use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};

use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;

use super::interaction;
use super::types::{AutomationEditCtx, PanelInteractionOut};
use super::velocity;

/// 按面板模式分派编辑交互：Tempo / CC / PB / RPN / NRPN 走 lane 编辑；
/// Velocity 走铅笔笔划（改音符力度）。同时负责绘制 hover/drag tooltip。
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_edit_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &mut AutomationPanelView,
    automation_lanes: &[AutomationLane],
    tempo_lane: &AutomationLane,
    midi: Option<&dyn yinhe_types::NoteSource>,
    edit_ctx: Option<&AutomationEditCtx<'_>>,
    panel_index: usize,
    track_colors: &[[f32; 4]],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
    pr_sel_rect: &yinhe_editor_core::edit_state::SelRectState,
    pr_track_selected: &std::collections::HashSet<u16>,
) -> PanelInteractionOut {
    let mut out = PanelInteractionOut {
        automation_edits: Vec::new(),
        velocity_edits: Vec::new(),
        ghost: None,
        preview: None,
        anchor_drag: None,
        marquee_rect: None,
        sel_op: None,
    };
    let mut tooltip: Option<interaction::HoverTooltip> = None;
    if let Some(ctx) = edit_ctx {
        if panel.show_velocity {
            // Velocity：铅笔/选框笔划修改力度条（命中 noteon，只作用于 active_track）
            if matches!(
                ctx.active_tool,
                Tool::Pencil | Tool::Select | Tool::SelectVertical
            ) && let Some(track) = ctx.active_track
                && let Some(midi_src) = midi
            {
                let track_color = track_colors
                    .get(track as usize)
                    .copied()
                    .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
                let (vel_edits, preview, tip) = velocity::handle_velocity_interaction(
                    ui,
                    grid_area,
                    panel_rect,
                    panel,
                    midi_src,
                    track,
                    track_color,
                    panel_index,
                    pr_sel_rect,
                    pr_track_selected,
                );
                out.velocity_edits = vel_edits;
                out.preview = preview;
                tooltip = tip.map(|(tick, value, pos)| interaction::HoverTooltip::Anchor {
                    tick,
                    value,
                    pos,
                });
            }
        } else if panel.selected_target == AutomationTarget::Tempo {
            // Tempo 不依赖 active_track：无论编辑目标是哪个轨道（甚至没有编辑目标）
            // 都可编辑。document 层忽略 track_idx，直接操作 conductor.tempo，
            // 所以这里传 0。非 Tempo 事件绝不能落进 Conductor（曾导致弯音写入别的轨道）。
            let (panel_edits, ghost, drag_info, hover_info, marquee_rect, sel_op) =
                interaction::handle_automation_interaction(
                    ui,
                    grid_area,
                    panel_rect,
                    panel,
                    automation_lanes,
                    Some(tempo_lane),
                    0,
                    ctx,
                    ui.id().with(panel_index),
                    track_colors,
                    info_content,
                    right_tab,
                );
            out.automation_edits = panel_edits;
            out.ghost = ghost;
            out.marquee_rect = marquee_rect;
            out.sel_op = sel_op;
            // anchor_drag 只跟锚点拖拽（InfoPanel 用它显示实时 tick/value）
            if let Some(interaction::HoverTooltip::Anchor { tick, value, .. }) = drag_info {
                out.anchor_drag = Some((tick, value));
            }
            tooltip = drag_info.or(hover_info);
        } else if let Some(track) = ctx.active_track {
            let (panel_edits, ghost, drag_info, hover_info, marquee_rect, sel_op) =
                interaction::handle_automation_interaction(
                    ui,
                    grid_area,
                    panel_rect,
                    panel,
                    automation_lanes,
                    Some(tempo_lane),
                    track,
                    ctx,
                    ui.id().with(panel_index),
                    track_colors,
                    info_content,
                    right_tab,
                );
            out.automation_edits = panel_edits;
            out.ghost = ghost;
            out.marquee_rect = marquee_rect;
            out.sel_op = sel_op;
            // anchor_drag 只跟锚点拖拽（InfoPanel 用它显示实时 tick/value）
            if let Some(interaction::HoverTooltip::Anchor { tick, value, .. }) = drag_info {
                out.anchor_drag = Some((tick, value));
            }
            tooltip = drag_info.or(hover_info);
        }
    }

    // tooltip：拖拽中显示 drag_info，否则 hover 锚点/控制点超时显示 hover_info。
    if let (Some(tip), Some(ctx)) = (tooltip, edit_ctx) {
        let (lines, x, y): (Vec<String>, f32, f32) = match tip {
            interaction::HoverTooltip::Anchor { tick, value, pos } => {
                let pos_str = if let Some((ppq, num, den, ts_events)) = ctx.bar_line_data {
                    format_tick_bar_beat_with_time_sig(tick as f64, ppq, ts_events, num, den)
                } else {
                    format!("{}", tick)
                };
                let val_str = if panel.show_velocity {
                    format!("{}", value.round() as i32)
                } else if panel.selected_target == AutomationTarget::Tempo {
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
    out
}
