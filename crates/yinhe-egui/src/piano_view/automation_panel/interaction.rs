//! Automation panel mouse interaction logic (pencil/curve tools, right-click).

mod curve;
mod drag;
mod eraser;
mod ghost;
mod hit_test;
mod hover;
mod pencil;
mod select;
mod selection;

pub(crate) use drag::*;
pub(crate) use hit_test::*;
pub(crate) use hover::*;
pub(crate) use selection::*;

use super::constants::ANCHOR_HIT_PX;
use super::types::AutomationEditCtx;
use super::value::panel_max_val;
use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;
use eframe::egui;
use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};
use yinhe_wgpu::AutomationGhost;

pub(crate) enum ToolResult {
    Continue,
    Break(Option<AutomationGhost>),
}

pub(crate) struct InteractionCtx<'a> {
    pub grid_area: egui::Rect,
    pub panel_rect: egui::Rect,
    pub panel: &'a AutomationPanelView,
    pub target: AutomationTarget,
    pub lane: Option<&'a AutomationLane>,
    pub lane_idx: Option<usize>,
    pub track_idx: u16,
    pub active_tool: Tool,
    pub track_color: [f32; 3],
    pub max_val: f32,
    pub value_cap: f32,
    pub ppu: f32,
    pub scroll_x: f32,
    pub drag_id: egui::Id,
    pub move_offset_id: egui::Id,
    pub drag_state: Option<AutoDrag>,
    pub pointer_pressed: bool,
    pub pointer_released: bool,
    pub pointer_clicked: bool,
    pub pointer_double_clicked: bool,
    pub pointer_secondary_clicked: bool,
    pub mouse_info: Option<(egui::Pos2, u32, f32)>,
    pub pos: Option<egui::Pos2>,
    pub in_grid: bool,
    pub hit_anchor: Option<(usize, u32)>,
    pub hit_ctrl: Option<ControlPointHit>,
    pub suppress_blank_marquee: bool,
    pub cmd: bool,
    pub alt: bool,
    pub shift: bool,
    pub egui_ctx: egui::Context,
    pub marquee_rect: Option<egui::Rect>,
    pub sel_op: Option<SelOp>,
    pub in_sel_rect: bool,
    pub hover_anchor_id: egui::Id,
    pub hover_ctrl_id: egui::Id,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn handle_automation_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    automation_lanes: &[AutomationLane],
    tempo_lane: Option<&AutomationLane>,
    track_idx: u16,
    ctx: &AutomationEditCtx<'_>,
    id_base: egui::Id,
    track_colors: &[[f32; 4]],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
    suppress_blank_marquee: bool,
) -> (
    Vec<yinhe_types::AutomationEdit>,
    Option<AutomationGhost>,
    Option<HoverTooltip>,
    Option<HoverTooltip>,
    Option<egui::Rect>,
    Option<SelOp>,
) {
    let mut edits = Vec::new();
    let target = panel.selected_target.clone();
    if target == AutomationTarget::Tempo && tempo_lane.is_none() {
        return (edits, None, None, None, None, None);
    }
    let max_val = match tempo_lane {
        Some(tl) => panel_max_val(panel, tl),
        None => panel.selected_target.max_value(),
    };
    if max_val == 0.0 {
        return (edits, None, None, None, None, None);
    }
    let value_cap = if target == yinhe_types::AutomationTarget::Tempo {
        yinhe_types::AutomationTarget::Tempo.max_value()
    } else {
        max_val
    };

    let ppu = panel.base.pixels_per_tick;
    let scroll_x = panel.base.scroll_x;
    let drag_id = ui.id().with("auto_drag").with(id_base);
    let move_offset_id = ui.id().with("auto_move_offset").with(id_base);
    let track_color4 = track_colors
        .get(track_idx as usize)
        .copied()
        .unwrap_or([0.8, 0.8, 0.8, 1.0]);
    let track_color = [track_color4[0], track_color4[1], track_color4[2]];

    let drag_state = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));

    let pointer_hover_pos = ui.input(|i| i.pointer.hover_pos());
    let pointer_pressed = ui.input(|i| i.pointer.primary_pressed());
    let pointer_released = ui.input(|i| i.pointer.primary_released());
    let pointer_clicked = ui.input(|i| i.pointer.primary_clicked());
    let pointer_double_clicked = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });
    let pointer_secondary_clicked =
        ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));

    let pos = pointer_hover_pos;
    let mouse_info = pos.map(|p| {
        let x_in_grid = p.x - grid_area.min.x;
        let raw_tick = ((x_in_grid + scroll_x) / ppu).max(0.0);
        let snapped_tick = crate::view_interaction::snap_tick(
            raw_tick as f64,
            ctx.quantize,
            ctx.ppq,
            ctx.bar_line_data,
        )
        .max(0.0) as u32;
        let y_in_panel = p.y - panel_rect.min.y;
        let value = panel.y_to_value(y_in_panel, max_val).clamp(0.0, value_cap);
        (p, snapped_tick, value)
    });

    let in_grid = pos.is_some_and(|p| grid_area.contains(p));

    let (lane_idx, lane): (Option<usize>, Option<&AutomationLane>) =
        if target == yinhe_types::AutomationTarget::Tempo {
            (Some(0), tempo_lane)
        } else {
            let idx = automation_lanes.iter().position(|l| l.target == target);
            (idx, idx.and_then(|i| automation_lanes.get(i)))
        };

    let hit_anchor = lane.and_then(|l| {
        let (_, snapped_tick, _) = mouse_info?;
        l.events
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| (e.tick as i64 - snapped_tick as i64).unsigned_abs())
            .and_then(|(i, e)| {
                let (p, _, _) = mouse_info?;
                let ex = grid_area.min.x + (e.tick as f32) * ppu - scroll_x;
                let ey = panel_rect.min.y + panel.value_to_y(e.value, max_val);
                let dist = ((ex - p.x).powi(2) + (ey - p.y).powi(2)).sqrt();
                if dist <= ANCHOR_HIT_PX {
                    Some((i, e.tick))
                } else {
                    None
                }
            })
    });

    let hit_ctrl = if matches!(
        ctx.active_tool,
        Tool::Pencil | Tool::Select | Tool::SelectVertical
    ) && drag_state.is_none()
        && hit_anchor.is_none()
        && in_grid
    {
        lane.and_then(|l| {
            let (p, _, _) = mouse_info?;
            hit_control_point_on_lane(l, p, ppu, scroll_x, grid_area, panel_rect, panel, max_val)
        })
    } else {
        None
    };

    let egui_ctx = ui.ctx().clone();
    let (cmd, alt, shift) = egui_ctx.input(|i| {
        (
            i.modifiers.command || i.modifiers.ctrl,
            i.modifiers.alt,
            i.modifiers.shift,
        )
    });
    let in_sel_rect = panel.anchor_sel_rects.iter().any(|r| {
        let Some((_, tick, value)) = mouse_info else {
            return false;
        };
        r.contains(tick, value)
    });
    let hover_anchor_id = ui.id().with("auto_hover_anchor").with(id_base);
    let hover_ctrl_id = ui.id().with("auto_hover_ctrl").with(id_base);

    // 拖拽中/悬停光标
    if let Some(drag) = drag_state {
        match drag {
            AutoDrag::MoveAnchors { .. } => {
                egui_ctx.set_cursor_icon(egui::CursorIcon::Move);
            }
            AutoDrag::MarqueeSelect { .. } | AutoDrag::EraserMarquee { .. } => {}
            _ => {
                egui_ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
    } else if (hit_anchor.is_some() || hit_ctrl.is_some()) && in_grid {
        egui_ctx.set_cursor_icon(egui::CursorIcon::Grab);
    } else if matches!(ctx.active_tool, Tool::Select | Tool::SelectVertical)
        && hit_anchor.is_none()
        && in_grid
        && !panel.anchor_sel_rects.is_empty()
        && in_sel_rect
    {
        egui_ctx.set_cursor_icon(egui::CursorIcon::Move);
    }

    let mut ictx = InteractionCtx {
        grid_area,
        panel_rect,
        panel,
        target,
        lane,
        lane_idx,
        track_idx,
        active_tool: ctx.active_tool,
        track_color,
        max_val,
        value_cap,
        ppu,
        scroll_x,
        drag_id,
        move_offset_id,
        drag_state,
        pointer_pressed,
        pointer_released,
        pointer_clicked,
        pointer_double_clicked,
        pointer_secondary_clicked,
        mouse_info,
        pos,
        in_grid,
        hit_anchor,
        hit_ctrl,
        suppress_blank_marquee,
        cmd,
        alt,
        shift,
        egui_ctx: egui_ctx.clone(),
        marquee_rect: None,
        sel_op: None,
        in_sel_rect,
        hover_anchor_id,
        hover_ctrl_id,
    };

    match ictx.active_tool {
        Tool::Pencil => match pencil::handle_pencil(&mut ictx, &mut edits, info_content, right_tab)
        {
            ToolResult::Break(g) => {
                return (edits, g, None, None, ictx.marquee_rect, ictx.sel_op);
            }
            ToolResult::Continue => {}
        },
        Tool::Curve => match curve::handle_curve(&mut ictx, &mut edits) {
            ToolResult::Break(g) => {
                return (edits, g, None, None, ictx.marquee_rect, ictx.sel_op);
            }
            ToolResult::Continue => {}
        },
        Tool::Select | Tool::SelectVertical => match select::handle_select(&mut ictx, &mut edits) {
            ToolResult::Break(g) => {
                return (edits, g, None, None, ictx.marquee_rect, ictx.sel_op);
            }
            ToolResult::Continue => {}
        },
        Tool::Eraser => match eraser::handle_eraser(&mut ictx, &mut edits) {
            ToolResult::Break(g) => {
                return (edits, g, None, None, ictx.marquee_rect, ictx.sel_op);
            }
            ToolResult::Continue => {}
        },
        _ => {}
    }

    // 右键点击锚点 → 记录编辑信息
    let right_click_id = ui.id().with("auto_right_click").with(id_base);
    if ictx.pointer_secondary_clicked
        && ictx.in_grid
        && let Some((_, tick)) = ictx.hit_anchor
        && let Some(lidx) = ictx.lane_idx
        && let Some(l) = ictx.lane
        && l.events.iter().find(|e| e.tick == tick).is_some()
    {
        let edit_tick_id = ui.id().with("auto_right_tick").with(id_base);
        let edit_value_id = ui.id().with("auto_right_value").with(id_base);
        let was_open_id = ui.id().with("auto_right_was_open").with(id_base);
        egui_ctx.data_mut(|d| {
            d.remove::<f64>(edit_tick_id);
            d.remove::<f64>(edit_value_id);
            d.remove::<bool>(was_open_id);
            d.insert_temp(
                right_click_id,
                RightClickAnchor {
                    track_idx,
                    lane_idx: lidx,
                    old_tick: tick,
                    target: ictx.target.clone(),
                },
            );
        });
    }

    let ghost = ghost::compute_ghost(&mut ictx);
    let drag_info = ghost::compute_drag_info(&ictx, ghost.is_some());
    let hover_info = ghost::compute_hover_info(&ictx, drag_info.is_some());

    (
        edits,
        ghost,
        drag_info,
        hover_info,
        ictx.marquee_rect,
        ictx.sel_op,
    )
}

#[cfg(test)]
mod tests;
