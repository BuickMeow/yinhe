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
}
