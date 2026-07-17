use yinhe_types::AutomationPanelView;
use yinhe_theme::GpuTheme;
use yinhe_types::{AutomationLane, SegmentShape, TRACK_PALETTE};
use crate::vertex::CurveInstance;

/// 自动化曲线的线宽（SDF 半宽，像素）。视觉宽度 ≈ 2×thickness + 1px AA。
/// 0.5 ≈ 原 1px 矩形拟合的视觉宽度，但带 AA 抗锯齿。
const LINE_THICKNESS: f32 = 0.5;
/// 锚点（pencil 工具下显示）的半径，像素。
const ANCHOR_RADIUS: f32 = 3.0;
/// 自动化线段的不透明度。
const LINE_ALPHA: f32 = 0.85;

/// 一个需要绘制的线段（panel 局部像素坐标）。
///
/// `shape` 描述 `(x1,y1) → (x2,y2)` 的插值方式。
struct SegSpan {
    x1: f32,
    y1: f32,
    shape: SegmentShape,
    x2: f32,
    y2: f32,
}

/// 把 lane.events 转换成需要绘制的段列表。
///
/// # 段的类型
/// - **chase 段**：从 grid 左边缘到第一个可见事件（Step shape，保持 chase 值）
/// - **event 段**：从一个事件到下一个事件（用前一个事件的 shape）
/// - **right 段**：从最后一个事件到右边界（Step shape，保持最后值）
fn collect_segments(
    lane: &AutomationLane,
    view: &AutomationPanelView,
    max_val: f32,
    w: f32,
    pad_start: u32,
    pad_end: u32,
    x_offset: f32,
    grid_left_x: f32,
) -> Vec<SegSpan> {
    let ppu = view.base.pixels_per_tick;
    let visible_events = lane.events_in_range(pad_start, pad_end);
    let mut segs = Vec::new();

    // 无可见事件：在 chase 值处画一条横贯网格的横线
    if visible_events.is_empty() {
        let idx = lane.events.partition_point(|e| e.tick < pad_start);
        let val = if idx > 0 { lane.events[idx - 1].value } else { 0 };
        let y = view.value_to_y(val as f32, max_val);
        if w > grid_left_x {
            segs.push(SegSpan { x1: grid_left_x, y1: y, shape: SegmentShape::Step, x2: w, y2: y });
        }
        return segs;
    }

    // chase 值（第一个可见事件之前的值）
    let prev_idx = lane.events.partition_point(|e| e.tick < visible_events[0].tick);
    let chase_val = if prev_idx > 0 { lane.events[prev_idx - 1].value } else { 0 };
    let first_tick = visible_events[0].tick;
    let first_x = x_offset + first_tick as f32 * ppu;
    let chase_y = view.value_to_y(chase_val as f32, max_val);

    // chase 段：grid_left → first_event
    if first_x > grid_left_x {
        segs.push(SegSpan { x1: grid_left_x, y1: chase_y, shape: SegmentShape::Step, x2: first_x, y2: chase_y });
    }

    // 事件段：prev → cur
    let mut prev_x = first_x;
    let mut prev_y = chase_y;
    let mut prev_shape = SegmentShape::Step;

    for evt in visible_events {
        let x2 = x_offset + evt.tick as f32 * ppu;
        let y2 = view.value_to_y(evt.value as f32, max_val);
        segs.push(SegSpan { x1: prev_x, y1: prev_y, shape: prev_shape, x2, y2 });

        prev_shape = evt.shape;
        prev_x = x2;
        prev_y = y2;
    }

    // right 段：last_event → right_bound
    let last_visible_tick = visible_events.last().unwrap().tick;
    let next_idx = lane.events.partition_point(|e| e.tick <= last_visible_tick);
    let right_bound = if next_idx < lane.events.len() {
        x_offset + lane.events[next_idx].tick as f32 * ppu
    } else {
        w
    };
    if right_bound > prev_x {
        segs.push(SegSpan { x1: prev_x, y1: prev_y, shape: SegmentShape::Step, x2: right_bound, y2: prev_y });
    }

    segs
}

