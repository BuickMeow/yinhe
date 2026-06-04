/// Shared grid-building utilities used by both pianoroll and arrangement instances.
use yinhe_types::TimeSigEvent;

use crate::vertex::{NoteInstance, pack_props, pack_rgba};

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

/// Compute ticks per measure from time signature.
///
/// `denominator_power` is the power-of-2 encoding: 2 = quarter note (4), 3 = eighth (8), etc.
pub fn measure_ticks(tpb: u32, numerator: u8, denominator_power: u8) -> u32 {
    if numerator == 0 {
        return (tpb * 4).max(1); // fallback 4/4
    }
    let num = numerator as f64;
    let den = (1u32 << denominator_power) as f64;
    ((tpb as f64 * num / den * 4.0).round() as u32).max(1)
}

/// Build sorted time-signature segments starting from tick 0.
///
/// Returns `Vec<(start_tick, numerator, denominator_power)>` sorted by tick.
/// Always includes a segment starting at tick 0 (using defaults if needed).
pub fn build_time_sig_segments(
    time_sig_events: &[TimeSigEvent],
    default_num: u8,
    default_den: u8,
) -> Vec<(u32, u8, u8)> {
    let mut segments: Vec<(u32, u8, u8)> = Vec::new();
    let mut prev_tick = 0u32;
    let mut prev_num = default_num;
    let mut prev_den = default_den;
    for ev in time_sig_events {
        if ev.tick > prev_tick {
            segments.push((prev_tick, prev_num, prev_den));
        }
        prev_tick = ev.tick;
        prev_num = ev.numerator;
        prev_den = ev.denominator;
    }
    segments.push((prev_tick, prev_num, prev_den));
    segments
}

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
        .iter()
        .rposition(|&(start, _, _)| start <= tick_u)
        .unwrap_or(0);
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

/// Push a grid line instance into `out`.
pub fn push_grid_line(
    out: &mut Vec<NoteInstance>,
    x: f32,
    h: f32,
    line_width: f32,
    color: (f32, f32, f32, f32),
    tick: u32,
) {
    out.push(NoteInstance {
        x,
        y: 0.0,
        w: line_width,
        h,
        rgba_packed: pack_rgba(color.0, color.1, color.2, color.3),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        flags: tick,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::TimeSigEvent;

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
        assert_eq!(out[0].x, 100.0);
        assert_eq!(out[0].h, 500.0);
        assert_eq!(out[0].w, 1.0);
        assert_eq!(out[0].flags, 42);
    }
}
