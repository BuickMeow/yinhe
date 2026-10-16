/// Quantization preset: snap notes to a regular grid.
///
/// Two modes:
/// - `Fraction(num, den)`: snap to `num/den` of a whole note
///   (e.g. `Fraction(1, 16)` = 1/16 note, `Fraction(3, 8)` = 3/8 note)
/// - `Absolute(n)`: snap every `n` ticks (PPQ-independent)
///
/// 另有两个特殊档位：`Auto`（自适应，随缩放自动变细/变粗，使用前必须
/// 经 [`QuantizePreset::resolved`] 解析）与 `Off`（关闭吸附）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantizePreset {
    /// Note fraction: `num / den` of a whole note.
    /// Tick interval = PPQ × 4 × num / den.
    Fraction(u32, u32),
    /// Absolute tick interval: snap every `n` ticks.
    Absolute(u32),
    /// 自适应网格：随视图缩放自动变细/变粗（放大越大格越细，最大到 1 tick）。
    /// **使用前必须经 [`QuantizePreset::resolved`] 解析成具体档位**；
    /// 未解析时按"不吸附"处理（防御性）。
    Auto,
    /// 关闭吸附：所有 snap 原样返回（保留原 tick）。
    Off,
}

impl QuantizePreset {
    /// 固定分数档位列表（不包括 `Absolute`）。
    /// 桌面量化下拉菜单已改用 [`presets_for_ppq`] 按 PPQ 动态推导；
    /// 此常量供 Android 弹窗与测试使用。
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
            QuantizePreset::Auto => "Auto".to_string(),
            QuantizePreset::Off => "Off".to_string(),
        }
    }

    /// Tick interval for this preset, given the MIDI file's `ticks_per_beat` (PPQ).
    ///
    /// For `Fraction(num, den)`: `tick_interval = PPQ × 4 × num / den`.
    /// For `Absolute(n)`: returns `n` directly.
    /// For `Auto` / `Off`: 0（无静态网格；0 使 snap 恒等返回）。
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
            QuantizePreset::Auto | QuantizePreset::Off => 0,
        }
    }

    /// 是否"自由"（不吸附）：`Off` 与未解析的 `Auto`。
    ///
    /// `Auto` 正常使用前会被 [`QuantizePreset::resolved`] 解析掉；
    /// 未解析的残留在 snap 路径上按不吸附处理，避免错误吸附。
    pub fn is_free(&self) -> bool {
        matches!(self, QuantizePreset::Auto | QuantizePreset::Off)
    }

    /// 把 `Auto` 解析为当前缩放下最合适的档位；其他档位原样返回。
    ///
    /// `pixels_per_tick` 为视图水平缩放（px / tick）。
    pub fn resolved(self, pixels_per_tick: f32, ppq: u32) -> QuantizePreset {
        match self {
            QuantizePreset::Auto => auto_preset(pixels_per_tick, ppq),
            other => other,
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

    /// Display string for the dropdown list, e.g. `"1/8  (60 tick)"` or `"3 tick"`.
    /// tick 是专有名词，所有语言统一使用，不做本地化。
    pub fn display_item(&self, ppq: u32) -> String {
        match self {
            QuantizePreset::Fraction(_, _) => {
                let ticks = self.tick_interval(ppq);
                format!("{}  ({} tick)", self.label(), ticks)
            }
            QuantizePreset::Absolute(n) => {
                format!("{} tick", n)
            }
            QuantizePreset::Auto | QuantizePreset::Off => self.label(),
        }
    }
}

/// 按 PPQ 推导的常用档位列表（桌面下拉菜单用）：
/// - **2 分频链**：`1/1, 1/2, 1/4, …` 细分到最后一个整数 tick 档
///   （如 1920 PPQ → `1/512` = 15 tick；再细分 7.5 非整即止）；
/// - **三连音链**：`1/6, 1/12, …` 同样细分到底
///   （如 1920 PPQ → `1/1536` = 5 tick）。
///
/// 列表显示仍用分数（`display_item` 附带折算出的 tick 数），
/// 随 PPQ 灵活伸缩（1920 PPQ 到 15 tick，480 PPQ 到 15 tick，96 PPQ 到 3 tick）。
pub fn presets_for_ppq(ppq: u32) -> Vec<QuantizePreset> {
    let whole = ppq.max(1).saturating_mul(4); // 全音符的 tick 数
    let mut out = Vec::new();
    push_dyadic_chain(&mut out, whole, 1); // 2 分频链
    push_dyadic_chain(&mut out, whole, 6); // 三连音链（分母 3 × 2^k）
    out
}

/// 从分母 `den` 起按 ×2 细分入列；分母不再整除全音符 tick 数（tick 非整数）即止。
fn push_dyadic_chain(out: &mut Vec<QuantizePreset>, whole: u32, mut den: u32) {
    while den <= whole && whole.is_multiple_of(den) {
        out.push(QuantizePreset::Fraction(1, den));
        den = match den.checked_mul(2) {
            Some(next) => next,
            None => break,
        };
    }
}

impl Default for QuantizePreset {
    fn default() -> Self {
        QuantizePreset::Fraction(1, 4)
    }
}

// ── 自适应网格（Auto） ──

/// 自适应网格的目标屏幕格宽（像素）：解析时选格宽最接近该值的档位。
/// 13px 保证默认视图（0.15 px/tick）落在 1/16，最大缩放（10 px/tick）能到 1 tick。
pub const AUTO_TARGET_PX: f32 = 13.0;

/// 自适应档位的候选序列（由粗到细）：标准时值 1/1…1/128，再延伸到 8/4/2/1 tick。
/// 不含三连音：自适应沿 2 分频链平滑切换，避免缩放时网格跳到奇数细分。
const AUTO_SEQUENCE: &[QuantizePreset] = &[
    QuantizePreset::Fraction(1, 1),
    QuantizePreset::Fraction(1, 2),
    QuantizePreset::Fraction(1, 4),
    QuantizePreset::Fraction(1, 8),
    QuantizePreset::Fraction(1, 16),
    QuantizePreset::Fraction(1, 32),
    QuantizePreset::Fraction(1, 64),
    QuantizePreset::Fraction(1, 128),
    QuantizePreset::Absolute(8),
    QuantizePreset::Absolute(4),
    QuantizePreset::Absolute(2),
    QuantizePreset::Absolute(1),
];

/// 自适应网格解析：选"屏幕格宽最接近 [`AUTO_TARGET_PX`]"的档位（对数距离）。
///
/// - 网格间距随 `pixels_per_tick` 变小而变粗、变大而变细；
/// - 序列按 tick_interval 做单调过滤：PPQ 较小时（如 96）跳过比
///   1/128 还粗的绝对 tick 档，保证结果始终是有效且单调变细的网格；
/// - 放大到最大缩放时解析为 1 tick，可写 tick 级音符。
pub fn auto_preset(pixels_per_tick: f32, ppq: u32) -> QuantizePreset {
    let ppu = pixels_per_tick.max(0.001);
    let mut best: Option<(QuantizePreset, f32)> = None;
    let mut last_interval = u32::MAX;
    for &cand in AUTO_SEQUENCE {
        let interval = cand.tick_interval(ppq);
        // 非单调（比上一档还粗）的候选跳过：小 PPQ 下绝对 tick 档会越界。
        if interval == 0 || interval >= last_interval {
            continue;
        }
        last_interval = interval;
        let px = interval as f32 * ppu;
        let err = (px / AUTO_TARGET_PX).ln().abs();
        if best.is_none_or(|(_, best_err)| err < best_err) {
            best = Some((cand, err));
        }
    }
    best.map_or(QuantizePreset::Fraction(1, 1), |(cand, _)| cand)
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
    if quantize.is_free() {
        return tick;
    }
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
    if quantize.is_free() {
        return tick;
    }
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
    if quantize.is_free() {
        return tick;
    }
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
    fn off_is_identity_for_all_snap_forms() {
        let ts_events: [TimeSigEvent; 0] = [];
        let bar_data = Some((480u32, 4u8, 4u8, ts_events.as_slice()));
        for tick in [0.0, 1.0, 123.7, 999.99, 4800.0] {
            assert_eq!(snap_tick(tick, QuantizePreset::Off, 480, None), tick);
            assert_eq!(snap_tick(tick, QuantizePreset::Off, 480, bar_data), tick);
            assert_eq!(
                snap_tick_ceil(tick, QuantizePreset::Off, 480, bar_data),
                tick
            );
            assert_eq!(
                snap_tick_floor(tick, QuantizePreset::Off, 480, bar_data),
                tick
            );
        }
    }

    #[test]
    fn auto_resolves_by_zoom() {
        // 默认视图（0.15 px/tick）解析为 1/16；最大缩放（10 px/tick）到 1 tick。
        assert_eq!(auto_preset(0.15, 480), QuantizePreset::Fraction(1, 16));
        assert_eq!(auto_preset(10.0, 480), QuantizePreset::Absolute(1));
        assert_eq!(auto_preset(1.0, 480), QuantizePreset::Fraction(1, 128));
        assert_eq!(auto_preset(0.001, 480), QuantizePreset::Fraction(1, 1));
    }

    #[test]
    fn auto_resolves_monotonic_across_zoom() {
        // 缩放连续增大 → 解析结果的 tick_interval 单调不增（网格只会越来越细）。
        let ppq = 480;
        let mut last = u32::MAX;
        let mut ppu = 0.001f32;
        while ppu <= 10.0 {
            let interval = auto_preset(ppu, ppq).tick_interval(ppq);
            assert!(
                interval <= last,
                "ppu={ppu} interval={interval} last={last}"
            );
            last = interval;
            ppu *= 1.1;
        }
    }

    #[test]
    fn auto_small_ppq_skips_non_monotonic_ticks() {
        // 96 PPQ：1/128 = 3t，绝对档 8/4 应被跳过，结果仍是合法网格。
        let p = auto_preset(10.0, 96);
        let interval = p.tick_interval(96);
        assert!((1..=3).contains(&interval), "got {p:?}");
        assert_eq!(p, QuantizePreset::Absolute(1));
    }

    #[test]
    fn resolved_passthrough_and_auto() {
        assert_eq!(
            QuantizePreset::Fraction(1, 8).resolved(10.0, 480),
            QuantizePreset::Fraction(1, 8)
        );
        assert_eq!(QuantizePreset::Off.resolved(10.0, 480), QuantizePreset::Off);
        assert_eq!(
            QuantizePreset::Auto.resolved(10.0, 480),
            QuantizePreset::Absolute(1)
        );
        assert!(!QuantizePreset::Auto.resolved(10.0, 480).is_free());
    }

    #[test]
    fn free_presets_have_zero_interval() {
        assert_eq!(QuantizePreset::Auto.tick_interval(480), 0);
        assert_eq!(QuantizePreset::Off.tick_interval(480), 0);
        assert!(QuantizePreset::Auto.is_free());
        assert!(QuantizePreset::Off.is_free());
    }

    #[test]
    fn presets_for_ppq_1920_chains() {
        let list = presets_for_ppq(1920);
        // 2 分频链：1/1 → 1/512（15 tick；再细分 7.5 非整即止）。
        assert_eq!(list.first(), Some(&QuantizePreset::Fraction(1, 1)));
        assert!(list.contains(&QuantizePreset::Fraction(1, 512)));
        assert!(!list.contains(&QuantizePreset::Fraction(1, 1024)));
        // 三连音链：1/6（1280 tick）→ 1/1536（5 tick；再细分 2.5 非整即止）。
        assert!(list.contains(&QuantizePreset::Fraction(1, 6)));
        assert_eq!(QuantizePreset::Fraction(1, 6).tick_interval(1920), 1280);
        assert!(list.contains(&QuantizePreset::Fraction(1, 1536)));
        assert!(!list.contains(&QuantizePreset::Fraction(1, 3072)));
        // 全链整数 tick、无重复。
        let mut seen: Vec<QuantizePreset> = Vec::new();
        for p in &list {
            assert!(p.tick_interval(1920) >= 1, "{p:?}");
            assert!(!seen.contains(p), "重复档位 {p:?}");
            seen.push(*p);
        }
    }

    #[test]
    fn presets_for_ppq_480_chains() {
        let list = presets_for_ppq(480);
        assert!(list.contains(&QuantizePreset::Fraction(1, 128)));
        assert_eq!(QuantizePreset::Fraction(1, 128).tick_interval(480), 15);
        assert!(list.contains(&QuantizePreset::Fraction(1, 384)));
        assert_eq!(QuantizePreset::Fraction(1, 384).tick_interval(480), 5);
        assert!(!list.contains(&QuantizePreset::Fraction(1, 256))); // 7.5 非整
        assert!(!list.contains(&QuantizePreset::Fraction(1, 768))); // 2.5 非整
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