/// Build data line instances (layer 1, curve pipeline).
///
/// 渲染每条 lane 的线段和锚点。被 ghost 覆盖的 lane 由调用方通过 `skip_lane`
/// 跳过，本函数只画未被覆盖的 lane。
pub fn build_data_lines(
    out: &mut Vec<CurveInstance>,
    w: f32,
    _h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    track_visible: &[bool],
    track_color: &[[f32; 3]],
    show_anchors: bool,
    skip_lane: Option<&AutomationLane>,
    highlight_tick: Option<u32>,
    _theme: &GpuTheme,
) {
    if lanes.is_empty() {
        return;
    }
    let target = &lanes[0].target;
    let max_val = target.max_value() as f32;
    if max_val <= 0.0 {
        return;
    }

    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;
    let x_offset = view.base.left_panel_width - view.base.scroll_x;
    let grid_left_x = view.base.left_panel_width;

    for lane in lanes {
        // 跳过被 ghost 覆盖的 lane（通过 track + target 匹配）
        if skip_lane.map(|sl| sl.track == lane.track && sl.target == lane.target).unwrap_or(false) {
            continue;
        }

        let trk_idx = lane.track as usize;
        if !track_visible.get(trk_idx).copied().unwrap_or(true) {
            continue;
        }
        let color = track_color
            .get(trk_idx)
            .copied()
            .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

        // 收集并绘制所有段
        let segs = collect_segments(lane, view, max_val, w, pad_start, pad_end, x_offset, grid_left_x);
        for seg in &segs {
            render_segment(out, seg.x1, seg.y1, seg.x2, seg.y2, seg.shape, color);
        }

        // 画锚点
        if show_anchors {
            let visible_events = lane.events_in_range(pad_start, pad_end);
            for evt in visible_events {
                let x = x_offset + evt.tick as f32 * ppu;
                let y = view.value_to_y(evt.value as f32, max_val);
                // 选中锚点渲染为白色高亮
                let anchor_color = if highlight_tick == Some(evt.tick) {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [color[0], color[1], color[2], 1.0]
                };
                out.push(CurveInstance::circle(x, y, ANCHOR_RADIUS, anchor_color));
            }
        }
    }
}

/// 画单条 lane 的线段和锚点（用于 ghost 层）。
pub(crate) fn build_lane_instances(
    out: &mut Vec<CurveInstance>,
    w: f32,
    view: &AutomationPanelView,
    lane: &AutomationLane,
    color: [f32; 3],
    show_anchors: bool,
) {
    let target = &lane.target;
    let max_val = target.max_value() as f32;
    if max_val <= 0.0 {
        return;
    }

    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;
    let x_offset = view.base.left_panel_width - view.base.scroll_x;
    let grid_left_x = view.base.left_panel_width;
    let ppu = view.base.pixels_per_tick;

    let segs = collect_segments(lane, view, max_val, w, pad_start, pad_end, x_offset, grid_left_x);
    for seg in &segs {
        render_segment(out, seg.x1, seg.y1, seg.x2, seg.y2, seg.shape, color);
    }

    if show_anchors {
        let visible_events = lane.events_in_range(pad_start, pad_end);
        for evt in visible_events {
            let x = x_offset + evt.tick as f32 * ppu;
            let y = view.value_to_y(evt.value as f32, max_val);
            out.push(CurveInstance::circle(x, y, ANCHOR_RADIUS, [color[0], color[1], color[2], 1.0]));
        }
    }
}

/// 渲染从 `(x1, y1)` 到 `(x2, y2)` 的一段自动化曲线，按 shape 决定形状。
///
/// - `Step` → push 两个 line instance：水平段 + 竖直跳变段
/// - `Curve{tension}` → push 一个 curve instance（tension=0 时退化为直线）
fn render_segment(
    out: &mut Vec<CurveInstance>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    shape: SegmentShape,
    color: [f32; 3],
) {
    let line_color = [color[0], color[1], color[2], LINE_ALPHA];
    let dx = x2 - x1;
    if dx <= 0.0 {
        // 同一 tick 的多事件：只画竖直跳变
        let dy = y2 - y1;
        if dy.abs() > 0.0 {
            out.push(CurveInstance::line(x1, y1, x2, y2, LINE_THICKNESS, line_color));
        }
        return;
    }

    match shape {
        SegmentShape::Step => {
            // 横线（保持 v1）+ 竖直跳变
            out.push(CurveInstance::line(x1, y1, x2, y1, LINE_THICKNESS, line_color));
            let dy = y2 - y1;
            if dy.abs() > 0.0 {
                out.push(CurveInstance::line(x2, y1, x2, y2, LINE_THICKNESS, line_color));
            }
        }
        SegmentShape::Curve { tension } => {
            // tension=0 → 直线；tension≠0 → 二次曲线。统一用一个 curve instance。
            let tension_norm = (tension as f32) / 127.0;
            out.push(CurveInstance::curve(
                x1, y1, x2, y2, LINE_THICKNESS, tension_norm, line_color,
            ));
        }
    }
}
