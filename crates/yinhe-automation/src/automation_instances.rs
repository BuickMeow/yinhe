use yinhe_types::{
    key_notes_in_range, AutomationEvent, AutomationLane, NoteSource, SegmentShape, TRACK_PALETTE,
    TimeSigEvent,
};

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
            let y_center = h - (center_val / max_val) * h;
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

/// Build data line instances (layer 2).
///
/// 渲染每条 lane：
/// - `Step` 段（默认）：保持值到下一点再瞬间跳变 → 阶梯线
/// - `Linear` 段：直线
/// - `Curve { tension }` 段：tension 控制的 ease-in/out 曲线，按像素步长子采样
///
/// `show_anchors = true` 时在每个事件位置画一个圆形锚点（铅笔工具下）。
pub fn build_data_lines(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    show_anchors: bool,
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
    let line_alpha = 0.85;

    let push_h_line = |out: &mut Vec<DrawInstance>, x: f32, y: f32, len: f32, color: [f32; 3]| {
        if len > 0.0 {
            out.push(DrawInstance {
                x,
                y,
                w: len,
                h: 1.0,
                rgba_packed: pack_rgba(color[0], color[1], color[2], line_alpha),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
    };

    for lane in lanes {
        let trk_idx = lane.track as usize;
        if !track_visible.get(trk_idx).copied().unwrap_or(true) {
            continue;
        }
        let color = track_colors
            .get(trk_idx)
            .copied()
            .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

        let visible_events = lane.events_in_range(pad_start, pad_end);

        // 无可见事件：在 chase 值处画一条横贯网格的横线
        if visible_events.is_empty() {
            let idx = lane.events.partition_point(|e| e.tick < pad_start);
            let val = if idx > 0 { lane.events[idx - 1].value } else { 0 };
            let y = h - (val as f32 / max_val) * h;
            if w > grid_left_x {
                push_h_line(out, grid_left_x, y, w - grid_left_x, color);
            }
            continue;
        }

        // 第一个可见事件之前的值（chase）
        let prev_idx = lane.events.partition_point(|e| e.tick < visible_events[0].tick);
        let prev_val = if prev_idx > 0 { lane.events[prev_idx - 1].value } else { 0 };
        let prev_tick = visible_events[0].tick;
        // 起始段（从 chase 值到第一个事件）按 Step 处理：保持 chase 值
        let mut prev_shape = SegmentShape::Step;
        let mut prev_x = x_offset + prev_tick as f32 * ppu;
        let mut prev_y = h - (prev_val as f32 / max_val) * h;

        // 从网格左边缘到第一个事件的横线（保持 chase 值）
        if prev_x > grid_left_x {
            push_h_line(out, grid_left_x, prev_y, prev_x - grid_left_x, color);
        }

        for evt in visible_events {
            let tick = evt.tick;
            let value = evt.value;
            let x2 = x_offset + tick as f32 * ppu;
            let y2 = h - (value as f32 / max_val) * h;

            render_segment(out, prev_x, prev_y, x2, y2, prev_shape, color);

            // 锚点（pencil 时显示）画在事件位置
            if show_anchors {
                out.push(DrawInstance {
                    x: x2 - ANCHOR_RADIUS,
                    y: y2 - ANCHOR_RADIUS,
                    w: 2.0 * ANCHOR_RADIUS,
                    h: 2.0 * ANCHOR_RADIUS,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                    props_packed: pack_props(ANCHOR_RADIUS, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }

            prev_shape = evt.shape;
            prev_x = x2;
            prev_y = y2;
        }

        // 最后一个事件 → 右边界：保持最后值到右边界
        let next_idx = lane.events.partition_point(|e| e.tick <= visible_events.last().unwrap().tick);
        let right_bound = if next_idx < lane.events.len() {
            x_offset + lane.events[next_idx].tick as f32 * ppu
        } else {
            w
        };
        if right_bound > prev_x {
            push_h_line(out, prev_x, prev_y, right_bound - prev_x, color);
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
        SegmentShape::Linear => {
            // 直线：用一条对角矩形近似（在像素级足够平滑）
            // 简单实现：画一系列 1px 短水平段。对 1 亿音符场景太重，
            // 这里直接画一条对角线（细矩形旋转近似）。
            // 为了视觉平滑且实现简单，按像素步长子采样。
            push_polyline(out, x1, y1, x2, y2, |t| t, color);
        }
        SegmentShape::Curve { tension } => {
            push_polyline(out, x1, y1, x2, y2, |t| shape.interpolate(t), color);
            // 避免未使用 tension 警告（interpolate 内部已使用）
            let _ = tension;
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

            let vel_h = (note.velocity as f32 / 127.0) * h;

            match display_mode {
                0 => {
                    let bar_x = x_offset + note.start_tick as f32 * ppu;
                    if bar_x + 2.0 < 0.0 || bar_x > w {
                        continue;
                    }
                    bars.push(VelBar {
                        x: bar_x,
                        y: h - vel_h,
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
                        y: h - vel_h,
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
        let y = h - (val / max_bpm) * h;
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
    let first_y = h - (prev_val / max_bpm) * h;
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
        let y1 = h - (prev_val / max_bpm) * h;
        let y2 = h - (val / max_bpm) * h;

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
    let last_y = h - (prev_val / max_bpm) * h;
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
