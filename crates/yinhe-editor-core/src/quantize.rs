/// Quantization preset: snap notes to a regular grid.
///
/// Two modes:
/// - `Fraction(num, den)`: snap to `num/den` of a whole note
///   (e.g. `Fraction(1, 16)` = 1/16 note, `Fraction(3, 8)` = 3/8 note)
/// - `Absolute(n)`: snap every `n` ticks (PPQ-independent)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantizePreset {
    /// Note fraction: `num / den` of a whole note.
    /// Tick interval = PPQ × 4 × num / den.
    Fraction(u32, u32),
    /// Absolute tick interval: snap every `n` ticks.
    Absolute(u32),
}

impl QuantizePreset {
    /// Common fraction presets in display order (excluding `Absolute`).
    pub const ALL: &'static [QuantizePreset] = &[
        QuantizePreset::Fraction(1, 1),    // Whole
        QuantizePreset::Fraction(1, 2),    // Half
        QuantizePreset::Fraction(1, 4),    // Quarter
        QuantizePreset::Fraction(1, 8),    // Eighth
        QuantizePreset::Fraction(1, 16),   // Sixteenth
        QuantizePreset::Fraction(1, 32),   // 1/32
        QuantizePreset::Fraction(1, 64),   // 1/64
        QuantizePreset::Fraction(1, 128),  // 1/128
        // Triplets
        QuantizePreset::Fraction(1, 6),    // Quarter triplet  (was 1/4T)
        QuantizePreset::Fraction(1, 12),   // Eighth triplet   (was 1/8T)
        QuantizePreset::Fraction(1, 24),   // 1/16 triplet     (was 1/16T)
        QuantizePreset::Fraction(1, 48),   // 1/32 triplet     (was 1/32T)
    ];

    /// Human-readable label (used in the button and dropdown).
    pub fn label(&self) -> String {
        match self {
            QuantizePreset::Fraction(num, den) => format!("{}/{}", num, den),
            QuantizePreset::Absolute(n) => format!("{} 刻度", n),
        }
    }

    /// Tick interval for this preset, given the MIDI file's `ticks_per_beat` (PPQ).
    ///
    /// For `Fraction(num, den)`: `tick_interval = PPQ × 4 × num / den`.
    /// For `Absolute(n)`: returns `n` directly.
    pub fn tick_interval(&self, ppq: u32) -> u32 {
        match self {
            QuantizePreset::Fraction(num, den) => {
                let d = (*den).max(1);
                ppq.max(1).saturating_mul(4).saturating_mul(*num).div_ceil(d)
            }
            QuantizePreset::Absolute(n) => *n,
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

    /// Display string for the dropdown list, e.g. `"1/8  (60 刻度)"` or `"3 刻度"`.
    pub fn display_item(&self, ppq: u32) -> String {
        match self {
            QuantizePreset::Fraction(_, _) => {
                let ticks = self.tick_interval(ppq);
                format!("{}  ({} 刻度)", self.label(), ticks)
            }
            QuantizePreset::Absolute(n) => {
                format!("{} 刻度", n)
            }
        }
    }
}

impl Default for QuantizePreset {
    fn default() -> Self {
        QuantizePreset::Fraction(1, 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quarter_at_480ppq() {
        // 1/4: 480*4/4 = 480
        assert_eq!(QuantizePreset::Fraction(1, 4).tick_interval(480), 480);
    }

    #[test]
    fn test_eighth_at_480ppq() {
        // 1/8: 480*4/8 = 240
        assert_eq!(QuantizePreset::Fraction(1, 8).tick_interval(480), 240);
    }

    #[test]
    fn test_whole_at_480ppq() {
        // 1/1: 480*4/1 = 1920
        assert_eq!(QuantizePreset::Fraction(1, 1).tick_interval(480), 1920);
    }

    #[test]
    fn test_custom_half() {
        // Fraction(1,2): 480*4*1/2 = 960
        assert_eq!(QuantizePreset::Fraction(1, 2).tick_interval(480), 960);
    }

    #[test]
    fn test_snap_tick_rounds() {
        // Quarter at 480ppq → interval=480
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick(100.0, 480), 0.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick(240.0, 480), 480.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick(480.0, 480), 480.0);
    }

    #[test]
    fn test_snap_tick_ceil() {
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_ceil(100.0, 480), 480.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_ceil(240.0, 480), 480.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_ceil(480.0, 480), 480.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_ceil(481.0, 480), 960.0);
    }

    #[test]
    fn test_snap_tick_floor() {
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_floor(100.0, 480), 0.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_floor(240.0, 480), 0.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_floor(479.0, 480), 0.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_floor(480.0, 480), 480.0);
        assert_eq!(QuantizePreset::Fraction(1, 4).snap_tick_floor(720.0, 480), 480.0);
    }

    #[test]
    fn test_default_is_quarter() {
        assert_eq!(QuantizePreset::default(), QuantizePreset::Fraction(1, 4));
    }

    #[test]
    fn test_triplet_intervals() {
        let ppq = 480;
        assert_eq!(QuantizePreset::Fraction(1, 6).tick_interval(ppq), 320);
        assert_eq!(QuantizePreset::Fraction(1, 12).tick_interval(ppq), 160);
        assert_eq!(QuantizePreset::Fraction(1, 24).tick_interval(ppq), 80);
    }

    #[test]
    fn test_absolute_tick() {
        assert_eq!(QuantizePreset::Absolute(3).tick_interval(480), 3);
        assert_eq!(QuantizePreset::Absolute(3).tick_interval(1), 3);
        assert_eq!(QuantizePreset::Absolute(3).label(), "3 tick");
    }

    #[test]
    fn test_half_and_thirtysec_intervals() {
        let ppq = 480;
        assert_eq!(QuantizePreset::Fraction(1, 2).tick_interval(ppq), 960);
        assert_eq!(QuantizePreset::Fraction(1, 32).tick_interval(ppq), 60);
        assert_eq!(QuantizePreset::Fraction(1, 64).tick_interval(ppq), 30);
    }

    #[test]
    fn test_label_not_empty() {
        for preset in QuantizePreset::ALL {
            assert!(!preset.label().is_empty(), "label should not be empty for {:?}", preset);
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
        let result = QuantizePreset::Fraction(1, 4).snap_tick(100.0, 0);
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
