use crate::note::TimeSigEvent;

/// Format seconds as `mm:ss.ms` (e.g. 0:00.000).
pub fn format_time(seconds: f64) -> String {
    let mins = (seconds / 60.0) as u32;
    let secs = (seconds % 60.0) as u32;
    let ms = ((seconds % 1.0) * 1000.0) as u32;
    format!("{}:{:02}.{:03}", mins, secs, ms)
}

/// Format BPM with two decimal places (e.g. 120.00).
pub fn format_bpm(bpm: f32) -> String {
    format!("{:.2}", bpm)
}

/// Format time signature from numerator / denominator power.
/// `denominator` is the power of 2 (e.g. 2 means 2^2 = 4).
pub fn format_time_sig(numerator: u8, denominator_power: u8) -> String {
    let denom = 2u32.pow(denominator_power as u32);
    format!("{}/{}", numerator, denom)
}

/// Convert tick to `bar.beat.tick_in_beat` format (all 1-indexed).
///
/// NOTE: This function assumes a single uniform time signature throughout the
/// entire project.  Use [`format_tick_bar_beat_with_time_sig`] when there are
/// time signature changes.
pub fn format_tick_bar_beat(tick: f64, ppq: u32, numerator: u8) -> String {
    let ticks_per_bar = ppq * numerator as u32;
    let bar = (tick / ticks_per_bar as f64).floor() as u32 + 1;
    let beat = ((tick % ticks_per_bar as f64) / ppq as f64).floor() as u32 + 1;
    let tick_in_beat = (tick % ppq as f64) as u32;
    format!("{}.{}.{:03}", bar, beat, tick_in_beat)
}

/// Convert tick to `bar.beat.tick_in_beat` format, correctly accounting for
/// variable (changing) time signatures by segmenting the timeline.
///
/// Uses the same segment+build logic as the time ruler so the two displays
/// always agree.
pub fn format_tick_bar_beat_with_time_sig(
    tick: f64,
    ppq: u32,
    time_sig_events: &[TimeSigEvent],
    default_num: u8,
    default_den: u8,
) -> String {
    // ── 1. Build time-signature segments (sorted by start tick) ──
    let segments = build_time_sig_segments(time_sig_events, default_num, default_den);

    // ── 2. Compute cumulative bar offsets ──
    let bar_offsets = compute_bar_offsets(ppq, &segments);

    // ── 3. Find which segment tick falls in ──
    let tick_u32 = tick as u32;
    let seg_idx = segments
        .partition_point(|&(start, _, _)| start <= tick_u32)
        .saturating_sub(1);
    let (seg_start, num, den) = segments[seg_idx];
    let local = tick_u32 - seg_start;

    // ── 4. Compute bar / beat / tick_in_beat within the segment ──
    let ticks_per_measure = measure_ticks(ppq, num, den);
    let ticks_per_beat = (ticks_per_measure / num as u32).max(1);
    let bar = bar_offsets[seg_idx] + (local / ticks_per_measure) + 1;
    let beat = ((local % ticks_per_measure) / ticks_per_beat) + 1;
    let tick_in_beat = tick as u32 % ppq;

    format!("{}.{}.{:03}", bar, beat, tick_in_beat)
}

/// Compute ticks per measure from time signature.
///
/// `denominator_power` is the power-of-2 encoding: 2 = quarter note (4), 3 = eighth (8), etc.
pub fn measure_ticks(tpb: u32, numerator: u8, denominator_power: u8) -> u32 {
    if numerator == 0 {
        return (tpb * 4).max(1);
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

/// Compute the cumulative number of complete bars before each segment.
fn compute_bar_offsets(tpb: u32, segments: &[(u32, u8, u8)]) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(segments.len());
    let mut acc = 0u32;
    for i in 0..segments.len() {
        offsets.push(acc);
        if i + 1 < segments.len() {
            let (start, num, den) = segments[i];
            let end = segments[i + 1].0;
            let tm = measure_ticks(tpb, num, den);
            if tm > 0 && end > start {
                acc += (end - start) / tm;
            }
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_time_zero() {
        assert_eq!(format_time(0.0), "0:00.000");
    }

    #[test]
    fn test_format_time_seconds() {
        assert_eq!(format_time(65.123), "1:05.123");
    }

    #[test]
    fn test_format_bpm() {
        assert_eq!(format_bpm(120.0), "120.00");
        assert_eq!(format_bpm(140.5), "140.50");
    }

    #[test]
    fn test_format_time_sig_4_4() {
        assert_eq!(format_time_sig(4, 2), "4/4");
    }

    #[test]
    fn test_format_time_sig_6_8() {
        assert_eq!(format_time_sig(6, 3), "6/8");
    }

    #[test]
    fn test_format_tick_bar_beat_start() {
        // tick=0, ppq=480, num=4 → 1.1.000
        assert_eq!(format_tick_bar_beat(0.0, 480, 4), "1.1.000");
    }

    #[test]
    fn test_format_tick_bar_beat_second_beat() {
        // tick=480, ppq=480, num=4 → beat 2 of bar 1
        assert_eq!(format_tick_bar_beat(480.0, 480, 4), "1.2.000");
    }

    #[test]
    fn test_format_tick_bar_beat_second_bar() {
        // tick=1920 (480*4), ppq=480, num=4 → bar 2
        assert_eq!(format_tick_bar_beat(1920.0, 480, 4), "2.1.000");
    }
}
