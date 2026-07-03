/// Shared grid-building utilities used by both pianoroll and arrangement instances.
use yinhe_types::{build_time_sig_segments, measure_ticks, TimeSigEvent, TimelineViewBase};

use crate::vertex::{DrawInstance, pack_props, pack_rgba};

// ── Pianoroll grid colors ──
pub const PR_BG_COLOR: (f32, f32, f32) = (0.12, 0.12, 0.14);
pub const PR_MEASURE_LINE_COLOR: (f32, f32, f32, f32) = (0.35, 0.35, 0.40, 1.0);
pub const PR_BEAT_LINE_COLOR: (f32, f32, f32, f32) = (0.22, 0.22, 0.25, 1.0);
pub const PR_SUB_BEAT_LINE_COLOR: (f32, f32, f32, f32) = (0.16, 0.16, 0.18, 1.0);

// ── Arrangement grid colors ──
pub const AR_BG_COLOR: (f32, f32, f32) = (0.14, 0.14, 0.16);
pub const AR_LANE_EVEN_COLOR: (f32, f32, f32) = (0.16, 0.16, 0.18);
pub const AR_LANE_ODD_COLOR: (f32, f32, f32) = (0.13, 0.13, 0.15);
pub const AR_MEASURE_LINE_COLOR: (f32, f32, f32, f32) = (0.30, 0.30, 0.35, 1.0);
pub const AR_BEAT_LINE_COLOR: (f32, f32, f32, f32) = (0.20, 0.20, 0.23, 1.0);
pub const AR_PLAYHEAD_COLOR: (f32, f32, f32, f32) = (1.0, 1.0, 1.0, 0.8);

/// Given a tick and time signature info, return the previous and next
/// bar-line positions.  Respects time-signature changes.
pub fn measure_bounds_at_tick(
    tick: f64,
    ticks_per_beat: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
) -> (f64, f64) {
    let tick_u = tick.max(0.0) as u32;
    let segments = build_time_sig_segments(time_sig_events, default_num, default_den);

    let seg_idx = segments
        .partition_point(|&(start, _, _)| start <= tick_u)
        .saturating_sub(1);
    let (seg_start, num, den) = segments[seg_idx];
    let seg_end = segments
        .get(seg_idx + 1)
        .map_or(u32::MAX, |&(end, _, _)| end);

    let measure = measure_ticks(ticks_per_beat, num, den);
    let offset = tick_u.saturating_sub(seg_start);
    let bars_past = offset / measure;
    let prev_bar = seg_start + bars_past * measure;
    let next_bar = (prev_bar + measure).min(seg_end);

    (prev_bar as f64, next_bar as f64)
}

/// Build timeline grid lines shared by pianoroll and arrangement views.
///
/// `sub_beat_color`: if Some, render sub-beat lines when `pixels_per_sub >= 2.0`.
/// `scroll_x_pixel`: the integer-scrolled scroll_x used for pixel positions.
///   This should be `floor(scroll_x)` so grid lines are stable across frames;
///   the fractional part is applied as a uniform NDC offset in the shader.
pub fn build_timeline_grid(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    base: &TimelineViewBase,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    measure_color: (f32, f32, f32, f32),
    beat_color: (f32, f32, f32, f32),
    sub_beat_color: Option<(f32, f32, f32, f32)>,
    scroll_x_pixel: f32,
) {
    let ppu = base.pixels_per_tick;
    if ppu <= 0.001 {
        return;
    }
    let (tick_start, tick_end) = base.visible_tick_range(w);
    let left_w = base.left_panel_width;
    let x_origin = left_w - scroll_x_pixel;

    let sub_beat_div = 4u32;
    let ticks_per_sub = (tpb / sub_beat_div).max(1);
    let segments = build_time_sig_segments(time_sig_events, default_num, default_den);
    let sub_f = ticks_per_sub as f64;

    for i in 0..segments.len() {
        let (seg_start, num, den) = segments[i];
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
        let seg_start_f = seg_start as f64;
        if seg_start_f > tick_end {
            break;
        }

        let ticks_per_measure = measure_ticks(tpb, num, den);
        let ticks_per_beat = ticks_per_measure / num as u32;

        let pixels_per_sub = ticks_per_sub as f32 * ppu;
        let show_sub_beat = sub_beat_color.is_some() && pixels_per_sub >= 2.0;
        let show_beat = pixels_per_sub >= 1.0;

        let first_tick = seg_start_f.max(tick_start);
        let first = ((first_tick / sub_f).floor() as u32)
            .saturating_mul(ticks_per_sub)
            .max(seg_start);

        let mut tick = first;
        while (tick as f64) <= tick_end && tick < seg_end {
            let local = tick - seg_start;

            let x = x_origin + tick as f32 * ppu;
            if x >= left_w && x <= w {
                let is_measure = local % ticks_per_measure == 0;
                let is_beat = if !is_measure {
                    let beat_local = local % ticks_per_measure;
                    beat_local.is_multiple_of(ticks_per_beat) && beat_local > 0
                } else {
                    false
                };
                if is_measure {
                    push_grid_line(out, x, h, 2.0, measure_color, tick);
                } else if is_beat && show_beat {
                    push_grid_line(out, x, h, 1.0, beat_color, tick);
                } else if show_sub_beat {
                    push_grid_line(out, x, h, 1.0, sub_beat_color.unwrap(), tick);
                }
            }
            tick += ticks_per_sub;
        }
    }
}

