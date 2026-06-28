/// Quantization preset: snap notes to a regular grid.
///
/// Named variants are formatted as `1/N` where N is the number of
/// grid divisions per whole note.  `Custom` stores an arbitrary
/// fraction `numerator / denominator`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum QuantizePreset {
    Whole,           // 1/1
    Half,            // 1/2
    #[default]
    Quarter,         // 1/4
    Eighth,          // 1/8
    Sixteenth,       // 1/16
    ThirtySec,       // 1/32
    SixtyFourth,     // 1/64
    OneTwentyEighth, // 1/128

    // ── Triplets (also 1/N, where N is divisions-per-whole-note) ──
    QuarterTriplet,   // 1/6  (was 1/4T)
    EighthTriplet,    // 1/12 (was 1/8T)
    SixteenthTriplet, // 1/24 (was 1/16T)
    ThirtySecTriplet, // 1/48 (was 1/32T)

    /// User-defined fraction: `numerator / denominator`.
    /// Tick interval = PPQ × 4 × numerator / denominator.
    Custom(u32, u32),
}


impl QuantizePreset {
    /// Standard named presets (excluding `Custom`) in display order.
    pub const ALL: &'static [QuantizePreset] = &[
        QuantizePreset::Whole,
        QuantizePreset::Half,
        QuantizePreset::Quarter,
        QuantizePreset::Eighth,
        QuantizePreset::Sixteenth,
        QuantizePreset::ThirtySec,
        QuantizePreset::SixtyFourth,
        QuantizePreset::OneTwentyEighth,
        // Triplets
        QuantizePreset::QuarterTriplet,
        QuantizePreset::EighthTriplet,
        QuantizePreset::SixteenthTriplet,
        QuantizePreset::ThirtySecTriplet,
    ];

    /// The denominator `N` from the `1/N` notation for named presets.
    /// Returns `0` for `Custom`.
    pub const fn denominator_value(&self) -> u32 {
        match self {
            QuantizePreset::Whole => 1,
            QuantizePreset::Half => 2,
            QuantizePreset::Quarter => 4,
            QuantizePreset::Eighth => 8,
            QuantizePreset::Sixteenth => 16,
            QuantizePreset::ThirtySec => 32,
            QuantizePreset::SixtyFourth => 64,
            QuantizePreset::OneTwentyEighth => 128,
            QuantizePreset::QuarterTriplet => 6,
            QuantizePreset::EighthTriplet => 12,
            QuantizePreset::SixteenthTriplet => 24,
            QuantizePreset::ThirtySecTriplet => 48,
            QuantizePreset::Custom(_, _) => 0,
        }
    }

    /// Short human-readable label (used in the button).
    pub fn label(&self) -> &str {
        match self {
            QuantizePreset::Whole => "1/1",
            QuantizePreset::Half => "1/2",
            QuantizePreset::Quarter => "1/4",
            QuantizePreset::Eighth => "1/8",
            QuantizePreset::Sixteenth => "1/16",
            QuantizePreset::ThirtySec => "1/32",
            QuantizePreset::SixtyFourth => "1/64",
            QuantizePreset::OneTwentyEighth => "1/128",
            QuantizePreset::QuarterTriplet => "1/6",
            QuantizePreset::EighthTriplet => "1/12",
            QuantizePreset::SixteenthTriplet => "1/24",
            QuantizePreset::ThirtySecTriplet => "1/48",
            QuantizePreset::Custom(_, _) => "Custom",
        }
    }

    /// Tick interval for this preset, given the MIDI file's `ticks_per_beat` (PPQ).
    ///
    /// For named presets (`1/N`): `tick_interval = PPQ × 4 / N` (ceiling division).
    /// For `Custom(num, den)`: `tick_interval = PPQ × 4 × num / den`.
    pub fn tick_interval(&self, ppq: u32) -> u32 {
        let ppq = ppq.max(1);
        match self {
            QuantizePreset::Custom(num, den) => {
                let d = (*den).max(1);
                let total = ppq.saturating_mul(4).saturating_mul(*num);
                total.div_ceil(d)
            }
            named => {
                let d = named.denominator_value();
                let total = ppq.saturating_mul(4);
                total.div_ceil(d)
            }
        }
    }

    /// Snap a tick value to the nearest quantization grid boundary (round).
    pub fn snap_tick(&self, tick: f64, ppq: u32) -> f64 {
        let interval = self.tick_interval(ppq) as f64;
        if interval <= 0.0 {
            return tick;
        }
        (tick / interval).round() * interval
    }

    /// Snap a tick value to the next quantization grid boundary (ceil).
    pub fn snap_tick_ceil(&self, tick: f64, ppq: u32) -> f64 {
        let interval = self.tick_interval(ppq) as f64;
        if interval <= 0.0 {
            return tick;
        }
        (tick / interval).ceil() * interval
    }

    /// Snap a tick value to the previous quantization grid boundary (floor).
    pub fn snap_tick_floor(&self, tick: f64, ppq: u32) -> f64 {
        let interval = self.tick_interval(ppq) as f64;
        if interval <= 0.0 {
            return tick;
        }
        (tick / interval).floor() * interval
    }

    /// Display string for the dropdown list, e.g. `"1/8  (60 tick)"`.
    pub fn display_item(&self, ppq: u32) -> String {
        let ticks = self.tick_interval(ppq);
        match self {
            QuantizePreset::Custom(num, den) => {
                format!("{}/{}  ({} tick)", num, den, ticks)
            }
            _ => format!("{}  ({} tick)", self.label(), ticks),
        }
    }

    /// Return `(numerator, denominator)` for `Custom`, or `(1, denominator)` for named presets.
    pub fn as_fraction(&self) -> (u32, u32) {
        match self {
            QuantizePreset::Custom(num, den) => (*num, *den),
            named => {
                let d = named.denominator_value();
                if d == 0 { (1, 1) } else { (1, d) }
            }
        }
    }

    /// Human-friendly short text for the button.
    pub fn button_text(&self) -> String {
        match self {
            QuantizePreset::Custom(num, den) => format!("{}/{}", num, den),
            other => other.label().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quarter_at_480ppq() {
        // 1/4: 480*4/4 = 480
        assert_eq!(QuantizePreset::Quarter.tick_interval(480), 480);
    }

    #[test]
    fn test_eighth_at_480ppq() {
        // 1/8: 480*4/8 = 240
        assert_eq!(QuantizePreset::Eighth.tick_interval(480), 240);
    }

    #[test]
    fn test_whole_at_480ppq() {
        // 1/1: 480*4/1 = 1920
        assert_eq!(QuantizePreset::Whole.tick_interval(480), 1920);
    }

    #[test]
    fn test_custom_half() {
        // Custom(1,2): 480*4*1/2 = 960
        assert_eq!(QuantizePreset::Custom(1, 2).tick_interval(480), 960);
    }

    #[test]
    fn test_snap_tick_rounds() {
        // Quarter at 480ppq → interval=480
        assert_eq!(QuantizePreset::Quarter.snap_tick(100.0, 480), 0.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick(240.0, 480), 480.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick(480.0, 480), 480.0);
    }

    #[test]
    fn test_snap_tick_ceil() {
        assert_eq!(QuantizePreset::Quarter.snap_tick_ceil(100.0, 480), 480.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_ceil(240.0, 480), 480.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_ceil(480.0, 480), 480.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_ceil(481.0, 480), 960.0);
    }

    #[test]
    fn test_snap_tick_floor() {
        assert_eq!(QuantizePreset::Quarter.snap_tick_floor(100.0, 480), 0.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_floor(240.0, 480), 0.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_floor(479.0, 480), 0.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_floor(480.0, 480), 480.0);
        assert_eq!(QuantizePreset::Quarter.snap_tick_floor(720.0, 480), 480.0);
    }

    #[test]
    fn test_default_is_quarter() {
        assert_eq!(QuantizePreset::default(), QuantizePreset::Quarter);
    }

    #[test]
    fn test_denominator_value() {
        assert_eq!(QuantizePreset::Sixteenth.denominator_value(), 16);
        assert_eq!(QuantizePreset::EighthTriplet.denominator_value(), 12);
        assert_eq!(QuantizePreset::Custom(1, 4).denominator_value(), 0);
    }

    #[test]
    fn test_as_fraction() {
        assert_eq!(QuantizePreset::Quarter.as_fraction(), (1, 4));
        assert_eq!(QuantizePreset::Custom(3, 8).as_fraction(), (3, 8));
    }

    #[test]
    fn test_triplet_intervals() {
        let ppq = 480;
        assert_eq!(QuantizePreset::QuarterTriplet.tick_interval(ppq), 320);
        assert_eq!(QuantizePreset::EighthTriplet.tick_interval(ppq), 160);
        assert_eq!(QuantizePreset::SixteenthTriplet.tick_interval(ppq), 80);
    }

    #[test]
    fn test_half_and_thirtysec_intervals() {
        let ppq = 480;
        assert_eq!(QuantizePreset::Half.tick_interval(ppq), 960);
        assert_eq!(QuantizePreset::ThirtySec.tick_interval(ppq), 60);
        assert_eq!(QuantizePreset::SixtyFourth.tick_interval(ppq), 30);
    }

    #[test]
    fn test_label_not_empty() {
        for preset in QuantizePreset::ALL {
            assert!(!preset.label().is_empty(), "label should not be empty for {:?}", preset);
        }
    }

    #[test]
    fn test_button_text_not_empty() {
        for preset in QuantizePreset::ALL {
            assert!(!preset.button_text().is_empty(), "button_text should not be empty for {:?}", preset);
        }
    }

    #[test]
    fn test_display_item_not_empty() {
        for preset in QuantizePreset::ALL {
            assert!(!preset.display_item(480).is_empty(), "display_item should not be empty for {:?}", preset);
        }
    }

    #[test]
    fn test_snap_tick_zero_interval() {
        // When interval is 0, snap_tick should return the input unchanged
        // (edge case protection)
        let result = QuantizePreset::Quarter.snap_tick(100.0, 0);
        assert_eq!(result, 100.0);
    }

    #[test]
    fn test_all_presets_have_unique_labels() {
        let mut labels = std::collections::HashSet::new();
        for preset in QuantizePreset::ALL {
            assert!(labels.insert(preset.label()), "duplicate label: {}", preset.label());
        }
    }
}
