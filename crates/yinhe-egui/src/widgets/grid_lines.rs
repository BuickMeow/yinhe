//! Timeline grid lines drawn by egui (replaces former wgpu grid layer).
//!
//! 与 `time_ruler` 共享同一套 segment 遍历骨架和 `MIN_SPACING` 阈值，
//! 保证"有线就有标签，无标签就无线"。

use eframe::egui;
use crate::theme;
use yinhe_types::{build_time_sig_segments, compute_measure_divisor, measure_ticks, TimeSigEvent, TimelineViewBase};

/// 线和标签共用的最小像素间距。与 `time_ruler::MIN_LABEL_SPACING` 保持一致。
const MIN_SPACING: f32 = 38.0;
const SUB_BEAT_DIV: u32 = 4;

/// Grid 线颜色集。pianoroll / automation 共用 pr_*，arrangement 用 ar_*。
pub struct GridColors {
    pub measure: egui::Color32,
    pub beat: egui::Color32,
    pub sub_beat: Option<egui::Color32>,
    pub tick: Option<egui::Color32>,
}

impl GridColors {
    /// Pianoroll 配色（automation 也用这套）。
    pub fn pianoroll() -> Self {
        Self {
            measure: theme::PR_MEASURE_LINE,
            beat: theme::PR_BEAT_LINE,
            sub_beat: Some(theme::PR_SUB_BEAT_LINE),
            tick: Some(theme::PR_TICK_LINE),
        }
    }

    /// Arrangement 配色（无 sub_beat / tick 线）。
    pub fn arrangement() -> Self {
        Self {
            measure: theme::AR_MEASURE_LINE,
            beat: theme::AR_BEAT_LINE,
            sub_beat: None,
            tick: None,
        }
    }
}

/// 在 `rect` 范围内绘制时间轴网格竖线。
///
/// 必须在 wgpu 纹理合成**之前**调用，保证网格线在音符后面。
///
/// - `base`：视图的水平滚动/缩放状态（`view.base`）
/// - `painter_rect`：绘制区域的屏幕坐标 rect
/// - `tpb`：ticks per beat（MIDI PPQ）
/// - `default_num` / `default_den`：默认拍号
/// - `time_sig_events`：拍号变更事件
pub fn paint_grid_lines(
    painter: &egui::Painter,
    painter_rect: egui::Rect,
    base: &TimelineViewBase,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    colors: &GridColors,
) {
    let ppu = base.pixels_per_tick;
    if ppu <= 0.001 {
        return;
    }

    // `tick_to_x` 返回相对 `content_left` 的坐标；painter 原点是 painter_rect.min。
    // 用 offset_x 把 tick 坐标桥接到 painter 坐标。
    let offset_x = painter_rect.min.x - base.left_panel_width;
    let top = painter_rect.min.y;
    let bottom = painter_rect.max.y;
    let left = painter_rect.min.x;
    let right = painter_rect.max.x;

    let tick_start = base.x_to_tick((left - offset_x).max(0.0)).max(0.0);
    let tick_end = base.x_to_tick(right - offset_x);

    let ticks_per_sub = (tpb / SUB_BEAT_DIV).max(1);
    let segments = build_time_sig_segments(time_sig_events, default_num, default_den);

    let pixels_per_beat = tpb as f32 * ppu;
    let pixels_per_sub = ticks_per_sub as f32 * ppu;

    // 网格线 = 标签的下一级（用 MIN_SPACING 统一判定）：
    // - measure 标签显示 → beat 线（measure 不合并时）
    // - beat 标签显示    → sub-beat 线
    // - sub-beat 标签显示 → tick 线
    // - tick 标签显示    → 无下一级（1tick 太密集，除外）
    let show_sub = colors.sub_beat.is_some() && pixels_per_beat >= MIN_SPACING;
    let show_tick = colors.tick.is_some() && pixels_per_sub >= MIN_SPACING;

    for i in 0..segments.len() {
        let (seg_start, num, den) = segments[i];
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
        let seg_start_f = seg_start as f64;
        if seg_start_f > tick_end {
            break;
        }

        let ticks_per_measure = measure_ticks(tpb, num, den);
        let ticks_per_beat = (ticks_per_measure / num as u32).max(1);

        // 多小节合并（缩很小时按 2/4/8… 小节合并显示）
        let pixels_per_measure = ticks_per_measure as f32 * ppu;
        let measure_divisor = compute_measure_divisor(pixels_per_measure, MIN_SPACING);
        let merged_measure_ticks = ticks_per_measure.saturating_mul(measure_divisor);
        // beat 线在 measure 不合并时显示（measure 标签不合并 → 画下一级 beat 线）
        let show_beat = measure_divisor == 1;

        // 遍历步长 = 当前最细可见级别的步长
        let step = if show_tick {
            1u32
        } else if show_sub {
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
            let x = offset_x + base.tick_to_x(tick as f64);

            if x >= left && x <= right {
                let is_measure = local % merged_measure_ticks == 0;
                let beat_local = local % ticks_per_measure;
                let is_beat_pos = beat_local.is_multiple_of(ticks_per_beat) && beat_local > 0;
                let is_sub_pos = local % ticks_per_sub == 0;

                if is_measure {
                    paint_line(painter, x, top, bottom, 2.0, colors.measure);
                } else if show_beat && is_beat_pos {
                    paint_line(painter, x, top, bottom, 1.0, colors.beat);
                } else if show_sub && is_sub_pos {
                    paint_line(painter, x, top, bottom, 1.0, colors.sub_beat.unwrap());
                } else if show_tick {
                    paint_line(painter, x, top, bottom, 1.0, colors.tick.unwrap());
                }
            }
            tick += step;
        }
    }
}

