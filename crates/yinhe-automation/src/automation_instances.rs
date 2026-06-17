use yinhe_types::{AutomationEvent, AutomationLane, NoteSource, TRACK_PALETTE, TimeSigEvent};

use crate::AutomationPanelView;
use crate::grid;
use crate::{NoteInstance, pack_props, pack_rgba};

/// Bar width in pixels for automation events.
const BAR_WIDTH: f32 = 2.0;
/// Corner radius for automation bars (flat).
const BAR_ROUNDING: f32 = 0.0;
/// Border width for automation bars.
const BAR_BORDER: f32 = 0.0;
/// Color for the center/default reference line.
const CENTER_LINE_COLOR: (f32, f32, f32) = (0.30, 0.30, 0.35);
/// Center line alpha.
const CENTER_LINE_ALPHA: f32 = 0.6;

/// Build background + center line instances (layer 0).
/// Dependencies: none (background is static), lane target (center line)
pub fn build_decor(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    lane: Option<&AutomationLane>,
) {
    out.push(NoteInstance {
        x: 0.0,
        y: 0.0,
        w,
        h,
        rgba_packed: pack_rgba(
            grid::PR_BG_COLOR.0,
            grid::PR_BG_COLOR.1,
            grid::PR_BG_COLOR.2,
            1.0,
        ),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: 0,
    });

    if let Some(lane) = lane {
        let target = &lane.target;
        let max_val = target.max_value() as f32;
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value() as f32;
            let y_center = h - (center_val / max_val) * h;
            out.push(NoteInstance {
                x: 0.0,
                y: y_center - 0.5,
                w,
                h: 1.0,
                rgba_packed: pack_rgba(
                    CENTER_LINE_COLOR.0,
                    CENTER_LINE_COLOR.1,
                    CENTER_LINE_COLOR.2,
                    CENTER_LINE_ALPHA,
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
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    scroll_x_pixel: f32,
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
            grid::PR_MEASURE_LINE_COLOR,
            grid::PR_BEAT_LINE_COLOR,
            Some(grid::PR_SUB_BEAT_LINE_COLOR),
            scroll_x_pixel,
        );
    }
}

/// Build data bar instances (layer 2).
/// Dependencies: scroll_x, pixels_per_tick, track_visible, track_colors
pub fn build_data_bars(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    if let Some(lane) = lane {
        let target = &lane.target;
        let max_val = target.max_value() as f32;
        if max_val <= 0.0 {
            return;
        }

        let ppu = view.base.pixels_per_tick;
        let (tick_start, tick_end) = view.base.visible_tick_range(w);
        let pad_start = tick_start.max(0.0) as u32;
        let pad_end = tick_end.max(0.0) as u32;

        let events = lane.events_in_range(pad_start, pad_end);
        let x_offset = view.base.left_panel_width - view.base.scroll_x;

        struct Bar<'a> {
            evt: &'a AutomationEvent,
            bar_x: f32,
            bar_h: f32,
            color: [f32; 3],
        }

        let mut bars: Vec<Bar> = Vec::new();
        let mut last_tick = u32::MAX;
        let mut seen_values = [false; 128];

        for evt in events {
            let trk_idx = evt.track as usize;
            if !track_visible.get(trk_idx).copied().unwrap_or(true) {
                continue;
            }

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

            let trk = evt.track as usize;
            let color = track_colors
                .get(trk)
                .copied()
                .unwrap_or_else(|| TRACK_PALETTE[trk % TRACK_PALETTE.len()]);

            bars.push(Bar {
                evt,
                bar_x,
                bar_h,
                color,
            });
        }

        bars.sort_by(|a, b| b.evt.value.cmp(&a.evt.value));

        for bar in &bars {
            out.push(NoteInstance {
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
}

/// Build stepped-line instances for automation data (layer 2, replaces data bars).
///
/// Renders each event as a staircase: horizontal line (value held) + vertical
/// line (value change).  Optionally draws dots at event positions.
pub fn build_data_lines(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    show_dots: bool,
) {
    let Some(lane) = lane else { return };
    let target = &lane.target;
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

    // Group all events by track so per-track operations don't leak across tracks.
    let mut track_all_events: Vec<Vec<(u32, u16)>> = vec![Vec::new(); track_visible.len()];
    for evt in lane.events.iter() {
        let trk = evt.track as usize;
        if trk >= track_all_events.len() || !track_visible.get(trk).copied().unwrap_or(true) {
            continue;
        }
        track_all_events[trk].push((evt.tick, evt.value));
    }

    // Collect visible events (within pad_start..pad_end) grouped by track
    let mut track_visible_events: Vec<Vec<(u32, u16)>> = vec![Vec::new(); track_visible.len()];
    for evt in lane.events_in_range(pad_start, pad_end) {
        let trk = evt.track as usize;
        if trk >= track_visible_events.len() || !track_visible.get(trk).copied().unwrap_or(true) {
            continue;
        }
        track_visible_events[trk].push((evt.tick, evt.value));
    }

    for (ti, events) in track_visible_events.iter().enumerate() {
        if events.is_empty() {
            continue;
        }
        let color = track_colors
            .get(ti)
            .copied()
            .unwrap_or_else(|| TRACK_PALETTE[ti % TRACK_PALETTE.len()]);

        let all = &track_all_events[ti];

        // Find the value before the first visible event (same track only)
        let prev_idx = all.partition_point(|e| e.0 < events[0].0);
        let mut prev_val = if prev_idx > 0 { all[prev_idx - 1].1 } else { 0 };
        let mut prev_tick = events[0].0;

        // Horizontal line from grid left edge to the first event
        let first_x = x_offset + events[0].0 as f32 * ppu;
        let first_y = h - (prev_val as f32 / max_val) * h;
        if first_x > grid_left_x {
            out.push(NoteInstance {
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

        for &(tick, value) in events {
            let x1 = x_offset + prev_tick as f32 * ppu;
            let x2 = x_offset + tick as f32 * ppu;
            let y1 = h - (prev_val as f32 / max_val) * h;
            let y2 = h - (value as f32 / max_val) * h;

            // Horizontal line: value held from prev_tick to tick
            if x2 > x1 {
                out.push(NoteInstance {
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
                out.push(NoteInstance {
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
                out.push(NoteInstance {
                    x: x2 - 2.0,
                    y: y2 - 2.0,
                    w: 4.0,
                    h: 4.0,
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                    props_packed: pack_props(2.0, 0.0), // rounded dot
                    velocity: 0,
                    tag: 0,
                });
            }

            prev_val = value;
            prev_tick = tick;
        }

        // Horizontal line from the last event to the next event (same track only)
        let last_x = x_offset + prev_tick as f32 * ppu;
        let last_y = h - (prev_val as f32 / max_val) * h;
        let next_idx = all.partition_point(|e| e.0 <= prev_tick);
        let right_bound = if next_idx < all.len() {
            x_offset + all[next_idx].0 as f32 * ppu
        } else {
            w
        };
        if right_bound > last_x {
            out.push(NoteInstance {
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

    // Handle tracks with no visible events: chase the value at pad_start and
    // draw a full-width horizontal line so the automation doesn't disappear
    // when the viewport contains no event points.
    for (ti, events) in track_visible_events.iter().enumerate() {
        if !events.is_empty() {
            continue;
        }
        if !track_visible.get(ti).copied().unwrap_or(true) {
            continue;
        }
        let all = &track_all_events[ti];
        let idx = all.partition_point(|e| e.0 < pad_start);
        let val = if idx > 0 { all[idx - 1].1 } else { 0 };
        let color = track_colors
            .get(ti)
            .copied()
            .unwrap_or_else(|| TRACK_PALETTE[ti % TRACK_PALETTE.len()]);
        let y = h - (val as f32 / max_val) * h;
        if w > grid_left_x {
            out.push(NoteInstance {
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
    }
}

/// Build velocity bars from NoteSource (layer 2, replaces data bars for Velocity).
///
/// `display_mode`: 0=柱状(2px竖条), 1=矩形(填充), 2=空心矩形(边框)
pub fn build_velocity_bars(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &AutomationPanelView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    display_mode: u32,
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
        out.push(NoteInstance {
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

/// Build all instances for one automation panel (backward-compatible).
pub fn build_automation_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    let w = width as f32;
    let h = height as f32;

    build_decor(instances, w, h, lane);
    build_grid(instances, w, h, view, tpb, default_num, default_den, time_sig_events, 0.0);
    build_data_bars(instances, w, h, view, lane, track_visible, track_colors);
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{
        AutomationEvent, AutomationLane, AutomationTarget, TimeSigEvent, TimelineViewBase,
    };

    fn make_view(pixels_per_tick: f32, scroll_x: f32, panel_height: f32) -> AutomationPanelView {
        AutomationPanelView {
            base: TimelineViewBase {
                pixels_per_tick,
                scroll_x,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            panel_height,
            selected_target: AutomationTarget::CC { controller: 7 },
            lane_index: 0,
            dirty: true,
        }
    }

    fn make_lane(target: AutomationTarget, ticks: &[(u32, u16)]) -> AutomationLane {
        AutomationLane {
            target,
            events: ticks
                .iter()
                .map(|&(tick, value)| AutomationEvent {
                    tick,
                    value,
                    channel: 0,
                    track: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn test_background_always_first() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);

        build_automation_instances(&mut instances, 800, 100, &view, None, None, 4, 2, &[], &[], &[]);

        assert!(!instances.is_empty());
        let bg = &instances[0];
        assert_eq!(bg.x, 0.0);
        assert_eq!(bg.y, 0.0);
        assert_eq!(bg.w, 800.0);
        assert_eq!(bg.h, 100.0);
    }

    #[test]
    fn test_no_grid_when_tpb_is_none() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);

        build_automation_instances(&mut instances, 800, 100, &view, None, None, 4, 2, &[], &[], &[]);

        assert_eq!(instances.len(), 1, "no TPB means no grid lines");
    }

    #[test]
    fn test_grid_lines_when_tpb_provided() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);

        build_automation_instances(&mut instances, 800, 100, &view, None, Some(480), 4, 2, &[], &[], &[]);

        assert!(instances.len() > 1);
        let grid_line = &instances[1];
        assert!(grid_line.x >= 0.0);
        assert!(grid_line.w < 5.0, "grid lines should be thin");
    }

    #[test]
    fn test_center_line_appears_for_pitch_bend() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::PitchBend,
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        let center = &instances[1];
        let expected_center_y = 100.0 - (8192.0_f32 / 16383.0_f32) * 100.0 - 0.5;
        assert!((center.y - expected_center_y).abs() < 0.001);
        assert_eq!(center.w, 800.0);
        assert_eq!(center.h, 1.0);
    }

    #[test]
    fn test_center_line_appears_for_fine_tune() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::FineTune,
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        let center = &instances[1];
        assert!((center.y - (100.0 - 50.0 - 0.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_no_center_line_for_velocity() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::Velocity,
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn test_no_center_line_for_cc() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn test_data_bars_positioned_correctly() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[(100, 64), (200, 127)],
        );

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 3);

        let bar0 = &instances[1];
        assert!((bar0.x - 260.0).abs() < 0.001);
        assert!((bar0.y - 0.0).abs() < 0.001);
        assert_eq!(bar0.w, 2.0);

        let bar1 = &instances[2];
        assert!((bar1.x - 160.0).abs() < 0.001);
        let expected_y1 = 100.0 - ((64.0 + 1.0) / (127.0 + 1.0)) * 100.0;
        assert!((bar1.y - expected_y1).abs() < 0.001);
        assert_eq!(bar1.w, 2.0);
    }

    #[test]
    fn test_off_screen_events_are_skipped() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 1000.0, 100.0);
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[(100, 64), (200, 127)],
        );

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1, "off-screen events should be skipped");
    }

    #[test]
    fn test_scrolled_events_appear_at_correct_x() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 50.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(100, 64)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        let bar = &instances[1];
        assert!((bar.x - 110.0).abs() < 0.001);
    }

    #[test]
    fn test_data_bar_height_scales_with_value() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 200.0);
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[(100, 0), (200, 64), (300, 127)],
        );

        build_automation_instances(
            &mut instances,
            800,
            200,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 4);

        assert!((instances[1].h - 200.0).abs() < 0.001);
        assert!((instances[1].y - 0.0).abs() < 0.001);

        assert!((instances[2].h - 101.5625).abs() < 0.001);
        assert!((instances[2].y - 98.4375).abs() < 0.001);

        assert!((instances[3].h - 1.5625).abs() < 0.001);
        assert!((instances[3].y - 198.4375).abs() < 0.001);
    }

    #[test]
    fn test_track_colors_from_palette() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![
                AutomationEvent {
                    tick: 100,
                    value: 64,
                    channel: 0,
                    track: 0,
                },
                AutomationEvent {
                    tick: 200,
                    value: 64,
                    channel: 0,
                    track: 1,
                },
            ],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 3);
        assert_ne!(instances[1].rgba_packed, instances[2].rgba_packed);
        assert_eq!(instances[1].velocity, 64);
        assert_eq!(instances[2].velocity, 64);
    }

    #[test]
    fn test_empty_lane_produces_no_data_bars() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn test_zero_dimensions_produces_valid_instances() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 0.0);

        build_automation_instances(&mut instances, 0, 0, &view, None, None, 4, 2, &[], &[], &[]);

        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].w, 0.0);
        assert_eq!(instances[0].h, 0.0);
    }

    #[test]
    fn test_max_value_bar_full_height() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(AutomationTarget::PitchBend, &[(100, 16383)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 3);
        let bar = &instances[2];
        assert!((bar.y - 0.0).abs() < 0.001);
        assert!((bar.h - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_event_partially_off_screen_left_edge() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(0, 100)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        assert!((instances[1].x - 60.0).abs() < 0.001);
    }

    #[test]
    fn test_negative_bar_x_skipped() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 100.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(10, 100)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1, "bar with negative x should be skipped");
    }

    #[test]
    fn test_bar_beyond_right_edge_skipped() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(1000, 100)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(
            instances.len(),
            1,
            "bar beyond right edge should be skipped"
        );
    }

    #[test]
    fn test_grid_with_time_signature_events() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);
        let ts_events = [TimeSigEvent {
            tick: 0,
            numerator: 6,
            denominator: 2,
        }];

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            None,
            Some(480),
            4,
            2,
            &ts_events, &[], &[]);

        assert!(instances.len() > 1);
    }

    #[test]
    fn test_track_visibility_filters_bars() {
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![
                yinhe_types::AutomationEvent {
                    tick: 100,
                    value: 64,
                    channel: 0,
                    track: 0,
                },
                yinhe_types::AutomationEvent {
                    tick: 200,
                    value: 80,
                    channel: 1,
                    track: 1,
                },
            ],
        };

        let mut both = Vec::new();
        build_automation_instances(
            &mut both,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[],
            &[true, true],
            &[],
        );

        let mut only0 = Vec::new();
        build_automation_instances(
            &mut only0,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[],
            &[true, false],
            &[],
        );

        assert_eq!(both.len(), only0.len() + 1);
    }
}
