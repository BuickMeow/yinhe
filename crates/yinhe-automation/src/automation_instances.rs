use yinhe_types::{
    key_notes_in_range, AutomationEvent, AutomationLane, NoteSource, SegmentShape, TRACK_PALETTE,
    TimeSigEvent,
};

use crate::AutomationGhost;
use crate::AutomationPanelView;
use crate::grid;
use yinhe_wgpu::{pack_props, pack_rgba, DrawInstance};
use yinhe_theme::GpuTheme;

/// 折线绘制时的子段像素步长。Linear/Curve 段会按这个步长采样并连成多条 1px 短线，
/// 在保证视觉平滑的同时让 GPU 实例数可控（每段最多 `segment_pixel_len / STEP` 个）。
const CURVE_SUBSAMPLE_PX: f32 = 2.0;
/// 锚点（pencil 工具下显示）的半径，像素。
const ANCHOR_RADIUS: f32 = 3.0;

/// Build background + center line instances (layer 0).
/// Dependencies: none (background is static), lane target (center line)
pub fn build_decor(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    theme: &GpuTheme,
) {
    out.push(DrawInstance {
        x: 0.0,
        y: 0.0,
        w,
        h,
        rgba_packed: pack_rgba(
            theme.pr_bg.0,
            theme.pr_bg.1,
            theme.pr_bg.2,
            1.0,
        ),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: 0,
    });

    if let Some(lane) = lanes.first() {
        let target = &lane.target;
        let max_val = target.max_value() as f32;
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value() as f32;
            let y_center = view.value_to_y(center_val, max_val);
            out.push(DrawInstance {
                x: 0.0,
                y: y_center - 0.5,
                w,
                h: 1.0,
                rgba_packed: pack_rgba(
                    theme.center_line.0,
                    theme.center_line.1,
                    theme.center_line.2,
                    theme.center_line.3,
                ),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
    }
}

/// Build grid line instances (layer 1).
/// Dependencies: scroll_x, pixels_per_tick, time_sig
pub fn build_grid(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    scroll_x_pixel: f32,
    theme: &yinhe_theme::GpuTheme,
) {
    if let Some(tpb) = tpb {
        grid::build_timeline_grid(
            out,
            w,
            h,
            &view.base,
            tpb,
            default_num,
            default_den,
            time_sig_events,
            theme.pr_measure_line,
            theme.pr_beat_line,
            Some(theme.pr_sub_beat_line),
            scroll_x_pixel,
        );
    }
}

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
                out.push(DrawInstance {
                    x: x - ANCHOR_RADIUS,
                    y: y - ANCHOR_RADIUS,
                    w: 2.0 * ANCHOR_RADIUS,
                    h: 2.0 * ANCHOR_RADIUS,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                    props_packed: pack_props(ANCHOR_RADIUS, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }
        }
    }
}

/// 画单条 lane 的线段和锚点（用于 ghost 层）。
fn build_lane_instances(
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
            out.push(DrawInstance {
                x: x - ANCHOR_RADIUS,
                y: y - ANCHOR_RADIUS,
                w: 2.0 * ANCHOR_RADIUS,
                h: 2.0 * ANCHOR_RADIUS,
                rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                props_packed: pack_props(ANCHOR_RADIUS, 0.0),
                velocity: 0,
                tag: 0,
            });
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
            out.push(DrawInstance {
                x: x2 - 0.5,
                y: y1.min(y2),
                w: 1.0,
                h: dy.abs(),
                rgba_packed: pack_rgba(color[0], color[1], color[2], line_alpha),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
        return;
    }

    match shape {
        SegmentShape::Step => {
            // 横线（保持 v1）+ 竖直跳变
            out.push(DrawInstance {
                x: x1,
                y: y1,
                w: dx,
                h: 1.0,
                rgba_packed: pack_rgba(color[0], color[1], color[2], line_alpha),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
            let dy = y2 - y1;
            if dy.abs() > 0.0 {
                out.push(DrawInstance {
                    x: x2 - 0.5,
                    y: y1.min(y2),
                    w: 1.0,
                    h: dy.abs(),
                    rgba_packed: pack_rgba(color[0], color[1], color[2], line_alpha),
                    props_packed: pack_props(0.0, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }
        }
        SegmentShape::Curve { tension: _ } => {
            // Curve{tension:0} = 直线，tension≠0 = 弯曲。统一用 interpolate 子采样。
            push_polyline(out, x1, y1, x2, y2, |t| shape.interpolate(t), color);
        }
    }
}

/// 沿 `(x1,y1) → (x2,y2)` 用 `factor_fn(t)` 控制插值因子，按像素步长子采样并画折线。
fn push_polyline(
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
                out.push(DrawInstance {
                    x: px.min(nx),
                    y: py - 0.5,
                    w: seg_dx.abs().max(1.0),
                    h: 1.0,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], line_alpha),
                    props_packed: pack_props(0.0, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            } else {
                out.push(DrawInstance {
                    x: px - 0.5,
                    y: py.min(ny),
                    w: 1.0,
                    h: seg_dy.abs().max(1.0),
                    rgba_packed: pack_rgba(color[0], color[1], color[2], line_alpha),
                    props_packed: pack_props(0.0, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }
        }
        px = nx;
        py = ny;
    }
}

/// Build velocity bars from NoteSource (layer 2, replaces data bars for Velocity).
///
/// `display_mode`: 0=柱状(2px竖条), 1=矩形(填充), 2=空心矩形(边框)
pub fn build_velocity_bars(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &AutomationPanelView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    display_mode: u32,
    _theme: &GpuTheme,
) {
    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;
    let x_offset = view.base.left_panel_width - view.base.scroll_x;

    struct VelBar {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 3],
        vel: u32,
        duration: u32,
        start_tick: u32,
    }

    let mut bars: Vec<VelBar> = Vec::new();

    for key in 0u8..128 {
        let notes = key_notes_in_range(midi.key_notes(key), pad_start, pad_end);
        for note in notes {
            if note.start_tick as f64 > pad_end as f64 {
                break;
            }
            if (note.end_tick as f64) < pad_start as f64 {
                continue;
            }
            let trk_idx = note.track as usize;
            if !track_visible.get(trk_idx).copied().unwrap_or(true) {
                continue;
            }

            let color = track_colors
                .get(trk_idx)
                .copied()
                .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

            let y = view.value_to_y(note.velocity as f32, 127.0);
            let vel_h = view.value_to_y(0.0, 127.0) - y;

            match display_mode {
                0 => {
                    let bar_x = x_offset + note.start_tick as f32 * ppu;
                    if bar_x + 2.0 < 0.0 || bar_x > w {
                        continue;
                    }
                    bars.push(VelBar {
                        x: bar_x,
                        y,
                        w: 2.0,
                        h: vel_h,
                        color,
                        vel: note.velocity as u32,
                        duration: note.end_tick - note.start_tick,
                        start_tick: note.start_tick,
                    });
                }
                _ => {
                    let raw_x = x_offset + note.start_tick as f32 * ppu;
                    let raw_end = x_offset + note.end_tick as f32 * ppu;
                    let nx = raw_x;
                    let nw = (raw_end - raw_x).max(2.0);
                    if nx + nw < 0.0 || nx > w {
                        continue;
                    }
                    bars.push(VelBar {
                        x: nx,
                        y,
                        w: nw,
                        h: vel_h,
                        color,
                        vel: note.velocity as u32,
                        duration: note.end_tick - note.start_tick,
                        start_tick: note.start_tick,
                    });
                }
            }
        }
    }

    // Sort: shorter notes on top (later in draw order), then later-starting,
    // then softer.  This ensures overlapping bars don't fully hide short notes.
    bars.sort_by(|a, b| {
        a.duration.cmp(&b.duration)
            .then(b.start_tick.cmp(&a.start_tick))
            .then(a.vel.cmp(&b.vel))
    });

    let alpha = if display_mode == 1 { 1.0 } else { 0.85 };
    let border = if display_mode == 2 { 1.0 } else { 0.0 };
    let fill_alpha = if display_mode == 2 { 0.0 } else { alpha };

    for bar in &bars {
        out.push(DrawInstance {
            x: bar.x,
            y: bar.y,
            w: bar.w,
            h: bar.h,
            rgba_packed: pack_rgba(bar.color[0], bar.color[1], bar.color[2], fill_alpha),
            props_packed: pack_props(0.0, border),
            velocity: bar.vel,
            tag: 0,
        });
    }
}

/// Build stepped-line instances for tempo curve (layer 2).
///
/// Renders each tempo event as a staircase: horizontal line (bpm held) +
/// vertical line (bpm change).  Range is [0, max_bpm] where max_bpm is the
/// highest BPM across all tempo events.
pub fn build_tempo_lines(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    tempo_events: &[(u32, f64)],
    _theme: &GpuTheme,
) {
    if tempo_events.is_empty() {
        return;
    }

    let max_bpm = tempo_events
        .iter()
        .map(|(_, bpm)| *bpm)
        .fold(0.0f64, f64::max) as f32;
    if max_bpm <= 0.0 {
        return;
    }

    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;
    let x_offset = view.base.left_panel_width - view.base.scroll_x;
    let grid_left_x = view.base.left_panel_width;

    // Find first visible event index
    let vis_start = tempo_events.partition_point(|e| e.0 < pad_start);
    let vis_end = tempo_events.partition_point(|e| e.0 < pad_end);

    if vis_start >= vis_end {
        // No events in visible range: draw full-width line at chase value
        let chase_idx = if vis_start > 0 { vis_start - 1 } else { 0 };
        let val = tempo_events[chase_idx].1 as f32;
        let y = view.value_to_y(val, max_bpm);
        if w > grid_left_x {
            out.push(DrawInstance {
                x: grid_left_x,
                y,
                w: w - grid_left_x,
                h: 1.0,
                rgba_packed: pack_rgba(0.80, 0.30, 0.30, 0.85),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
        return;
    }

    // Value before first visible event (chase)
    let prev_idx = if vis_start > 0 { vis_start - 1 } else { 0 };
    let mut prev_val = tempo_events[prev_idx].1 as f32;
    let mut prev_tick = tempo_events[prev_idx].0;

    // Horizontal line from grid left edge to the first visible event
    let first_tick = tempo_events[vis_start].0;
    let first_x = x_offset + first_tick as f32 * ppu;
    let first_y = view.value_to_y(prev_val, max_bpm);
    if first_x > grid_left_x {
        out.push(DrawInstance {
            x: grid_left_x,
            y: first_y,
            w: first_x - grid_left_x,
            h: 1.0,
            rgba_packed: pack_rgba(0.80, 0.30, 0.30, 0.85),
            props_packed: pack_props(0.0, 0.0),
            velocity: 0,
            tag: 0,
        });
    }

    for i in vis_start..vis_end {
        let (tick, bpm) = tempo_events[i];
        let val = bpm as f32;
        let x1 = x_offset + prev_tick as f32 * ppu;
        let x2 = x_offset + tick as f32 * ppu;
        let y1 = view.value_to_y(prev_val, max_bpm);
        let y2 = view.value_to_y(val, max_bpm);

        // Horizontal line: value held from prev_tick to tick
        if x2 > x1 {
            out.push(DrawInstance {
                x: x1,
                y: y1,
                w: x2 - x1,
                h: 1.0,
                rgba_packed: pack_rgba(0.80, 0.30, 0.30, 0.85),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }

        // Vertical line: value change at tick
        let dy = y2 - y1;
        if dy.abs() > 0.0 {
            out.push(DrawInstance {
                x: x2 - 0.5,
                y: y1.min(y2),
                w: 1.0,
                h: dy.abs(),
                rgba_packed: pack_rgba(0.80, 0.30, 0.30, 0.85),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }

        prev_val = val;
        prev_tick = tick;
    }

    // Horizontal line from last visible event to right edge
    let last_x = x_offset + prev_tick as f32 * ppu;
    let last_y = view.value_to_y(prev_val, max_bpm);
    if w > last_x {
        out.push(DrawInstance {
            x: last_x,
            y: last_y,
            w: w - last_x,
            h: 1.0,
            rgba_packed: pack_rgba(0.80, 0.30, 0.30, 0.85),
            props_packed: pack_props(0.0, 0.0),
            velocity: 0,
            tag: 0,
        });
    }
}

/// ghost 锚点半径（像素）。
const GHOST_RADIUS: f32 = 4.0;
/// ghost 不透明度。
const GHOST_ALPHA: f32 = 0.9;

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
        out.push(DrawInstance {
            x: x - GHOST_RADIUS,
            y: y - GHOST_RADIUS,
            w: 2.0 * GHOST_RADIUS,
            h: 2.0 * GHOST_RADIUS,
            rgba_packed: pack_rgba(color[0], color[1], color[2], GHOST_ALPHA),
            props_packed: pack_props(GHOST_RADIUS, 0.0),
            velocity: 0,
            tag: 0,
        });
    };

    match ghost {
        AutomationGhost::Move { lane, color } => {
            // 整条 lane 作为 ghost 重新绘制
            build_lane_instances(out, w, view, &lane, color, show_anchors);
        }
        AutomationGhost::Curve { start_x, start_y, cur_x, cur_y, color } => {
            push_anchor(out, start_x, start_y, color);
            push_polyline(out, start_x, start_y, cur_x, cur_y, |t| t, color);
            push_anchor(out, cur_x, cur_y, color);
        }
    }
}
