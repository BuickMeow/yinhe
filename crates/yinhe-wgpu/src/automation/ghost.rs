use super::data_lines;
use super::prepare::AutomationGhost;
use yinhe_types::AutomationPanelView;
use yinhe_theme::GpuTheme;
use yinhe_types::{AutomationEvent, AutomationLane};
use crate::vertex::DrawInstance;

/// ghost 锚点半径（像素）。
const GHOST_RADIUS: f32 = 4.0;
/// ghost 不透明度。
const GHOST_ALPHA: f32 = 0.9;

/// 构造一条覆盖后的 lane：删除 `old_tick` 处的事件，在 `new_tick` 处插入新事件。
///
/// 用于拖拽 ghost：被拖事件从原位置移动到新位置，其余事件保持不变。
/// 如果 `new_tick` 处已有事件，先删除再插入（避免同一 tick 多个事件）。
pub fn build_lane_override(
    lane: &AutomationLane,
    old_tick: u32,
    new_tick: u32,
    new_value: u16,
) -> AutomationLane {
    let mut events = lane.events.clone();

    // 找到并删除 old_tick 对应的事件，同时保留其 shape
    let old_shape = events
        .iter()
        .position(|e| e.tick == old_tick)
        .map(|idx| events.remove(idx).shape)
        .unwrap_or_else(|| lane.target.default_shape());

    // 删除 new_tick 处可能存在的旧事件（避免重复 tick）
    if let Some(idx) = events.iter().position(|e| e.tick == new_tick) {
        events.remove(idx);
    }

    // 二分查找插入位置，保持有序
    let insert_idx = events.partition_point(|e| e.tick < new_tick);
    events.insert(
        insert_idx,
        AutomationEvent {
            tick: new_tick,
            value: new_value,
            shape: old_shape,
        },
    );

    AutomationLane {
        target: lane.target.clone(),
        track: lane.track,
        events,
    }
}

/// Build ghost preview instances (layer 3, rebuilt every frame).
///
/// `AutomationGhost` 坐标为 panel 局部像素坐标。
pub fn build_ghost(
    out: &mut Vec<DrawInstance>,
    ghost: AutomationGhost,
    w: f32,
    view: &AutomationPanelView,
    show_anchors: bool,
    _theme: &GpuTheme,
) {
    let push_anchor = |out: &mut Vec<DrawInstance>, x: f32, y: f32, color: [f32; 3]| {
        out.push(DrawInstance::with_props(
            x - GHOST_RADIUS, y - GHOST_RADIUS,
            2.0 * GHOST_RADIUS, 2.0 * GHOST_RADIUS,
            [color[0], color[1], color[2], GHOST_ALPHA],
            GHOST_RADIUS, 0.0,
        ));
    };

    match ghost {
        AutomationGhost::Move { lane, color } => {
            // 整条 lane 作为 ghost 重新绘制
            data_lines::build_lane_instances(out, w, view, &lane, color, show_anchors);
        }
        AutomationGhost::Curve { start_x, start_y, cur_x, cur_y, color } => {
            push_anchor(out, start_x, start_y, color);
            data_lines::push_polyline(out, start_x, start_y, cur_x, cur_y, |t| t, color);
            push_anchor(out, cur_x, cur_y, color);
        }
    }
}