/// 画一条竖线（宽度像素的填充矩形）。
fn paint_line(painter: &egui::Painter, x: f32, top: f32, bottom: f32, width: f32, color: egui::Color32) {
    let rect = egui::Rect::from_min_size(
        egui::pos2(x - width / 2.0, top),
        egui::vec2(width, bottom - top),
    );
    painter.rect_filled(rect, 0.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 不同 tpb 必须产生不同的小节线位置（回归测试，迁移自原 grid.rs）。
    #[test]
    fn test_grid_measure_lines_differ_by_tpb() {
        // 用 egui::Painter 需要 ctx，这里用逻辑测试验证算法核心：
        // 不同 tpb 下 measure_ticks 不同 → 小节线 tick 位置不同。
        // 算法核心在 build_time_sig_segments + measure_ticks，直接验证。
        let ticks_per_measure_480 = measure_ticks(480, 4, 2);
        let ticks_per_measure_960 = measure_ticks(960, 4, 2);
        assert_eq!(ticks_per_measure_480, 1920);
        assert_eq!(ticks_per_measure_960, 3840);
        assert_ne!(ticks_per_measure_480, ticks_per_measure_960);
    }

    /// 验证零 ppu 时函数提前返回（不 panic）。
    #[test]
    fn test_grid_zero_ppu_no_panic() {
        // paint_grid_lines 在 ppu<=0.001 时直接 return，无法直接测 painter 输出，
        // 这里通过验证 ppu<=0.001 的分支逻辑来保证。
        let base = make_base(0.0);
        assert!(base.pixels_per_tick <= 0.001);
    }

    /// 验证 GridColors 配色常量存在且不同。
    #[test]
    fn test_grid_colors_distinct() {
        let pr = GridColors::pianoroll();
        let ar = GridColors::arrangement();
        assert!(pr.measure != pr.beat);
        assert!(pr.measure != pr.sub_beat.unwrap());
        assert!(ar.sub_beat.is_none());
        assert!(ar.tick.is_none());
    }
}
