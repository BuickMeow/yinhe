use yinhe_types::AutomationPanelView;
use yinhe_theme::GpuTheme;
use yinhe_types::{AutomationEvent, AutomationLane, SegmentShape, TRACK_PALETTE};
use crate::vertex::CurveInstance;

/// 自动化曲线的线宽（SDF 半宽，像素）。视觉宽度 ≈ 2×thickness + 1px AA。
/// 0.5 ≈ 原 1px 矩形拟合的视觉宽度，但带 AA 抗锯齿。
const LINE_THICKNESS: f32 = 0.5;
/// 锚点（pencil 工具下显示）的半径/半边长，像素。
const ANCHOR_RADIUS: f32 = 3.0;
/// 贝塞尔控制点（空心圆）外半径，像素。
const CTRL_POINT_RADIUS: f32 = 4.0;
/// 控制点到锚点的连线线宽。
const CTRL_HANDLE_THICKNESS: f32 = 0.5;
/// 控制点连线的不透明度（比锚点淡）。
const CTRL_HANDLE_ALPHA: f32 = 0.5;
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
    /// Curve 段采样所需（Step/chase/right 段填 0，不用）
    tick1: u32,
    tick2: u32,
    v1: f32,
    v2: f32,
}

/// 把 lane.events 转换成需要绘制的段列表。
///
/// # 段的类型
/// - **chase 段**：从 grid 左边缘到第一个可见事件（Step shape，保持 chase 值）
/// - **event 段**：从一个事件到下一个事件（用前一个事件的 shape）
/// - **right 段**：从最后一个事件到右边界（Step shape，保持最后值）
///
/// 单锚点情况下，visible_events 只有 1 个事件，循环第一次迭代产生
/// dx=0 的竖直跳变段（chase_y → evt.value），render_segment 走 dx<=0 分支
/// 正确画竖线；right 段从锚点画到右边界，保持锚点值。
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
        let val = if idx > 0 { lane.events[idx - 1].value } else { 0.0 };
        let y = view.value_to_y(val, max_val);
        if w > grid_left_x {
            segs.push(SegSpan { x1: grid_left_x, y1: y, shape: SegmentShape::Step, x2: w, y2: y, tick1: 0, tick2: 0, v1: 0.0, v2: 0.0 });
        }
        return segs;
    }

    // chase 值（第一个可见事件之前的值）
    let prev_idx = lane.events.partition_point(|e| e.tick < visible_events[0].tick);
    let chase_val = if prev_idx > 0 { lane.events[prev_idx - 1].value } else { 0.0 };
    let first_tick = visible_events[0].tick;
    let first_x = x_offset + first_tick as f32 * ppu;
    let chase_y = view.value_to_y(chase_val, max_val);

    // chase 段：grid_left → first_event
    if first_x > grid_left_x {
        segs.push(SegSpan { x1: grid_left_x, y1: chase_y, shape: SegmentShape::Step, x2: first_x, y2: chase_y, tick1: 0, tick2: 0, v1: 0.0, v2: 0.0 });
    }

    // 事件段：prev → cur（prev 从虚拟 chase 段尾端开始，每次迭代后 prev = evt）
    let mut prev_x = first_x;
    let mut prev_y = chase_y;
    let mut prev_shape = SegmentShape::Step;
    let mut prev_tick: u32 = 0;
    let mut prev_value = chase_val;

    for evt in visible_events {
        let x2 = x_offset + evt.tick as f32 * ppu;
        let y2 = view.value_to_y(evt.value, max_val);
        segs.push(SegSpan {
            x1: prev_x, y1: prev_y, shape: prev_shape, x2, y2,
            tick1: prev_tick, tick2: evt.tick, v1: prev_value, v2: evt.value,
        });
        prev_shape = evt.shape;
        prev_x = x2;
        prev_y = y2;
        prev_tick = evt.tick;
        prev_value = evt.value;
    }

    // right 段：last_event → right_bound
    let last_visible_tick = visible_events[visible_events.len() - 1].tick;
    let next_idx = lane.events.partition_point(|e| e.tick <= last_visible_tick);
    let right_bound = if next_idx < lane.events.len() {
        x_offset + lane.events[next_idx].tick as f32 * ppu
    } else {
        w
    };
    if right_bound > prev_x {
        segs.push(SegSpan {
            x1: prev_x, y1: prev_y, shape: SegmentShape::Step, x2: right_bound, y2: prev_y,
            tick1: prev_tick, tick2: 0, v1: prev_value, v2: 0.0,
        });
    }

    segs
}

