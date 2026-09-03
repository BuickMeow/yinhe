use crate::piano_view::automation_panel::constants::MARQUEE_THRESHOLD;

use super::drag::{AutoDrag, commit_anchor_or_ctrl_release};
use super::hit_test::hit_line_on_lane;
use super::hover::CtrlEnd;
use super::selection::{SelOp, SelRectOp, union_anchor_sel_rect};
use super::{InteractionCtx, ToolResult};
use crate::widgets::tools_panel::Tool;
use eframe::egui;
use yinhe_types::{AnchorSelRect, SegmentShape};

pub(crate) fn handle_select(
    ctx: &mut InteractionCtx<'_>,
    edits: &mut Vec<yinhe_types::AutomationEdit>,
) -> ToolResult {
    let vertical = matches!(ctx.active_tool, Tool::SelectVertical);

    if ctx.pointer_double_clicked && ctx.in_grid {
        if let Some((_, tick)) = ctx.hit_anchor {
            if let Some(lidx) = ctx.lane_idx {
                edits.push(yinhe_types::AutomationEdit::Delete {
                    track_idx: ctx.track_idx,
                    lane_idx: lidx,
                    target: ctx.target.clone(),
                    tick,
                });
            }
            ctx.egui_ctx.data_mut(|d| d.remove::<AutoDrag>(ctx.drag_id));
        } else if ctx.hit_ctrl.is_none()
            && let Some((_, tick, value)) = ctx.mouse_info
        {
            edits.push(yinhe_types::AutomationEdit::Add {
                track_idx: ctx.track_idx,
                target: ctx.target.clone(),
                tick,
                value,
                shape: SegmentShape::Step,
            });
        }
        return ToolResult::Break(None);
    }

    if ctx.pointer_pressed && ctx.in_grid {
        if let Some((event_idx, tick)) = ctx.hit_anchor {
            let anchor_value = ctx
                .lane
                .and_then(|l| l.events.get(event_idx))
                .map(|e| e.value)
                .unwrap_or(0.0);
            let anchor_in_sel = ctx
                .panel
                .anchor_sel_rects
                .iter()
                .any(|r| r.contains(tick, anchor_value));
            if ctx.cmd || ctx.shift {
                let point_rect = AnchorSelRect {
                    tick_start: tick as f64,
                    tick_end: tick as f64,
                    value_range: if vertical {
                        None
                    } else {
                        Some((anchor_value, anchor_value))
                    },
                };
                let final_rect = match ctx.panel.anchor_sel_rects.last().copied() {
                    Some(existing) => union_anchor_sel_rect(existing, point_rect),
                    None => point_rect,
                };
                ctx.sel_op = Some(SelOp::Set(SelRectOp::Set(final_rect)));
            } else {
                if !anchor_in_sel {
                    let point_rect = AnchorSelRect {
                        tick_start: tick as f64,
                        tick_end: tick as f64,
                        value_range: if vertical {
                            None
                        } else {
                            Some((anchor_value, anchor_value))
                        },
                    };
                    ctx.sel_op = Some(SelOp::Set(SelRectOp::Set(point_rect)));
                }
                if let Some((_, _, value)) = ctx.mouse_info {
                    ctx.egui_ctx.data_mut(|d| {
                        d.insert_temp(
                            ctx.drag_id,
                            AutoDrag::MoveAnchors {
                                start_tick: tick,
                                start_value: value,
                                alt: ctx.alt,
                            },
                        );
                    });
                }
            }
        } else if ctx.in_sel_rect && !ctx.panel.anchor_sel_rects.is_empty() {
            if let Some((_, tick, value)) = ctx.mouse_info {
                ctx.egui_ctx.data_mut(|d| {
                    d.insert_temp(
                        ctx.drag_id,
                        AutoDrag::MoveAnchors {
                            start_tick: tick,
                            start_value: value,
                            alt: ctx.alt,
                        },
                    );
                });
            }
        } else if let Some(hit) = ctx.hit_ctrl {
            let (start_x, start_y) = match hit.which {
                CtrlEnd::Out => (hit.x1, hit.y1),
                CtrlEnd::In => (hit.x2, hit.y2),
            };
            ctx.egui_ctx.data_mut(|d| {
                d.insert_temp(
                    ctx.drag_id,
                    AutoDrag::DragControlPoint {
                        prev_tick: hit.prev_tick,
                        which: hit.which,
                        start_x,
                        start_y,
                    },
                );
            });
        } else if !(ctx.cmd || ctx.shift)
            && let Some(l) = ctx.lane
            && let Some((_, tick, value)) = ctx.mouse_info
            && hit_line_on_lane(
                l,
                tick,
                value,
                ctx.ppu,
                ctx.scroll_x,
                ctx.grid_area.min.x,
                ctx.panel_rect.min.y,
                ctx.panel,
                ctx.max_val,
            )
        {
            edits.push(yinhe_types::AutomationEdit::Add {
                track_idx: ctx.track_idx,
                target: ctx.target.clone(),
                tick,
                value,
                shape: SegmentShape::Step,
            });
            ctx.egui_ctx.data_mut(|d| {
                d.insert_temp(
                    ctx.drag_id,
                    AutoDrag::MoveAnchor {
                        old_tick: tick,
                        start_tick: tick,
                        start_value: value,
                    },
                );
            });
        } else if let Some((p, _, _)) = ctx.mouse_info {
            if ctx.suppress_blank_marquee {
            } else {
                ctx.egui_ctx.data_mut(|d| {
                    d.insert_temp(ctx.drag_id, AutoDrag::MarqueeSelect { start_pos: p })
                });
                if !ctx.cmd && !ctx.shift {
                    ctx.sel_op = Some(SelOp::ClearNoteSelection);
                }
            }
        }
    }

    if let Some(AutoDrag::MarqueeSelect { start_pos, .. }) = ctx.drag_state
        && let Some(p) = ctx.pos
    {
        let dist = (p - start_pos).length();
        if dist >= MARQUEE_THRESHOLD {
            let rect = egui::Rect::from_min_max(
                egui::pos2(start_pos.x.min(p.x), ctx.grid_area.min.y),
                egui::pos2(start_pos.x.max(p.x), ctx.grid_area.max.y),
            )
            .intersect(ctx.grid_area);
            ctx.marquee_rect = Some(rect);
            if !ctx.cmd && !ctx.shift {
                ctx.sel_op = Some(SelOp::Set(SelRectOp::Keep));
            }
        }
    }

    if ctx.pointer_released {
        let drag = ctx.egui_ctx.data(|d| d.get_temp::<AutoDrag>(ctx.drag_id));
        ctx.egui_ctx.data_mut(|d| d.remove::<AutoDrag>(ctx.drag_id));
        ctx.egui_ctx
            .data_mut(|d| d.remove::<(i64, f32)>(ctx.move_offset_id));
        let mut release_ghost: Option<yinhe_wgpu::AutomationGhost> = None;
        match drag {
            Some(AutoDrag::MarqueeSelect { start_pos, .. }) => {
                let dist = ctx.pos.map(|p| (p - start_pos).length()).unwrap_or(0.0);
                if dist >= MARQUEE_THRESHOLD {
                    if let Some((p, _, _)) = ctx.mouse_info {
                        let min_x = start_pos.x.min(p.x);
                        let max_x = start_pos.x.max(p.x);
                        let tick_from_x = |x: f32| -> f64 {
                            ((x - ctx.grid_area.min.x + ctx.scroll_x) / ctx.ppu).max(0.0) as f64
                        };
                        let new_rect = AnchorSelRect {
                            tick_start: tick_from_x(min_x),
                            tick_end: tick_from_x(max_x),
                            value_range: None,
                        };
                        if ctx.cmd || ctx.shift {
                            ctx.sel_op = Some(SelOp::Set(SelRectOp::Append(new_rect)));
                        } else {
                            ctx.sel_op = Some(SelOp::Set(SelRectOp::Set(new_rect)));
                        }
                    }
                } else if !ctx.cmd && !ctx.shift {
                    ctx.sel_op = Some(SelOp::Clear);
                }
            }
            Some(AutoDrag::MoveAnchors {
                start_tick,
                start_value,
                alt,
                ..
            }) => {
                if let Some((_, cur_tick, cur_value)) = ctx.mouse_info
                    && let Some(l) = ctx.lane
                    && let Some(lidx) = ctx.lane_idx
                    && !ctx.panel.anchor_sel_rects.is_empty()
                {
                    let d_tick = cur_tick as i64 - start_tick as i64;
                    let d_value = if vertical
                        || ctx
                            .panel
                            .anchor_sel_rects
                            .iter()
                            .any(|r| r.value_range.is_none())
                    {
                        0.0
                    } else {
                        cur_value - start_value
                    };
                    if d_tick != 0 || d_value.abs() > 1e-6 {
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
                        if !moves.is_empty() {
                            let ghost_lane = if alt {
                                yinhe_wgpu::build_lane_multi_copy(l, &moves)
                            } else {
                                yinhe_wgpu::build_lane_multi_move(l, &moves)
                            };
                            release_ghost = Some(yinhe_wgpu::AutomationGhost::Move {
                                lane: ghost_lane,
                                color: ctx.track_color,
                            });
                            if alt {
                                for &(tick, new_tick, new_value) in &moves {
                                    let shape = l
                                        .events
                                        .iter()
                                        .find(|e| e.tick == tick)
                                        .map(|e| e.shape)
                                        .unwrap_or(SegmentShape::Step);
                                    edits.push(yinhe_types::AutomationEdit::Add {
                                        track_idx: ctx.track_idx,
                                        target: ctx.target.clone(),
                                        tick: new_tick,
                                        value: new_value,
                                        shape,
                                    });
                                }
                            } else {
                                edits.push(yinhe_types::AutomationEdit::MoveBatch {
                                    track_idx: ctx.track_idx,
                                    lane_idx: lidx,
                                    target: ctx.target.clone(),
                                    moves,
                                });
                            }
                            let new_rects: Vec<AnchorSelRect> = ctx
                                .panel
                                .anchor_sel_rects
                                .iter()
                                .map(|sel_rect| AnchorSelRect {
                                    tick_start: (sel_rect.tick_start + d_tick as f64).max(0.0),
                                    tick_end: (sel_rect.tick_end + d_tick as f64).max(0.0),
                                    value_range: sel_rect.value_range.map(|(vmin, vmax)| {
                                        (
                                            (vmin + d_value).clamp(0.0, ctx.value_cap),
                                            (vmax + d_value).clamp(0.0, ctx.value_cap),
                                        )
                                    }),
                                })
                                .collect();
                            ctx.sel_op = Some(SelOp::Set(SelRectOp::ReplaceAll(new_rects)));
                        }
                    }
                }
            }
            drag
            @ (Some(AutoDrag::MoveAnchor { .. }) | Some(AutoDrag::DragControlPoint { .. })) => {
                release_ghost = commit_anchor_or_ctrl_release(
                    drag,
                    ctx.lane,
                    ctx.lane_idx,
                    ctx.track_idx,
                    &ctx.target,
                    ctx.mouse_info,
                    ctx.ppu,
                    ctx.scroll_x,
                    ctx.grid_area,
                    ctx.panel_rect,
                    ctx.panel,
                    ctx.max_val,
                    edits,
                    ctx.track_color,
                );
            }
            _ => {}
        }
        return ToolResult::Break(release_ghost);
    }

    ToolResult::Continue
}
