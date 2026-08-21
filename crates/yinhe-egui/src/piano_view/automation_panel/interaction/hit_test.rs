use super::super::constants::ANCHOR_HIT_PX;
use super::hover::{ControlPointHit, CtrlEnd};
use eframe::egui;
use yinhe_types::{AutomationLane, AutomationPanelView, SegmentShape};

/// 检测鼠标是否悬停在两个锚点之间的线段上。
///
/// 如果鼠标位置在插值线附近（阈值 8 像素），返回 `true`。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn hit_line_on_lane(
    lane: &AutomationLane,
    tick: u32,
    value: f32,
    _ppu: f32,
    _scroll_x: f32,
    _grid_min_x: f32,
    _panel_min_y: f32,
    panel: &AutomationPanelView,
    max_val: f32,
) -> bool {
    // 找 bracket tick 的两个事件
    let idx = lane.events.partition_point(|e| e.tick <= tick);
    if idx == 0 || idx >= lane.events.len() {
        return false; // 左侧无事件或右侧无事件
    }
    let left = &lane.events[idx - 1];
    let right = &lane.events[idx];

    // 计算插值值
    let t = if right.tick == left.tick {
        0.0
    } else {
        (tick - left.tick) as f32 / (right.tick - left.tick) as f32
    };
    let interp = left.shape.interpolate(t);
    let interp_value = left.value + interp * (right.value - left.value);

    // 转换为像素坐标并检查距离
    let interp_y = panel.value_to_y(interp_value, max_val);
    let mouse_y = panel.value_to_y(value, max_val);
    (interp_y - mouse_y).abs() <= 8.0
}
/// 检测鼠标是否悬停在 Curve 段的两个空心圆控制点之一上。
///
/// 遍历所有 Curve 段（非直线），计算两个控制点屏幕位置（偏移量 *4 放大，
/// P1 相对 P0、P2 相对 P3），返回最近控制点所属段的前驱事件 tick + 端别 +
/// 该段的 4 个 ctrl 值 + 控制点像素位置。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn hit_control_point_on_lane(
    lane: &AutomationLane,
    mouse: egui::Pos2,
    ppu: f32,
    scroll_x: f32,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    max_val: f32,
) -> Option<ControlPointHit> {
    let x_offset = grid_area.min.x - scroll_x;
    let hit_sq = ANCHOR_HIT_PX * ANCHOR_HIT_PX;
    let mut best: Option<ControlPointHit> = None;
    for i in 1..lane.events.len() {
        let prev = &lane.events[i - 1];
        let cur = &lane.events[i];
        let SegmentShape::Curve { x1, y1, x2, y2 } = prev.shape else {
            continue;
        };
        if prev.shape.is_linear() {
            continue;
        }

        let px0 = x_offset + prev.tick as f32 * ppu;
        let py0 = panel_rect.min.y + panel.value_to_y(prev.value, max_val);
        let px3 = x_offset + cur.tick as f32 * ppu;
        let py3 = panel_rect.min.y + panel.value_to_y(cur.value, max_val);

        // 两个控制点屏幕坐标（偏移量 *4：P1 相对 P0，P2 相对 P3）
        let c1x = px0 + (px3 - px0) * x1 * 4.0;
        let c1y = py0 + (py3 - py0) * y1 * 4.0;
        let c2x = px3 + (px3 - px0) * x2 * 4.0;
        let c2y = py3 + (py3 - py0) * y2 * 4.0;

        // 分别检测两个控制点
        let d1 = (c1x - mouse.x).powi(2) + (c1y - mouse.y).powi(2);
        let d2 = (c2x - mouse.x).powi(2) + (c2y - mouse.y).powi(2);
        if d1 <= hit_sq && best.as_ref().map(|b| d1 < b.dist_sq).unwrap_or(true) {
            best = Some(ControlPointHit {
                prev_tick: prev.tick,
                which: CtrlEnd::Out,
                x1,
                y1,
                x2,
                y2,
                pos: egui::pos2(c1x, c1y),
                dist_sq: d1,
            });
        }
        if d2 <= hit_sq && best.as_ref().map(|b| d2 < b.dist_sq).unwrap_or(true) {
            best = Some(ControlPointHit {
                prev_tick: prev.tick,
                which: CtrlEnd::In,
                x1,
                y1,
                x2,
                y2,
                pos: egui::pos2(c2x, c2y),
                dist_sq: d2,
            });
        }
    }
    best
}
/// 从鼠标屏幕位置反推 Curve 段某一端控制点的偏移量 `(x, y) ∈ [-0.5, 0.5]`。
///
/// 偏移量参数化（内部 *4 放大）：
/// - `Out`（P1）：相对 P0，`offset = (mouse - P0) / (P3 - P0) / 4`
/// - `In`（P2）：相对 P3，`offset = (mouse - P3) / (P3 - P0) / 4`
///
/// 段水平或竖直时（dx/dy ≈ 0），对应分量为 0（与直线控制点对齐）。
/// 返回 `None` 表示 `prev_tick` 不存在或没有下一个事件。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn compute_ctrl_from_mouse(
    lane: &AutomationLane,
    prev_tick: u32,
    which: CtrlEnd,
    mouse: egui::Pos2,
    ppu: f32,
    scroll_x: f32,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    max_val: f32,
) -> Option<(f32, f32)> {
    let prev_idx = lane.events.iter().position(|e| e.tick == prev_tick)?;
    let prev = &lane.events[prev_idx];
    let next = lane.events.get(prev_idx + 1)?;
    let x_offset = grid_area.min.x - scroll_x;
    let px0 = x_offset + prev.tick as f32 * ppu;
    let py0 = panel_rect.min.y + panel.value_to_y(prev.value, max_val);
    let px3 = x_offset + next.tick as f32 * ppu;
    let py3 = panel_rect.min.y + panel.value_to_y(next.value, max_val);
    let dx = px3 - px0;
    let dy = py3 - py0;
    // 参考点：Out 用 P0，In 用 P3
    let (rx, ry) = match which {
        CtrlEnd::Out => (px0, py0),
        CtrlEnd::In => (px3, py3),
    };
    // offset = (mouse - ref) / seg / 4。
    // x 方向 clamp 到 CSS 单调区间：P1.x ∈ [0,1] → x1 ∈ [0, 0.25]；
    // P2.x ∈ [0,1] → x2 ∈ [-0.25, 0]。保证 x(u) 单调，渲染=音频=命中三者一致。
    let x_range = match which {
        CtrlEnd::Out => (0.0, 0.25),
        CtrlEnd::In => (-0.25, 0.0),
    };
    let new_x = if dx.abs() < 1e-3 {
        0.0
    } else {
        ((mouse.x - rx) / dx / 4.0).clamp(x_range.0, x_range.1)
    };
    // y 方向不限单调（automation 值可任意变化），clamp 到 [-0.5, 0.5]
    let new_y = if dy.abs() < 1e-3 {
        0.0
    } else {
        ((mouse.y - ry) / dy / 4.0).clamp(-0.5, 0.5)
    };
    Some((new_x, new_y))
}

/// 把拖拽出的控制点 (x, y) 按端别合并进 `prev_tick` 事件的 shape。
/// 释放提交与 ghost 预览共用，保证两者生成的 shape 完全一致。
pub(crate) fn merge_ctrl_shape(
    lane: &AutomationLane,
    prev_tick: u32,
    which: CtrlEnd,
    new_ctrl: (f32, f32),
) -> SegmentShape {
    lane.events
        .iter()
        .find(|e| e.tick == prev_tick)
        .map(|e| match e.shape {
            SegmentShape::Curve { x1, y1, x2, y2 } => match which {
                CtrlEnd::Out => SegmentShape::Curve {
                    x1: new_ctrl.0,
                    y1: new_ctrl.1,
                    x2,
                    y2,
                },
                CtrlEnd::In => SegmentShape::Curve {
                    x1,
                    y1,
                    x2: new_ctrl.0,
                    y2: new_ctrl.1,
                },
            },
            SegmentShape::Step => SegmentShape::Step,
        })
        .unwrap_or(SegmentShape::Step)
}
