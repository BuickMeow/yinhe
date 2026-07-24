/// Shared grid-building utilities used by both pianoroll and arrangement instances.
use yinhe_types::{
    build_time_sig_segments, compute_measure_divisor, measure_ticks, TimeSigEvent,
    TimelineViewBase,
};

use crate::vertex::{DrawInstance, pack_props, pack_rgba};

// ── Grid density thresholds (pixels) ──
/// measure 线最小像素间距；低于此值则按 2/4/8… 小节合并显示。
const MIN_MEASURE_PX: f32 = 20.0;
/// beat 线最小像素间距。
const MIN_BEAT_PX: f32 = 8.0;
/// sub-beat 线最小像素间距。
const MIN_SUB_BEAT_PX: f32 = 3.0;
/// tick 线最小像素间距（放大到每个 tick 可见时才画）。
const MIN_TICK_PX: f32 = 2.0;

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

/// 计算多小节合并的步长倍数（委托给 yinhe_types 共享实现）。
fn measure_divisor_for(pixels_per_measure: f32) -> u32 {
    compute_measure_divisor(pixels_per_measure, MIN_MEASURE_PX)
}

/// Build timeline grid lines shared by pianoroll and arrangement views.
///
/// 显示分级（根据 `pixels_per_tick` 自适应）：
/// - 缩很小时：measure 线按 2/4/8… 小节合并，避免过密
/// - 正常：measure 线 + beat 线 + sub-beat 线
/// - 放很大：额外画出每个 tick 的分界线
///
/// `sub_beat_color` / `tick_color`: `Some` 时才考虑对应级别的线，
/// 最终是否绘制还取决于像素间距是否达标。
///
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
    tick_color: Option<(f32, f32, f32, f32)>,
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

    for i in 0..segments.len() {
        let (seg_start, num, den) = segments[i];
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
        let seg_start_f = seg_start as f64;
        if seg_start_f > tick_end {
            break;
        }

        let ticks_per_measure = measure_ticks(tpb, num, den);
        let ticks_per_beat = (ticks_per_measure / num as u32).max(1);

        let pixels_per_measure = ticks_per_measure as f32 * ppu;
        let pixels_per_beat = ticks_per_beat as f32 * ppu;
        let pixels_per_sub = ticks_per_sub as f32 * ppu;

        // 多小节合并
        let measure_divisor = measure_divisor_for(pixels_per_measure);
        let merged_measure_ticks = ticks_per_measure.saturating_mul(measure_divisor);

        // 各级别是否绘制
        let show_tick = tick_color.is_some() && ppu >= MIN_TICK_PX;
        let show_sub_beat = sub_beat_color.is_some() && pixels_per_sub >= MIN_SUB_BEAT_PX;
        let show_beat = pixels_per_beat >= MIN_BEAT_PX;

        // 遍历步长 = 当前最细可见级别的步长，保证遍历量始终可控
        let step = if show_tick {
            1u32
        } else if show_sub_beat {
            ticks_per_sub
        } else if show_beat {
            ticks_per_beat
        } else {
            merged_measure_ticks.max(1)
        };

        let first_tick = seg_start_f.max(tick_start);
        let step_f = step as f64;
        let first = ((first_tick / step_f).floor() as u32)
            .saturating_mul(step)
            .max(seg_start);

        let mut tick = first;
        while (tick as f64) <= tick_end && tick < seg_end {
            let local = tick - seg_start;

            let x = x_origin + tick as f32 * ppu;
            if x >= left_w && x <= w {
                let is_measure = local % merged_measure_ticks == 0;
                let beat_local = local % ticks_per_measure;
                let is_beat_pos = beat_local.is_multiple_of(ticks_per_beat) && beat_local > 0;
                let is_sub_pos = local % ticks_per_sub == 0;

                if is_measure {
                    push_grid_line(out, x, h, 2.0, measure_color, tick);
                } else if show_beat && is_beat_pos {
                    push_grid_line(out, x, h, 1.0, beat_color, tick);
                } else if show_sub_beat && is_sub_pos {
                    push_grid_line(out, x, h, 1.0, sub_beat_color.unwrap(), tick);
                } else if show_tick {
                    push_grid_line(out, x, h, 1.0, tick_color.unwrap(), tick);
                }
            }
            tick += step;
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
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), Some((0.13, 0.13, 0.15, 0.6)), 0.0);
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
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), Some((0.13, 0.13, 0.15, 0.6)), 0.0);
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
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), Some((0.13, 0.13, 0.15, 0.6)), 0.0);
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
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), None, None, 0.0);
        assert!(!out.is_empty());
    }

    fn make_base(ppu: f32) -> TimelineViewBase {
        TimelineViewBase {
            pixels_per_tick: ppu,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 60.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
        }
    }

    #[test]
    fn test_compute_measure_divisor_no_merge() {
        // pixels_per_measure = 1920 * 0.1 = 192 >= 20 → 不合并
        assert_eq!(measure_divisor_for(192.0), 1);
        assert_eq!(measure_divisor_for(20.0), 1);
    }

    #[test]
    fn test_compute_measure_divisor_merge() {
        // 10px → 需要 2x → 20px（刚好达标）
        assert_eq!(measure_divisor_for(10.0), 2);
        // 5px → 4x → 20px
        assert_eq!(measure_divisor_for(5.0), 4);
        // 3px → 8x → 24px
        assert_eq!(measure_divisor_for(3.0), 8);
        // 0.3px → 64x（上限）
        assert_eq!(measure_divisor_for(0.3), 64);
    }

    #[test]
    fn test_build_timeline_grid_multi_measure_merge() {
        // 缩到极小：pixels_per_measure = 1920 * 0.005 = 9.6 < 20 → 合并为 2 小节
        let mut out = Vec::new();
        let base = make_base(0.005);
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &[],
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), None, 0.0);
        // 合并后 measure 线间距 = 1920*2 * 0.005 = 19.2px，仍 < 20，实际会合并到 4 小节
        // 4 小节间距 = 1920*4 * 0.005 = 38.4px >= 20
        let measure_ticks: Vec<u32> = out.iter()
            .filter(|i| i.w == 2.0) // measure 线宽 2.0
            .map(|i| i.tag)
            .collect();
        // 第一条在 tick 0，下一条应在 tick 1920*4 = 7680
        assert!(measure_ticks.iter().any(|&t| t == 0), "first measure at tick 0");
        assert!(measure_ticks.iter().any(|&t| t == 7680), "merged measure at tick 7680 (4 bars)");
        // 不应有 tick 1920 的 measure 线（被合并掉）
        assert!(!measure_ticks.iter().any(|&t| t == 1920), "tick 1920 should be merged away");
    }

    #[test]
    fn test_build_timeline_grid_tick_lines() {
        // 放到极大：pixels_per_tick = 3.0 >= 2.0 → 显示 tick 线
        let mut out = Vec::new();
        let base = make_base(3.0);
        // left_panel_width=60, 视口 800，可见 tick 范围约 0..246
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &[],
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), Some((0.13, 0.13, 0.15, 0.6)), 0.0);
        // 应该有 tick 线（tag 不是 measure/beat/sub-beat 倍数的线）
        let has_tick_line = out.iter().any(|i| {
            let t = i.tag;
            t % 120 != 0 // 非 sub-beat 倍数
        });
        assert!(has_tick_line, "should have tick-level grid lines at high zoom");
    }

    #[test]
    fn test_build_timeline_grid_no_tick_lines_when_too_dense() {
        // 中等缩放：pixels_per_tick = 0.5 < 2.0 → 不显示 tick 线
        let mut out = Vec::new();
        let base = make_base(0.5);
        build_timeline_grid(&mut out, 800.0, 500.0, &base, 480, 4, 2, &[],
            (0.3, 0.3, 0.35, 1.0), (0.2, 0.2, 0.25, 1.0), Some((0.16, 0.16, 0.18, 1.0)), Some((0.13, 0.13, 0.15, 0.6)), 0.0);
        // 不应有 tick 线（所有线的 tag 都应是 sub-beat 的倍数）
        let all_aligned = out.iter().all(|i| i.tag % 120 == 0);
        assert!(all_aligned, "no tick lines when pixels_per_tick < MIN_TICK_PX");
    }

    /// 回归测试：不同 tpb 必须产生不同的小节线位置。
    ///
    /// 背景：grid 缓存键曾遗漏 tpb 字段，导致两个 tpb 不同但拍号事件相同的 MIDI
    /// 共享同一缓存键，加载新 MIDI 后网格线不更新。
    /// 此测试验证 `build_timeline_grid` 本身正确响应 tpb 变化
    /// （缓存已移除，但此测试仍作为网格构建正确性的回归保护）。
    #[test]
    fn test_grid_measure_lines_differ_by_tpb() {
        let base = make_base(0.1);
        let measure_color = (0.3, 0.3, 0.35, 1.0);
        let beat_color = (0.2, 0.2, 0.25, 1.0);

        // tpb=480, 4/4 → measure = 1920 ticks
        let mut out_480 = Vec::new();
        build_timeline_grid(&mut out_480, 800.0, 500.0, &base, 480, 4, 2, &[],
            measure_color, beat_color, None, None, 0.0);
        let measure_ticks_480: Vec<u32> = out_480.iter()
            .filter(|i| i.w == 2.0)
            .map(|i| i.tag)
            .collect();

        // tpb=960, 4/4 → measure = 3840 ticks
        let mut out_960 = Vec::new();
        build_timeline_grid(&mut out_960, 800.0, 500.0, &base, 960, 4, 2, &[],
            measure_color, beat_color, None, None, 0.0);
        let measure_ticks_960: Vec<u32> = out_960.iter()
            .filter(|i| i.w == 2.0)
            .map(|i| i.tag)
            .collect();

        // tpb=480 应在小节线 1920 处有线，tpb=960 不应有
        assert!(measure_ticks_480.contains(&1920),
            "tpb=480 should have a measure line at tick 1920, got {:?}", measure_ticks_480);
        assert!(!measure_ticks_960.contains(&1920),
            "tpb=960 should NOT have a measure line at tick 1920, got {:?}", measure_ticks_960);
        // 两组小节线位置必须不同
        assert_ne!(measure_ticks_480, measure_ticks_960,
            "different tpb must produce different measure line positions");
    }
}