/// Push a grid line instance into `out`.
pub fn push_grid_line(
    out: &mut Vec<DrawInstance>,
    x: f32,
    h: f32,
    line_width: f32,
    color: (f32, f32, f32, f32),
    tick: u32,
) {
    out.push(DrawInstance {
        x: x - line_width / 2.0, // centre the line on the tick position
        y: 0.0,
        w: line_width,
        h,
        rgba_packed: pack_rgba(color.0, color.1, color.2, color.3),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: tick,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{TimeSigEvent, TimelineViewBase};

    #[test]
    fn test_measure_ticks_4_4() {
        // 4/4 at 480 tpb: 4 beats = 1920 ticks
        assert_eq!(measure_ticks(480, 4, 2), 1920);
    }

    #[test]
    fn test_measure_ticks_3_4() {
        // 3/4 at 480 tpb: 3 beats = 1440 ticks
        assert_eq!(measure_ticks(480, 3, 2), 1440);
    }

    #[test]
    fn test_measure_ticks_6_8() {
        // 6/8 at 480 tpb: 6/8 * 4 = 3 beats = 1440 ticks
        assert_eq!(measure_ticks(480, 6, 3), 1440);
    }

    #[test]
    fn test_measure_ticks_zero_numerator_fallback() {
        // numerator=0 → fallback to 4/4
        assert_eq!(measure_ticks(480, 0, 2), 1920);
    }

    #[test]
    fn test_measure_ticks_min_1() {
        // Very small tpb should still return at least 1
        assert_eq!(measure_ticks(1, 1, 4), 1);
    }

    #[test]
    fn test_build_time_sig_segments_no_events() {
        let segs = build_time_sig_segments(&[], 4, 2);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], (0, 4, 2));
    }

    #[test]
    fn test_build_time_sig_segments_with_change() {
        let events = vec![
            TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            },
            TimeSigEvent {
                tick: 1920,
                numerator: 3,
                denominator: 2,
            },
        ];
        let segs = build_time_sig_segments(&events, 4, 2);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], (0, 4, 2));
        assert_eq!(segs[1], (1920, 3, 2));
    }

    #[test]
    fn test_push_grid_line_creates_instance() {
        let mut out = Vec::new();
        push_grid_line(&mut out, 100.0, 500.0, 1.0, (0.5, 0.5, 0.5, 1.0), 42);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].x, 99.5); // centered: 100 - 1.0/2
        assert_eq!(out[0].h, 500.0);
        assert_eq!(out[0].w, 1.0);
        assert_eq!(out[0].tag, 42);
    }

    #[test]
    fn test_measure_bounds_at_tick_4_4() {
        let (prev, next) = measure_bounds_at_tick(0.0, 480, 4, 2, &[]);
        assert!((prev - 0.0).abs() < 1.0);
        assert!((next - 1920.0).abs() < 1.0);
    }

    #[test]
    fn test_measure_bounds_at_tick_mid_bar() {
        let (prev, next) = measure_bounds_at_tick(1000.0, 480, 4, 2, &[]);
        assert!((prev - 0.0).abs() < 1.0);
        assert!((next - 1920.0).abs() < 1.0);
    }

    #[test]
    fn test_measure_bounds_at_tick_second_bar() {
        let (prev, next) = measure_bounds_at_tick(2000.0, 480, 4, 2, &[]);
        assert!((prev - 1920.0).abs() < 1.0);
        assert!((next - 3840.0).abs() < 1.0);
    }

    #[test]
    fn test_measure_bounds_at_tick_with_time_sig_change() {
        let events = vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ];
        let (prev, next) = measure_bounds_at_tick(2000.0, 480, 4, 2, &events);
        // After tick 1920, time sig is 3/4 → measure = 1440 ticks
        assert!((prev - 1920.0).abs() < 1.0);
        assert!((next - 3360.0).abs() < 1.0);
    }

    #[test]
    fn test_measure_bounds_at_tick_negative() {
        let (prev, next) = measure_bounds_at_tick(-10.0, 480, 4, 2, &[]);
        assert!((prev - 0.0).abs() < 1.0);
        assert!((next - 1920.0).abs() < 1.0);
    }

    #[test]
    fn test_build_timeline_grid_basic() {
        let mut out = Vec::new();
        let base = TimelineViewBase {
            pixels_per_tick: 0.1,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 60.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
        };
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &[],
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), 0.0);
        assert!(!out.is_empty(), "grid should produce lines");
        for inst in &out {
            assert!(inst.x >= 0.0, "line should be within viewport");
            assert!(inst.x <= 800.0, "line should be within viewport");
            assert_eq!(inst.h, 500.0);
        }
    }

    #[test]
    fn test_build_timeline_grid_zero_ppu() {
        let mut out = Vec::new();
        let base = TimelineViewBase {
            pixels_per_tick: 0.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 60.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
        };
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &[],
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), 0.0);
        assert!(out.is_empty(), "no grid lines when ppu is 0");
    }

    #[test]
    fn test_build_timeline_grid_with_time_sig_change() {
        let mut out = Vec::new();
        let base = TimelineViewBase {
            pixels_per_tick: 0.1,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 60.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
        };
        let events = vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ];
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &events,
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), 0.0);
        assert!(!out.is_empty());
    }

    #[test]
    fn test_build_timeline_grid_no_sub_beats() {
        let mut out = Vec::new();
        let base = TimelineViewBase {
            pixels_per_tick: 0.1,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 60.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
        };
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &[],
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), None, 0.0);
        assert!(!out.is_empty());
    }
}
