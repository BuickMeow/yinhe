use crate::piano_view::automation_panel::constants::MARQUEE_THRESHOLD;

use super::drag::AutoDrag;
use super::{InteractionCtx, ToolResult};
use eframe::egui;

pub(crate) fn handle_eraser(
    ctx: &mut InteractionCtx<'_>,
    edits: &mut Vec<yinhe_types::AutomationEdit>,
) -> ToolResult {
    if ctx.pointer_pressed && ctx.in_grid {
        if let Some((_, tick)) = ctx.hit_anchor {
            if let Some(lidx) = ctx.lane_idx {
                edits.push(yinhe_types::AutomationEdit::Delete {
                    track_idx: ctx.track_idx,
                    lane_idx: lidx,
                    target: ctx.target.clone(),
                    tick,
                });
            }
        } else if let Some((p, _, _)) = ctx.mouse_info {
            ctx.egui_ctx
                .data_mut(|d| d.insert_temp(ctx.drag_id, AutoDrag::EraserMarquee { start_pos: p }));
        }
    }

    if let Some(AutoDrag::EraserMarquee { start_pos, .. }) = ctx.drag_state
        && let Some(p) = ctx.pos
        && (p - start_pos).length() >= MARQUEE_THRESHOLD
    {
        ctx.marquee_rect = Some(egui::Rect::from_two_pos(start_pos, p).intersect(ctx.grid_area));
    }

    if ctx.pointer_released {
        let drag = ctx.egui_ctx.data(|d| d.get_temp::<AutoDrag>(ctx.drag_id));
        ctx.egui_ctx.data_mut(|d| d.remove::<AutoDrag>(ctx.drag_id));
        if let Some(AutoDrag::EraserMarquee { start_pos }) = drag
            && let Some(l) = ctx.lane
            && let Some(lidx) = ctx.lane_idx
            && let Some(p) = ctx.pos
            && (p - start_pos).length() >= MARQUEE_THRESHOLD
        {
            let rect = egui::Rect::from_two_pos(start_pos, p);
            let tick_from_x = |x: f32| -> f64 {
                ((x - ctx.grid_area.min.x + ctx.scroll_x) / ctx.ppu).max(0.0) as f64
            };
            let t0 = tick_from_x(rect.min.x);
            let t1 = tick_from_x(rect.max.x);
            let v_hi = ctx
                .panel
                .y_to_value(rect.min.y - ctx.panel_rect.min.y, ctx.max_val);
            let v_lo = ctx
                .panel
                .y_to_value(rect.max.y - ctx.panel_rect.min.y, ctx.max_val);
            for e in &l.events {
                if (e.tick as f64) >= t0
                    && (e.tick as f64) <= t1
                    && e.value >= v_lo
                    && e.value <= v_hi
                {
                    edits.push(yinhe_types::AutomationEdit::Delete {
                        track_idx: ctx.track_idx,
                        lane_idx: lidx,
                        target: ctx.target.clone(),
                        tick: e.tick,
                    });
                }
            }
        }
        return ToolResult::Break(None);
    }

    ToolResult::Continue
}
