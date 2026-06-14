/// Tempo segment: a region of constant tempo.
#[derive(Clone, Debug)]
pub struct TempoSegment {
    /// Tick where this tempo starts.
    pub start_tick: u32,
    /// Time in seconds where this tempo starts.
    pub start_time: f64,
    /// Microseconds per quarter note (BPM = 60_000_000 / micros_per_quarter).
    pub micros_per_quarter: u64,
}

/// Default microseconds per quarter note (120 BPM).
pub const DEFAULT_MPQ: u64 = 500_000;

/// Fallback ticks-per-beat for timecode-based MIDI files.
pub const TIMECODE_FALLBACK_TPB: u32 = 480;

/// Default BPM when no tempo event is present.
pub const DEFAULT_BPM: f64 = 120.0;

/// Convert microseconds-per-quarter-note to BPM.
pub fn bpm_from_mpq(mpq: u64) -> f64 {
    if mpq == 0 {
        return DEFAULT_BPM;
    }
    60_000_000.0 / mpq as f64
}

/// Convert BPM to microseconds-per-quarter-note.
pub fn mpq_from_bpm(bpm: f32) -> u64 {
    if bpm <= 0.0 {
        return DEFAULT_MPQ;
    }
    (60_000_000.0 / bpm as f64).round() as u64
}

/// Convert a tick delta to seconds, given ticks-per-beat and microseconds-per-quarter.
pub fn ticks_to_seconds(dtick: u64, ticks_per_beat: u32, mpq: u64) -> f64 {
    if ticks_per_beat == 0 {
        return 0.0;
    }
    dtick as f64 * mpq as f64 / (ticks_per_beat as f64 * 1_000_000.0)
}

/// Convert seconds to ticks, given ticks-per-beat and microseconds-per-quarter.
pub fn seconds_to_ticks(seconds: f64, ticks_per_beat: u32, mpq: u64) -> f64 {
    if mpq == 0 {
        return 0.0;
    }
    seconds * ticks_per_beat as f64 * 1_000_000.0 / mpq as f64
}

/// Calculate bar divide (ticks per measure).
pub fn bar_divide(ticks_per_beat: u32, time_sig_numerator: u8, time_sig_denominator: u8) -> f64 {
    let num = time_sig_numerator as f64;
    let den = (1u32 << time_sig_denominator) as f64;
    ticks_per_beat as f64 * num / den * 4.0
}

/// Calculate bar number at a given tick (1-based).
pub fn bar_at_tick(tick: u64, bar_div: f64) -> u64 {
    if bar_div <= 0.0 {
        return 1;
    }
    (tick as f64 / bar_div).floor() as u64 + 1
}

/// Calculate total number of bars.
pub fn total_bars(tick_length: u64, bar_div: f64) -> u64 {
    if bar_div <= 0.0 {
        return 1;
    }
    (tick_length as f64 / bar_div).ceil() as u64
}

