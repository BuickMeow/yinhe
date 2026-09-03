use crate::piano_view::automation_panel::constants::HOVER_DELAY;

use super::InteractionCtx;
use super::drag::AutoDrag;
use super::hit_test::{compute_ctrl_from_mouse, merge_ctrl_shape};
use super::hover::{CtrlEnd, HoverTooltip};
use yinhe_types::SegmentShape;
use yinhe_wgpu::{AutomationGhost, build_lane_multi_copy, build_lane_multi_move};

pub(crate) fn compute_ghost(ctx: &mut InteractionCtx<'_>) -> Option<AutomationGhost> {
    let drag_now = ctx.egui_ctx.data(|d| d.get_temp::<AutoDrag>(ctx.drag_id));
    let (p, cur_tick, cur_value) = ctx.mouse_info?;
    let drag = drag_now?;
    let x_offset = ctx.panel.base.left_panel_width - ctx.scroll_x;
    let cur_x = x_offset + cur_tick as f32 * ctx.ppu;
    let cur_y = ctx.panel.value_to_y(cur_value, ctx.max_val);
    match drag {
        AutoDrag::MoveAnchor {
            old_tick,
            start_tick: _,
            start_value: _,
        } => ctx.lane.map(|l| {
            let override_lane = yinhe_wgpu::build_lane_override(l, old_tick, cur_tick, cur_value);
            AutomationGhost::Move {
                lane: override_lane,
                color: ctx.track_color,
            }
        }),
        AutoDrag::CurveDraw {
            start_tick,
            start_value,
        } => {
            let start_x = x_offset + start_tick as f32 * ctx.ppu;
            let start_y = ctx.panel.value_to_y(start_value, ctx.max_val);
            Some(AutomationGhost::Curve {
                start_x,
                start_y,
                cur_x,
                cur_y,
                color: ctx.track_color,
            })
        }
        AutoDrag::DragControlPoint {
            prev_tick, which, ..
        } => ctx.lane.and_then(|l| {
            let new_ctrl = compute_ctrl_from_mouse(
                l,
                prev_tick,
                which,
                p,
                ctx.ppu,
                ctx.scroll_x,
                ctx.grid_area,
                ctx.panel_rect,
                ctx.panel,
                ctx.max_val,
            )?;
            let new_shape = merge_ctrl_shape(l, prev_tick, which, new_ctrl);
            let override_lane = yinhe_wgpu::build_lane_shape_override(l, prev_tick, new_shape);
            Some(AutomationGhost::Move {
                lane: override_lane,
                color: ctx.track_color,
            })
        }),
        AutoDrag::MoveAnchors {
            start_tick,
            start_value,
            alt,
            ..
        } => {
            let d_tick = cur_tick as i64 - start_tick as i64;
            let vertical_now = matches!(
                ctx.active_tool,
                crate::widgets::tools_panel::Tool::SelectVertical
            ) || ctx
                .panel
                .anchor_sel_rects
                .iter()
                .any(|r| r.value_range.is_none());
            let d_value = if vertical_now {
                0.0
            } else {
                cur_value - start_value
            };
            ctx.egui_ctx
                .data_mut(|d| d.insert_temp(ctx.move_offset_id, (d_tick, d_value)));
            if d_tick == 0 && d_value.abs() <= 1e-6 {
                return None;
            }
            ctx.lane.and_then(|l| {
                if ctx.panel.anchor_sel_rects.is_empty() {
                    return None;
                }
                let moves: Vec<(u32, u32, f32)> = l
                    .events
                    .iter()
                    .filter_map(|e| {
                        if !ctx
                            .panel
                            .anchor_sel_rects
                            .iter()
                            .any(|r| r.contains(e.tick, e.value))
                        {
                            return None;
                        }
                        let new_tick = (e.tick as i64 + d_tick).max(0) as u32;
                        let new_value = (e.value + d_value).clamp(0.0, ctx.value_cap);
                        Some((e.tick, new_tick, new_value))
                    })
                    .collect();
                if moves.is_empty() {
                    return None;
                }
                let override_lane = if alt {
                    build_lane_multi_copy(l, &moves)
                } else {
                    build_lane_multi_move(l, &moves)
                };
                Some(AutomationGhost::Move {
                    lane: override_lane,
                    color: ctx.track_color,
                })
            })
        }
        AutoDrag::MarqueeSelect { .. } | AutoDrag::EraserMarquee { .. } => None,
    }
}

