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
        QuantizePreset::Fraction(1, 1),   // Whole
        QuantizePreset::Fraction(1, 2),   // Half
        QuantizePreset::Fraction(1, 4),   // Quarter
        QuantizePreset::Fraction(1, 8),   // Eighth
        QuantizePreset::Fraction(1, 16),  // Sixteenth
        QuantizePreset::Fraction(1, 32),  // 1/32
        QuantizePreset::Fraction(1, 64),  // 1/64
        QuantizePreset::Fraction(1, 128), // 1/128
        // Triplets
        QuantizePreset::Fraction(1, 6),  // Quarter triplet  (was 1/4T)
        QuantizePreset::Fraction(1, 12), // Eighth triplet   (was 1/8T)
        QuantizePreset::Fraction(1, 24), // 1/16 triplet     (was 1/16T)
        QuantizePreset::Fraction(1, 48), // 1/32 triplet     (was 1/32T)
    ];

    /// Human-readable label (used in the button and dropdown).
    pub fn label(&self) -> String {
        match self {
            QuantizePreset::Fraction(num, den) => format!("{}/{}", num, den),
            QuantizePreset::Absolute(n) => format!("{} tick", n),
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
                ppq.max(1)
                    .saturating_mul(4)
                    .saturating_mul(*num)
                    .div_ceil(d)
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

// ── 小节感知的 snap（原 yinhe-egui/src/view_interaction.rs，下沉至此） ──

use yinhe_types::{TimeSigEvent, measure_bounds_at_tick};

/// Snap 到最近网格，带小节线感知（小节起始与下一小节起始作为候选）。
pub fn snap_tick(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) = measure_bounds_at_tick(tick, tpb, num, den, events);
        let offset = tick - bar_start;
        let snapped_offset = quantize.snap_tick(offset, ppq);
        let grid_tick = bar_start + snapped_offset;
        if (tick - next_bar).abs() < (tick - grid_tick).abs() {
            next_bar
        } else {
            grid_tick
        }
    } else {
        quantize.snap_tick(tick, ppq)
    }
}

/// Snap 到下一网格（ceil），带小节线感知。
pub fn snap_tick_ceil(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) = measure_bounds_at_tick(tick, tpb, num, den, events);
        let offset = tick - bar_start;
        let snapped_offset = quantize.snap_tick_ceil(offset, ppq);
        let grid_tick = bar_start + snapped_offset;
        if (tick - next_bar).abs() < (tick - grid_tick).abs() {
            next_bar
        } else {
            grid_tick
        }
    } else {
        quantize.snap_tick_ceil(tick, ppq)
    }
}

/// Snap 到上一网格（floor），带小节线感知。
pub fn snap_tick_floor(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) = measure_bounds_at_tick(tick, tpb, num, den, events);
        let offset = tick - bar_start;
        let snapped_offset = quantize.snap_tick_floor(offset, ppq);
        let grid_tick = bar_start + snapped_offset;
        if (tick - next_bar).abs() < (tick - grid_tick).abs() {
            next_bar
        } else {
            grid_tick
        }
    } else {
        quantize.snap_tick_floor(tick, ppq)
    }
}

// ── 锚点线（直线/剪刀工具） ──

/// 线在 `key` 行处的时间坐标（两个锚点同 key 时返回起点 tick）。
pub fn line_tick_at_key(start: (f64, u8), end: (f64, u8), key: u8) -> f64 {
    let (t1, k1) = start;
    let (t2, k2) = end;
    if k1 == k2 {
        return t1;
    }
    t1 + (t2 - t1) * (key as f64 - k1 as f64) / (k2 as f64 - k1 as f64)
}

/// 剪刀锚点线的逐行切点：`(key, cut_tick)`，按 key 升序。
///
/// - 同 key（单击/水平）：在该 tick 全列切一刀；
/// - 跨 key：每行取线与该行相交的 tick，吸附量化（含小节感知）。
pub fn line_cuts(
    start: (f64, u8),
    end: (f64, u8),
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> Vec<(u8, u32)> {
    let snap = |t: f64| -> u32 { snap_tick(t, quantize, ppq, bar_line_data).max(0.0) as u32 };
    let (t1, k1) = start;
    let (_, k2) = end;
    if k1 == k2 {
        let cut = snap(t1);
        return (0..=yinhe_types::MAX_KEY).map(|k| (k, cut)).collect();
    }
    let (lo, hi) = (k1.min(k2), k1.max(k2));
    (lo..=hi)
        .map(|k| (k, snap(line_tick_at_key(start, end, k))))
        .collect()
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
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_ceil(100.0, 480),
            480.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_ceil(240.0, 480),
            480.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_ceil(480.0, 480),
            480.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_ceil(481.0, 480),
            960.0
        );
    }

    #[test]
    fn test_snap_tick_floor() {
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_floor(100.0, 480),
            0.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_floor(240.0, 480),
            0.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_floor(479.0, 480),
            0.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_floor(480.0, 480),
            480.0
        );
        assert_eq!(
            QuantizePreset::Fraction(1, 4).snap_tick_floor(720.0, 480),
            480.0
        );
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
            assert!(
                !preset.label().is_empty(),
                "label should not be empty for {:?}",
                preset
            );
        }
    }

    #[test]
    fn test_display_item_not_empty() {
        for preset in QuantizePreset::ALL {
            assert!(
                !preset.display_item(480).is_empty(),
                "display_item should not be empty for {:?}",
                preset
            );
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
            assert!(
                labels.insert(preset.label()),
                "duplicate label: {}",
                preset.label()
            );
        }
    }

    #[test]
    fn line_cuts_vertical_same_tick_across_rows() {
        let c = line_cuts(
            (125.0, 60),
            (125.0, 63),
            QuantizePreset::Fraction(1, 16),
            480,
            None,
        );
        assert_eq!(c, vec![(60, 120), (61, 120), (62, 120), (63, 120)]);
    }

    #[test]
    fn line_cuts_slanted_interpolates_per_row() {
        let c = line_cuts(
            (0.0, 60),
            (480.0, 63),
            QuantizePreset::Fraction(1, 16),
            480,
            None,
        );
        assert_eq!(c, vec![(60, 0), (61, 120), (62, 360), (63, 480)]);
    }

    #[test]
    fn line_cuts_click_cuts_whole_column() {
        let c = line_cuts(
            (100.0, 42),
            (100.0, 42),
            QuantizePreset::Fraction(1, 16),
            480,
            None,
        );
        assert_eq!(c.len(), yinhe_types::KEY_COUNT, "单击 = 全列所有键");
        assert!(c.iter().all(|&(_, t)| t == 120));
    }

    #[test]
    fn line_cuts_horizontal_uses_start_tick() {
        let c = line_cuts(
            (100.0, 42),
            (700.0, 42),
            QuantizePreset::Fraction(1, 16),
            480,
            None,
        );
        assert_eq!(c.len(), yinhe_types::KEY_COUNT);
        assert!(c.iter().all(|&(_, t)| t == 120));
    }

    #[test]
    fn line_tick_at_key_interpolates() {
        assert_eq!(line_tick_at_key((0.0, 60), (480.0, 64), 60), 0.0);
        assert_eq!(line_tick_at_key((0.0, 60), (480.0, 64), 62), 240.0);
        assert_eq!(line_tick_at_key((0.0, 60), (480.0, 64), 64), 480.0);
        assert_eq!(
            line_tick_at_key((100.0, 60), (700.0, 60), 60),
            100.0,
            "同 key 返回起点 tick"
        );
    }
}