/// Recompute `start_time` for each tempo segment based on cumulative tick deltas.
///
/// Segments must already be sorted by `start_tick`. The first segment's
/// `start_time` is set to 0.0. Each subsequent segment's `start_time` is
/// computed by adding the duration of the previous segment using its mpq.
pub fn recompute_tempo_start_times(segments: &mut [TempoSegment], ticks_per_beat: u32) {
    if segments.is_empty() {
        return;
    }
    segments[0].start_time = 0.0;
    for i in 1..segments.len() {
        let prev = &segments[i - 1];
        let dtick = (segments[i].start_tick - prev.start_tick) as u64;
        let elapsed = ticks_to_seconds(dtick, ticks_per_beat, prev.micros_per_quarter);
        segments[i].start_time = prev.start_time + elapsed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ticks_to_seconds_and_back() {
        let dtick = 480;
        let tpb = 480;
        let mpq = DEFAULT_MPQ; // 120 BPM
        let secs = ticks_to_seconds(dtick, tpb, mpq);
        assert!((secs - 0.5).abs() < 1e-9);

        let ticks_back = seconds_to_ticks(secs, tpb, mpq);
        assert!((ticks_back - dtick as f64).abs() < 1e-9);
    }

    #[test]
    fn test_bpm_from_mpq() {
        assert!((bpm_from_mpq(DEFAULT_MPQ) - 120.0).abs() < 1e-3);
        assert!((bpm_from_mpq(250_000) - 240.0).abs() < 1e-3);
    }

    #[test]
    fn test_is_black_key() {
        use yinhe_types::is_black_key;
        assert!(!is_black_key(0)); // C
        assert!(is_black_key(1)); // C#
        assert!(is_black_key(3)); // D#
        assert!(!is_black_key(4)); // E
        assert!(!is_black_key(12)); // next C
    }

    #[test]
    fn test_mpq_from_bpm() {
        assert_eq!(mpq_from_bpm(120.0), DEFAULT_MPQ);
        assert_eq!(mpq_from_bpm(240.0), 250_000);
        assert_eq!(mpq_from_bpm(60.0), 1_000_000);
    }

    #[test]
    fn test_mpq_from_bpm_zero_or_negative() {
        assert_eq!(mpq_from_bpm(0.0), DEFAULT_MPQ);
        assert_eq!(mpq_from_bpm(-10.0), DEFAULT_MPQ);
    }

    #[test]
    fn test_bpm_from_mpq_zero() {
        assert!((bpm_from_mpq(0) - DEFAULT_BPM).abs() < 1e-3);
    }

    #[test]
    fn test_ticks_to_seconds_zero_tpb() {
        assert_eq!(ticks_to_seconds(480, 0, DEFAULT_MPQ), 0.0);
    }

    #[test]
    fn test_seconds_to_ticks_zero_mpq() {
        assert_eq!(seconds_to_ticks(1.0, 480, 0), 0.0);
    }

    #[test]
    fn test_bar_divide_4_4() {
        let div = bar_divide(480, 4, 2);
        assert!((div - 1920.0).abs() < 1e-9);
    }

    #[test]
    fn test_bar_divide_3_4() {
        let div = bar_divide(480, 3, 2);
        assert!((div - 1440.0).abs() < 1e-9);
    }

    #[test]
    fn test_bar_divide_6_8() {
        let div = bar_divide(480, 6, 3);
        assert!((div - 1440.0).abs() < 1e-9);
    }

    #[test]
    fn test_bar_at_tick_basic() {
        let div = 1920.0; // 4/4 at 480 tpb
        assert_eq!(bar_at_tick(0, div), 1);
        assert_eq!(bar_at_tick(1, div), 1);
        assert_eq!(bar_at_tick(1920, div), 2);
        assert_eq!(bar_at_tick(1919, div), 1);
        assert_eq!(bar_at_tick(1920 * 4, div), 5);
    }

    #[test]
    fn test_bar_at_tick_zero_div() {
        assert_eq!(bar_at_tick(100, 0.0), 1);
    }

    #[test]
    fn test_total_bars_basic() {
        let div = 1920.0;
        assert_eq!(total_bars(0, div), 0);
        assert_eq!(total_bars(1, div), 1);
        assert_eq!(total_bars(1920, div), 1);
        assert_eq!(total_bars(1921, div), 2);
        assert_eq!(total_bars(1920 * 4, div), 4);
    }

    #[test]
    fn test_total_bars_zero_div() {
        assert_eq!(total_bars(100, 0.0), 1);
    }

    #[test]
    fn test_recompute_tempo_start_times_empty() {
        let mut segments: Vec<TempoSegment> = vec![];
        recompute_tempo_start_times(&mut segments, 480);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_recompute_tempo_start_times_single() {
        let mut segments = vec![TempoSegment {
            start_tick: 0,
            start_time: 999.0,
            micros_per_quarter: DEFAULT_MPQ,
        }];
        recompute_tempo_start_times(&mut segments, 480);
        assert!((segments[0].start_time - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_recompute_tempo_start_times_two_segments() {
        let mut segments = vec![
            TempoSegment {
                start_tick: 0,
                start_time: 999.0,
                micros_per_quarter: DEFAULT_MPQ,
            },
            TempoSegment {
                start_tick: 480,
                start_time: 999.0,
                micros_per_quarter: 250_000,
            },
        ];
        recompute_tempo_start_times(&mut segments, 480);
        assert!((segments[0].start_time - 0.0).abs() < 1e-9);
        assert!((segments[1].start_time - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_ticks_to_seconds_roundtrip() {
        let tpb = 480;
        let mpq = DEFAULT_MPQ;
        let ticks = [0, 1, 480, 960, 100000];
        for &t in &ticks {
            let secs = ticks_to_seconds(t, tpb, mpq);
            let back = seconds_to_ticks(secs, tpb, mpq);
            assert!((back - t as f64).abs() < 1e-6, "roundtrip failed for tick={}", t);
        }
    }

    #[test]
    fn test_roundtrip_bpm_mpq() {
        let bpms = [30.0, 60.0, 120.0, 140.0, 200.0, 240.0];
        for &bpm in &bpms {
            let mpq = mpq_from_bpm(bpm as f32);
            let back = bpm_from_mpq(mpq);
            assert!((back - bpm).abs() < 0.1, "roundtrip failed for bpm={}", bpm);
        }
    }
}