pub(crate) fn compute_drag_info(ctx: &InteractionCtx<'_>, has_ghost: bool) -> Option<HoverTooltip> {
    if !has_ghost {
        return None;
    }
    let drag_now = ctx.egui_ctx.data(|d| d.get_temp::<AutoDrag>(ctx.drag_id));
    match drag_now {
        Some(AutoDrag::DragControlPoint {
            prev_tick, which, ..
        }) => ctx.lane.and_then(|l| {
            let (p, _, _) = ctx.mouse_info?;
            let new_ctrl = compute_ctrl_from_mouse(
                l,
                prev_tick,
                which,
                p,
                ctx.ppu,
                ctx.scroll_x,
                ctx.grid_area,
                ctx.panel_rect,
                ctx.panel,
                ctx.max_val,
            )?;
            let (x1, y1, x2, y2) =
                l.events
                    .iter()
                    .find(|e| e.tick == prev_tick)
                    .map(|e| match e.shape {
                        SegmentShape::Curve { x1, y1, x2, y2 } => (x1, y1, x2, y2),
                        SegmentShape::Step => (0.0, 0.0, 0.0, 0.0),
                    })?;
            let (x1, y1, x2, y2) = match which {
                CtrlEnd::Out => (new_ctrl.0, new_ctrl.1, x2, y2),
                CtrlEnd::In => (x1, y1, new_ctrl.0, new_ctrl.1),
            };
            Some(HoverTooltip::ControlPoint {
                x1,
                y1,
                x2,
                y2,
                pos: p,
            })
        }),
        _ => ctx.mouse_info.map(|(p, tick, value)| HoverTooltip::Anchor {
            tick,
            value,
            pos: p,
        }),
    }
}

pub(crate) fn compute_hover_info(
    ctx: &InteractionCtx<'_>,
    has_drag_info: bool,
) -> Option<HoverTooltip> {
    if has_drag_info || !ctx.in_grid {
        return None;
    }
    let hover_anchor_id = ctx.hover_anchor_id;
    let hover_ctrl_id = ctx.hover_ctrl_id;
    let now = ctx.egui_ctx.input(|i| i.time);
    if let Some((_, anchor_tick)) = ctx.hit_anchor {
        ctx.egui_ctx
            .data_mut(|d| d.remove::<(u32, f64)>(hover_ctrl_id));
        let prev: Option<(u32, f64)> = ctx
            .egui_ctx
            .data(|d| d.get_temp::<(u32, f64)>(hover_anchor_id));
        let entry = match prev {
            Some(e) if e.0 == anchor_tick => e,
            _ => {
                let new_entry = (anchor_tick, now);
                ctx.egui_ctx
                    .data_mut(|d| d.insert_temp(hover_anchor_id, new_entry));
                new_entry
            }
        };
        if now - entry.1 >= HOVER_DELAY {
            let anchor_value = ctx
                .lane
                .and_then(|l| l.events.iter().find(|e| e.tick == anchor_tick))
                .map(|e| e.value);
            if let Some(v) = anchor_value {
                let ax = ctx.grid_area.min.x + anchor_tick as f32 * ctx.ppu - ctx.scroll_x;
                let ay = ctx.panel_rect.min.y + ctx.panel.value_to_y(v, ctx.max_val);
                Some(HoverTooltip::Anchor {
                    tick: anchor_tick,
                    value: v,
                    pos: eframe::egui::pos2(ax, ay),
                })
            } else {
                None
            }
        } else {
            ctx.egui_ctx.request_repaint();
            None
        }
    } else if let Some(hit) = ctx.hit_ctrl {
        ctx.egui_ctx
            .data_mut(|d| d.remove::<(u32, f64)>(hover_anchor_id));
        let prev: Option<(u32, f64)> = ctx
            .egui_ctx
            .data(|d| d.get_temp::<(u32, f64)>(hover_ctrl_id));
        let entry = match prev {
            Some(e) if e.0 == hit.prev_tick => e,
            _ => {
                let new_entry = (hit.prev_tick, now);
                ctx.egui_ctx
                    .data_mut(|d| d.insert_temp(hover_ctrl_id, new_entry));
                new_entry
            }
        };
        if now - entry.1 >= HOVER_DELAY {
            Some(HoverTooltip::ControlPoint {
                x1: hit.x1,
                y1: hit.y1,
                x2: hit.x2,
                y2: hit.y2,
                pos: hit.pos,
            })
        } else {
            ctx.egui_ctx.request_repaint();
            None
        }
    } else {
        ctx.egui_ctx.data_mut(|d| {
            d.remove::<(u32, f64)>(hover_anchor_id);
            d.remove::<(u32, f64)>(hover_ctrl_id);
        });
        None
    }
}