/// Build data line instances (layer 1, curve pipeline).
///
/// 渲染每条 lane 的线段和锚点。被 ghost 覆盖的 lane 由调用方通过 `skip_lane`
/// 跳过，本函数只画未被覆盖的 lane。
///
/// `max_val` 由调用方传入：Tempo 时为实际事件的最大值（动态），其他 target
/// 时为 `target.max_value()`。所有 lane 必须共享同一 max_val。
pub fn build_data_lines(
    out: &mut Vec<CurveInstance>,
    w: f32,
    _h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    max_val: f32,
    track_visible: &[bool],
    track_color: &[[f32; 3]],
    show_anchors: bool,
    skip_lane: Option<&AutomationLane>,
    highlight_tick: Option<u32>,
    _theme: &GpuTheme,
) {
    if lanes.is_empty() || max_val <= 0.0 {
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
            render_segment(out, seg, view, max_val, color);
        }

        // 画锚点 + 曲线段的控制点
        if show_anchors {
            let visible_events = lane.events_in_range(pad_start, pad_end);
            // 锚点形状按 shape 分派：Step → 方形，Curve → 圆形
            for evt in visible_events {
                let x = x_offset + evt.tick as f32 * ppu;
                let y = view.value_to_y(evt.value, max_val);
                // 选中锚点渲染为白色高亮
                let anchor_color = if highlight_tick == Some(evt.tick) {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [color[0], color[1], color[2], 1.0]
                };
                match evt.shape {
                    SegmentShape::Step => out.push(CurveInstance::square(x, y, ANCHOR_RADIUS, anchor_color)),
                    SegmentShape::Curve { .. } => out.push(CurveInstance::circle(x, y, ANCHOR_RADIUS, anchor_color)),
                }
            }
            // 曲线段中间的空心控制点（仅 Curve 段，非直线时才画）
            push_curve_control_points(out, lane, &visible_events, view, max_val, x_offset, ppu, color);
        }
    }
}

/// 画单条 lane 的线段和锚点（用于 ghost 层）。
pub(crate) fn build_lane_instances(
    out: &mut Vec<CurveInstance>,
    w: f32,
    view: &AutomationPanelView,
    lane: &AutomationLane,
    max_val: f32,
    color: [f32; 3],
    show_anchors: bool,
) {
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
        render_segment(out, seg, view, max_val, color);
    }

    if show_anchors {
        let visible_events = lane.events_in_range(pad_start, pad_end);
        for evt in visible_events {
            let x = x_offset + evt.tick as f32 * ppu;
            let y = view.value_to_y(evt.value, max_val);
            let anchor_color = [color[0], color[1], color[2], 1.0];
            match evt.shape {
                SegmentShape::Step => out.push(CurveInstance::square(x, y, ANCHOR_RADIUS, anchor_color)),
                SegmentShape::Curve { .. } => out.push(CurveInstance::circle(x, y, ANCHOR_RADIUS, anchor_color)),
            }
        }
        push_curve_control_points(out, lane, &visible_events, view, max_val, x_offset, ppu, color);
    }
}

/// 为 lane 中每个 Curve 段（前一个事件 → 当前事件，shape=Curve 且非直线）
/// 在两个控制点位置各画一个空心圆，并从锚点画线段连接到对应控制点（CSS 风格 handle）。
///
/// 段的 shape 取自前一个事件。前驱从 `visible_events[0]` 在 `lane.events` 中的
/// 前一个事件开始（可能是窗口外的 chase 事件，控制点位置可能在窗口外但会被
/// GPU 裁剪）。偏移量参数化（内部 *4 放大）：
///   c1 = P0 + (P3 - P0) · (x1·4, y1·4)  — P1 相对 P0（起点出）
///   c2 = P3 + (P3 - P0) · (x2·4, y2·4)  — P2 相对 P3（终点入）
fn push_curve_control_points(
    out: &mut Vec<CurveInstance>,
    lane: &AutomationLane,
    visible_events: &[AutomationEvent],
    view: &AutomationPanelView,
    max_val: f32,
    x_offset: f32,
    ppu: f32,
    color: [f32; 3],
) {
    let ctrl_color = [color[0], color[1], color[2], 1.0];
    let handle_color = [color[0], color[1], color[2], CTRL_HANDLE_ALPHA];
    // 前驱事件（visible 之前最后一个事件，作为 chase 段的起点）
    let first_tick = visible_events.first().map(|e| e.tick).unwrap_or(0);
    let prev_idx = lane.events.partition_point(|e| e.tick < first_tick);
    let mut prev: Option<&AutomationEvent> = if prev_idx > 0 {
        Some(&lane.events[prev_idx - 1])
    } else {
        None
    };
    for evt in visible_events {
        if let Some(p) = prev
            && let SegmentShape::Curve { x1, y1, x2, y2 } = p.shape
            && !p.shape.is_linear()
        {
            // 段 p → evt：P0=p, P3=evt
            let px0 = x_offset + p.tick as f32 * ppu;
            let py0 = view.value_to_y(p.value, max_val);
            let px3 = x_offset + evt.tick as f32 * ppu;
            let py3 = view.value_to_y(evt.value, max_val);
            // 两个控制点屏幕坐标（偏移量 *4 放大）
            let c1x = px0 + (px3 - px0) * x1 * 4.0;
            let c1y = py0 + (py3 - py0) * y1 * 4.0;
            let c2x = px3 + (px3 - px0) * x2 * 4.0;
            let c2y = py3 + (py3 - py0) * y2 * 4.0;
            // 锚点 → 控制点的连线（handle）
            out.push(CurveInstance::line(px0, py0, c1x, c1y, CTRL_HANDLE_THICKNESS, handle_color));
            out.push(CurveInstance::line(px3, py3, c2x, c2y, CTRL_HANDLE_THICKNESS, handle_color));
            // 两个空心圆控制点
            out.push(CurveInstance::hollow_circle(c1x, c1y, CTRL_POINT_RADIUS, ctrl_color));
            out.push(CurveInstance::hollow_circle(c2x, c2y, CTRL_POINT_RADIUS, ctrl_color));
        }
        prev = Some(evt);
    }
}

