use yinhe_types::{AutomationEvent, AutomationLane, NoteSource, TRACK_PALETTE, TimeSigEvent};

use crate::AutomationPanelView;
use crate::grid;
use yinhe_wgpu::{DrawInstance, pack_props, pack_rgba};
use yinhe_theme::GpuTheme;

/// Bar width in pixels for automation events.
const BAR_WIDTH: f32 = 2.0;
/// Corner radius for automation bars (flat).
const BAR_ROUNDING: f32 = 0.0;
/// Border width for automation bars.
const BAR_BORDER: f32 = 0.0;

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
    measure_color: (f32, f32, f32, f32),
    beat_color: (f32, f32, f32, f32),
    sub_beat_color: Option<(f32, f32, f32, f32)>,
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
            measure_color,
            beat_color,
            sub_beat_color,
            scroll_x_pixel,
        );
    }
}

/// Build data bar instances (layer 2).
/// Dependencies: scroll_x, pixels_per_tick, track_visible, track_colors
pub fn build_data_bars(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
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

    struct Bar<'a> {
        evt: &'a AutomationEvent,
        bar_x: f32,
        bar_h: f32,
        color: [f32; 3],
    }

    let mut bars: Vec<Bar> = Vec::new();

    for lane in lanes {
        let trk_idx = lane.track as usize;
        if !track_visible.get(trk_idx).copied().unwrap_or(true) {
            continue;
        }

        let color = track_colors
            .get(trk_idx)
            .copied()
            .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

        let events = lane.events_in_range(pad_start, pad_end);
        let mut last_tick = u32::MAX;
        let mut seen_values = [false; 128];

        for evt in events {
            if evt.tick != last_tick {
                last_tick = evt.tick;
                seen_values.fill(false);
            }

            let val_idx = evt.value as usize;
            if val_idx < 128 && seen_values[val_idx] {
                continue;
            }
            if val_idx < 128 {
                seen_values[val_idx] = true;
            }

            let val = evt.value as f32;
            let bar_h = ((val + 1.0) / (max_val + 1.0)) * h;
            let bar_x = x_offset + evt.tick as f32 * ppu;

            if bar_x + BAR_WIDTH < 0.0 || bar_x > w {
                continue;
            }

            bars.push(Bar { evt, bar_x, bar_h, color });
        }
    }

    bars.sort_by(|a, b| b.evt.value.cmp(&a.evt.value));

    for bar in &bars {
        out.push(DrawInstance {
            x: bar.bar_x,
            y: h - bar.bar_h,
            w: BAR_WIDTH,
            h: bar.bar_h,
            rgba_packed: pack_rgba(bar.color[0], bar.color[1], bar.color[2], 0.85),
            props_packed: pack_props(BAR_ROUNDING, BAR_BORDER),
            velocity: bar.evt.value as u32,
            tag: 0,
        });
    }
}

/// Build stepped-line instances for automation data (layer 2, replaces data bars).
///
/// Renders each event as a staircase: horizontal line (value held) + vertical
/// line (value change).  Optionally draws dots at event positions.
pub fn build_data_lines(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    show_dots: bool,
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
        let trk_idx = lane.track as usize;
        if !track_visible.get(trk_idx).copied().unwrap_or(true) {
            continue;
        }

        let color = track_colors
            .get(trk_idx)
            .copied()
            .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

        let all_events: Vec<(u32, u16)> = lane.events.iter().map(|e| (e.tick, e.value)).collect();
        let visible_events: Vec<(u32, u16)> = lane
            .events_in_range(pad_start, pad_end)
            .iter()
            .map(|e| (e.tick, e.value))
            .collect();

        if visible_events.is_empty() {
            // No visible events: draw a full-width horizontal line at chase value
            let idx = all_events.partition_point(|e| e.0 < pad_start);
            let val = if idx > 0 { all_events[idx - 1].1 } else { 0 };
            let y = h - (val as f32 / max_val) * h;
            if w > grid_left_x {
                out.push(DrawInstance {
                    x: grid_left_x,
                    y,
                    w: w - grid_left_x,
                    h: 1.0,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                    props_packed: pack_props(0.0, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }
            continue;
        }

        // Find the value before the first visible event
        let prev_idx = all_events.partition_point(|e| e.0 < visible_events[0].0);
        let mut prev_val = if prev_idx > 0 { all_events[prev_idx - 1].1 } else { 0 };
        let mut prev_tick = visible_events[0].0;

        // Horizontal line from grid left edge to the first event
        let first_x = x_offset + visible_events[0].0 as f32 * ppu;
        let first_y = h - (prev_val as f32 / max_val) * h;
        if first_x > grid_left_x {
            out.push(DrawInstance {
                x: grid_left_x,
                y: first_y,
                w: first_x - grid_left_x,
                h: 1.0,
                rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }

        for &(tick, value) in &visible_events {
            let x1 = x_offset + prev_tick as f32 * ppu;
            let x2 = x_offset + tick as f32 * ppu;
            let y1 = h - (prev_val as f32 / max_val) * h;
            let y2 = h - (value as f32 / max_val) * h;

            // Horizontal line: value held from prev_tick to tick
            if x2 > x1 {
                out.push(DrawInstance {
                    x: x1,
                    y: y1,
                    w: x2 - x1,
                    h: 1.0,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
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
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                    props_packed: pack_props(0.0, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }

            // Dot at event position
            if show_dots {
                out.push(DrawInstance {
                    x: x2 - 2.0,
                    y: y2 - 2.0,
                    w: 4.0,
                    h: 4.0,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                    props_packed: pack_props(2.0, 0.0),
                    velocity: 0,
                    tag: 0,
                });
            }

            prev_val = value;
            prev_tick = tick;
        }

        // Horizontal line from the last event to the next event (or right edge)
        let last_x = x_offset + prev_tick as f32 * ppu;
        let last_y = h - (prev_val as f32 / max_val) * h;
        let next_idx = all_events.partition_point(|e| e.0 <= prev_tick);
        let right_bound = if next_idx < all_events.len() {
            x_offset + all_events[next_idx].0 as f32 * ppu
        } else {
            w
        };
        if right_bound > last_x {
            out.push(DrawInstance {
                x: last_x,
                y: last_y,
                w: right_bound - last_x,
                h: 1.0,
                rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
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
        let notes = midi.key_notes_in_range(key, pad_start, pad_end);
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
