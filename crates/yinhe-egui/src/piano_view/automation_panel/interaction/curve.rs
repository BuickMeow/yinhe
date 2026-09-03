use super::drag::AutoDrag;
use super::{InteractionCtx, ToolResult};
use yinhe_types::SegmentShape;

pub(crate) fn handle_curve(
    ctx: &mut InteractionCtx<'_>,
    edits: &mut Vec<yinhe_types::AutomationEdit>,
) -> ToolResult {
    if ctx.pointer_pressed
        && ctx.in_grid
        && let Some((_, tick, value)) = ctx.mouse_info
    {
        ctx.egui_ctx.data_mut(|d| {
            d.insert_temp(
                ctx.drag_id,
                AutoDrag::CurveDraw {
                    start_tick: tick,
                    start_value: value,
                },
            );
        });
    }
    if ctx.pointer_released {
        let drag = ctx.egui_ctx.data(|d| d.get_temp::<AutoDrag>(ctx.drag_id));
        ctx.egui_ctx.data_mut(|d| d.remove::<AutoDrag>(ctx.drag_id));
        if let Some(AutoDrag::CurveDraw {
            start_tick: t1,
            start_value: v1,
        }) = drag
            && let Some((_, t2, v2)) = ctx.mouse_info
        {
            if t1 != t2 {
                edits.push(yinhe_types::AutomationEdit::Add {
                    track_idx: ctx.track_idx,
                    target: ctx.target.clone(),
                    tick: t1.min(t2),
                    value: v1,
                    shape: SegmentShape::linear_curve(),
                });
                edits.push(yinhe_types::AutomationEdit::Add {
                    track_idx: ctx.track_idx,
                    target: ctx.target.clone(),
                    tick: t1.max(t2),
                    value: v2,
                    shape: SegmentShape::Step,
                });
            } else {
                edits.push(yinhe_types::AutomationEdit::Add {
                    track_idx: ctx.track_idx,
                    target: ctx.target.clone(),
                    tick: t2,
                    value: v2,
                    shape: SegmentShape::linear_curve(),
                });
            }
        }
        return ToolResult::Break(None);
    }
    ToolResult::Continue
}