/// 渲染从 `(x1, y1)` 到 `(x2, y2)` 的一段自动化曲线，按 shape 决定形状。
///
/// - `Step` → push 两个 line instance：水平段 + 竖直跳变段
/// - `Curve` → 按 DAW 标准做法 CPU 采样生成折线（render_curve_polyline）
fn render_segment(
    out: &mut Vec<CurveInstance>,
    seg: &SegSpan,
    view: &AutomationPanelView,
    max_val: f32,
    color: [f32; 3],
) {
    let line_color = [color[0], color[1], color[2], LINE_ALPHA];
    let dx = seg.x2 - seg.x1;
    if dx <= 0.0 {
        // 同一 tick 的多事件：只画竖直跳变
        let dy = seg.y2 - seg.y1;
        if dy.abs() > 0.0 {
            out.push(CurveInstance::line(seg.x1, seg.y1, seg.x2, seg.y2, LINE_THICKNESS, line_color));
        }
        return;
    }

    match seg.shape {
        SegmentShape::Step => {
            // 横线（保持 v1）+ 竖直跳变
            out.push(CurveInstance::line(seg.x1, seg.y1, seg.x2, seg.y1, LINE_THICKNESS, line_color));
            let dy = seg.y2 - seg.y1;
            if dy.abs() > 0.0 {
                out.push(CurveInstance::line(seg.x2, seg.y1, seg.x2, seg.y2, LINE_THICKNESS, line_color));
            }
        }
        SegmentShape::Curve { .. } => {
            render_curve_polyline(out, seg, view, max_val, line_color);
        }
    }
}

/// 按 DAW 标准做法：对每个屏幕像素列调一次 interpolate，生成折线。
/// 曲线在时间轴上单调（x1/x2 clamp 到 CSS 区间），保证 interpolate 语义正确。
/// 直线段（is_linear）或段宽 < 2px 时退化成单条 line。
fn render_curve_polyline(
    out: &mut Vec<CurveInstance>,
    seg: &SegSpan,
    view: &AutomationPanelView,
    max_val: f32,
    line_color: [f32; 4],
) {
    let px0 = seg.x1;
    let px3 = seg.x2;
    let dx = px3 - px0;
    let span = seg.tick2.saturating_sub(seg.tick1) as f32;
    // 直线或段宽不足：退化成单条 line
    if seg.shape.is_linear() || span <= 0.0 || dx < 2.0 {
        out.push(CurveInstance::line(px0, seg.y1, px3, seg.y2, LINE_THICKNESS, line_color));
        return;
    }

    // 按屏幕像素列采样 interpolate
    let first_px = px0.floor() as i32;
    let last_px = px3.ceil() as i32;
    let n = (last_px - first_px).max(1);
    let mut prev_x = px0;
    let mut prev_y = seg.y1;
    for i in 1..=n {
        let px = if i == n { px3 } else { (first_px + i) as f32 };
        let t = ((px - px0) / dx).clamp(0.0, 1.0);
        let f = seg.shape.interpolate(t);
        let v = seg.v1 + (seg.v2 - seg.v1) * f;
        let y = view.value_to_y(v, max_val);
        out.push(CurveInstance::line(prev_x, prev_y, px, y, LINE_THICKNESS, line_color));
        prev_x = px;
        prev_y = y;
    }
}
