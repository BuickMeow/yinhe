use yinhe_types::AutomationPanelView;
use yinhe_theme::GpuTheme;
use yinhe_types::{AutomationEvent, AutomationLane, SegmentShape, TRACK_PALETTE};
use yinhe_wgpu::{pack_props, pack_rgba, DrawInstance};

/// 折线绘制时的子段像素步长。Linear/Curve 段会按这个步长采样并连成多条 1px 短线，
/// 在保证视觉平滑的同时让 GPU 实例数可控（每段最多 `segment_pixel_len / STEP` 个）。
const CURVE_SUBSAMPLE_PX: f32 = 2.0;
/// 锚点（pencil 工具下显示）的半径，像素。
const ANCHOR_RADIUS: f32 = 3.0;

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

/// Build data line instances (layer 2).
///
/// 渲染每条 lane 的线段和锚点。被 ghost 覆盖的 lane 由调用方通过 `skip_lane`
/// 跳过，本函数只画未被覆盖的 lane。
pub fn build_data_lines(
    out: &mut Vec<DrawInstance>,
    w: f32,
    _h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    show_anchors: bool,
    skip_lane: Option<&AutomationLane>,
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
        let color = track_colors
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
                out.push(DrawInstance::with_props(
                    x - ANCHOR_RADIUS, y - ANCHOR_RADIUS,
                    2.0 * ANCHOR_RADIUS, 2.0 * ANCHOR_RADIUS,
                    [color[0], color[1], color[2], 1.0],
                    ANCHOR_RADIUS, 0.0,
                ));
            }
        }
    }
}

/// 画单条 lane 的线段和锚点（用于 ghost 层）。
pub(crate) fn build_lane_instances(
    out: &mut Vec<DrawInstance>,
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
            out.push(DrawInstance::with_props(
                x - ANCHOR_RADIUS, y - ANCHOR_RADIUS,
                2.0 * ANCHOR_RADIUS, 2.0 * ANCHOR_RADIUS,
                [color[0], color[1], color[2], 1.0],
                ANCHOR_RADIUS, 0.0,
            ));
        }
    }
}

/// 渲染从 `(x1, y1)` 到 `(x2, y2)` 的一段自动化曲线，按 shape 决定形状。
fn render_segment(
    out: &mut Vec<DrawInstance>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    shape: SegmentShape,
    color: [f32; 3],
) {
    let line_alpha = 0.85;
    let dx = x2 - x1;
    if dx <= 0.0 {
        // 同一 tick 的多事件：只画竖直跳变
        let dy = y2 - y1;
        if dy.abs() > 0.0 {
            out.push(DrawInstance::solid_rect(
                x2 - 0.5, y1.min(y2), 1.0, dy.abs(),
                [color[0], color[1], color[2], line_alpha],
            ));
        }
        return;
    }

    match shape {
        SegmentShape::Step => {
            // 横线（保持 v1）+ 竖直跳变
            out.push(DrawInstance::solid_rect(
                x1, y1, dx, 1.0,
                [color[0], color[1], color[2], line_alpha],
            ));
            let dy = y2 - y1;
            if dy.abs() > 0.0 {
                out.push(DrawInstance::solid_rect(
                    x2 - 0.5, y1.min(y2), 1.0, dy.abs(),
                    [color[0], color[1], color[2], line_alpha],
                ));
            }
        }
        SegmentShape::Curve { tension: _ } => {
            // Curve{tension:0} = 直线，tension≠0 = 弯曲。统一用 interpolate 子采样。
            push_polyline(out, x1, y1, x2, y2, |t| shape.interpolate(t), color);
        }
    }
}

/// 沿 `(x1,y1) → (x2,y2)` 用 `factor_fn(t)` 控制插值因子，按像素步长子采样并画折线。
pub(crate) fn push_polyline(
    out: &mut Vec<DrawInstance>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    factor_fn: impl Fn(f32) -> f32,
    color: [f32; 3],
) {
    let line_alpha = 0.85;
    let dx = x2 - x1;
    let dy = y2 - y1;
    let pixel_len = dx.hypot(dy);
    if pixel_len < 1.0 {
        return;
    }
    let steps = ((pixel_len / CURVE_SUBSAMPLE_PX).ceil() as usize).max(1);
    let inv = 1.0 / steps as f32;
    let mut px = x1;
    let mut py = y1;
    for i in 1..=steps {
        let t = i as f32 * inv;
        let f = factor_fn(t);
        let nx = x1 + dx * t;
        let ny = y1 + dy * f;
        // 画 (px,py) → (nx,ny) 的 1px 线段
        let seg_dx = nx - px;
        let seg_dy = ny - py;
        let len = seg_dx.hypot(seg_dy);
        if len > 0.5 {
            // 用一个细矩形表示该子段。角度通过近似水平/垂直分解表达：
            // 简单起见，按主导方向画一条水平或竖直短线（像素级足够平滑）。
            if seg_dx.abs() >= seg_dy.abs() {
                out.push(DrawInstance::solid_rect(
                    px.min(nx), py - 0.5, seg_dx.abs().max(1.0), 1.0,
                    [color[0], color[1], color[2], line_alpha],
                ));
            } else {
                out.push(DrawInstance::solid_rect(
                    px - 0.5, py.min(ny), 1.0, seg_dy.abs().max(1.0),
                    [color[0], color[1], color[2], line_alpha],
                ));
            }
        }
        px = nx;
        py = ny;
    }
}
