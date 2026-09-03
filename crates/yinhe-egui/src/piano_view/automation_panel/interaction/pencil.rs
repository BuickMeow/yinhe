use super::drag::{AutoDrag, commit_anchor_or_ctrl_release};
use super::hit_test::hit_line_on_lane;
use super::hover::CtrlEnd;
use super::{InteractionCtx, ToolResult};
use crate::right_panel::{InfoContent, RightTab};
use yinhe_types::SegmentShape;

pub(crate) fn handle_pencil(
    ctx: &mut InteractionCtx<'_>,
    edits: &mut Vec<yinhe_types::AutomationEdit>,
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
) -> ToolResult {
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
            if let Some(lidx) = ctx.lane_idx {
                *info_content = Some(InfoContent::Anchor {
                    track_idx: ctx.track_idx,
                    lane_idx: lidx,
                    event_idx,
                    target: ctx.target.clone(),
                });
                *right_tab = Some(RightTab::Info);
            }
            let anchor_value = ctx
                .lane
                .and_then(|l| l.events.iter().find(|e| e.tick == tick))
                .map(|e| e.value)
                .unwrap_or(0.0);
            ctx.egui_ctx.data_mut(|d| {
                d.insert_temp(
                    ctx.drag_id,
                    AutoDrag::MoveAnchor {
                        old_tick: tick,
                        start_tick: tick,
                        start_value: anchor_value,
                    },
                );
            });
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
        } else if ctx.drag_state.is_none()
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
        }
    }

    if ctx.pointer_released {
        let drag = ctx.egui_ctx.data(|d| d.get_temp::<AutoDrag>(ctx.drag_id));
        ctx.egui_ctx.data_mut(|d| d.remove::<AutoDrag>(ctx.drag_id));
        let ghost = commit_anchor_or_ctrl_release(
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
        return ToolResult::Break(ghost);
    }

    if ctx.pointer_clicked
        && ctx.in_grid
        && ctx.hit_anchor.is_none()
        && ctx.hit_ctrl.is_none()
        && ctx.drag_state.is_none()
    {
        if let Some((_, tick, value)) = ctx.mouse_info {
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

    ToolResult::Continue
}
