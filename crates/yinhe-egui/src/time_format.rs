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
pub fn format_tick_bar_beat(tick: f64, ppq: u32, numerator: u8) -> String {
    let ticks_per_bar = ppq as u32 * numerator as u32;
    let bar = (tick / ticks_per_bar as f64).floor() as u32 + 1;
    let beat = ((tick % ticks_per_bar as f64) / ppq as f64).floor() as u32 + 1;
    let tick_in_beat = (tick % ppq as f64) as u32;
    format!("{}.{}.{:03}", bar, beat, tick_in_beat)
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

