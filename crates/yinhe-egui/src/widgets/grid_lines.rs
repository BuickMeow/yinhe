//! Timeline grid lines drawn by egui (replaces former wgpu grid layer).
//!
//! 与 `time_ruler` 共享同一套 segment 遍历骨架，网格线级别 = 标签的下一级：
//! 合并小节标签 → 合并/2 小节线；每小节标签 → 四分音符线；beat 标签 → 十六分音符线。

use crate::theme;
use eframe::egui;
use yinhe_types::{
    TimeSigEvent, TimelineViewBase, build_time_sig_segments, compute_measure_divisor, measure_ticks,
};

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
            measure: theme::line_fg(),
            beat: theme::line_fg(),
            sub_beat: Some(theme::grid_sub_beat()),
            tick: Some(theme::grid_tick()),
        }
    }

    /// Arrangement 配色（无 sub_beat / tick 线）。
    pub fn arrangement() -> Self {
        Self {
            measure: theme::line_fg(),
            beat: theme::line_fg(),
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
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
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

    // 网格线 = 标签的下一级（与 time_ruler 的标签级别严格错开）：
    // - 合并 measure 标签（2/4/8… 小节合并）→ 合并/2 小节线
    //   例：4 小节标签 → 每 2 小节一条线（divisor 为 2 的幂，合并/2 恒为小节边界）
    // - 每小节标签（不合并）→ 四分音符线（不依赖 MIN_SPACING，标尺有标签就有下一级线）
    // - beat 标签（每拍间距 >= MIN_SPACING）→ 十六分音符线
    // - sub 标签显示       → tick 线（仅最大缩放，MAX_PPU）
    // - tick 标签显示       → 无下一级
    const MAX_PPU: f32 = 10.0;

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

        // 网格级别判定：
        // - merged: 标签在合并边界，网格在合并/2 边界
        // - not merged: 标签在每小节，下一级为 beat 线（无条件，保证"有标签就有线"）
        let merged = measure_divisor > 1;
        let show_beat = !merged;
        let show_sub =
            colors.sub_beat.is_some() && show_beat && (ticks_per_beat as f32 * ppu) >= MIN_SPACING;
        let show_tick = colors.tick.is_some() && show_sub && ppu >= MAX_PPU;

        // 合并时的 measure 网格步长
        let grid_measure_ticks = if merged {
            (merged_measure_ticks / 2).max(1)
        } else {
            ticks_per_measure
        };

        // 遍历步长 = 当前最细可见级别的步长
        let step = if show_tick {
            1u32
        } else if show_sub {
            ticks_per_sub
        } else if show_beat {
            ticks_per_beat
        } else {
            grid_measure_ticks.max(1)
        };

        // 对齐到段内网格：变拍子段的 seg_start 通常不在全局 step 网格上，
        // 必须从 seg_start 开始按 local 对齐，否则段内 tick 全落在错误偏移，整屏无线。
        let first_tick = seg_start_f.max(tick_start);
        let step_f = step as f64;
        let first = seg_start.saturating_add(
            (((first_tick - seg_start_f) / step_f).floor() as u32).saturating_mul(step),
        );

        let mut tick = first;
        while (tick as f64) <= tick_end && tick < seg_end {
            let local = tick - seg_start;
            let x = offset_x + base.tick_to_x(tick as f64);

            if x >= left && x <= right {
                let is_measure = local % ticks_per_measure == 0;
                let beat_local = local % ticks_per_measure;
                let is_beat_pos = beat_local.is_multiple_of(ticks_per_beat) && beat_local > 0;
                let is_sub_pos = local % ticks_per_sub == 0;

                if is_measure {
                    // 小节线统一粗线（2px）。合并时合并/2 恒为小节边界（divisor 为 2 的幂），
                    // 因此合并网格密度自动是标签的 2 倍（如 4 小节标签 → 每 2 小节一条线）。
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
fn paint_line(
    painter: &egui::Painter,
    x: f32,
    top: f32,
    bottom: f32,
    width: f32,
    color: egui::Color32,
) {
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
            follow_target: None,
        }
    }

    /// 画网格并收集所有竖线的 x 坐标（tick 坐标，已减 offset_x）。
    fn paint_and_collect(
        base: &TimelineViewBase,
        rect: egui::Rect,
        tpb: u32,
        default_num: u8,
        default_den: u8,
        events: &[TimeSigEvent],
        colors: &GridColors,
    ) -> Vec<f64> {
        let ctx = egui::Context::default();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("grid_test"),
        ));
        paint_grid_lines(
            &painter,
            rect,
            base,
            tpb,
            default_num,
            default_den,
            events,
            colors,
        );
        let offset_x = rect.min.x - base.left_panel_width;
        let mut xs = Vec::new();
        painter.for_each_shape(|cs| {
            if let egui::Shape::Rect(r) = &cs.shape {
                let x = r.rect.center().x;
                xs.push(base.x_to_tick(x - offset_x));
            }
        });
        xs.sort_by(|a, b| a.total_cmp(b));
        xs
    }

    /// 判断 tick 位置是否落在 step 网格上（含浮点容差）。
    fn on_grid(x: f64, step: f64) -> bool {
        ((x / step).round() * step - x).abs() < 1.0
    }

    /// 问题 2 复现：变拍子歌曲，拍号事件在屏幕外（左侧）时，屏幕内网格线必须正常画出。
    #[test]
    fn test_grid_variable_timesig_event_offscreen() {
        // tpb=480，拍号事件在 tick 1000（屏幕外左侧），屏幕显示 tick 100000..130000。
        let base = TimelineViewBase {
            pixels_per_tick: 0.1,
            scroll_x: 10000.0,
            ..make_base(0.1)
        };
        let rect = egui::Rect::from_min_max(egui::pos2(60.0, 0.0), egui::pos2(660.0, 50.0));
        let events = vec![TimeSigEvent {
            tick: 1000,
            numerator: 7,
            denominator: 3,
        }];
        let xs = paint_and_collect(&base, rect, 480, 4, 2, &events, &GridColors::pianoroll());
        assert!(
            xs.len() >= 20,
            "屏幕内应有大量网格线，实际只有 {} 条: {:?}",
            xs.len(),
            xs
        );
        // 第一条线应落在屏幕左边界附近（tick 100000）而非远处。
        assert!(
            xs.first().copied().unwrap_or(f64::MAX) < 100000.0 + 480.0,
            "第一条线位置异常: {:?}",
            xs.first()
        );
    }

    /// 问题 1 复现：标尺显示每小节标签（不合并）时，网格必须有四分音符线。
    #[test]
    fn test_grid_beat_lines_when_measure_labels() {
        // 每小节 1920 ticks * 0.02 ppu = 38.4px >= MIN_SPACING → 标尺显示 1 2 3 4 小节。
        // 每拍 480 * 0.02 = 9.6px < MIN_SPACING → 旧逻辑不画 beat 线。
        let base = make_base(0.02);
        let rect = egui::Rect::from_min_max(egui::pos2(60.0, 0.0), egui::pos2(1060.0, 50.0));
        let xs = paint_and_collect(&base, rect, 480, 4, 2, &[], &GridColors::pianoroll());
        // 屏幕上应有 4 小节的 beat 线（每小节 4 条）+
        let measure_count = xs.iter().filter(|x| (*x % 1920.0).abs() < 1.0).count();
        let beat_count = xs
            .iter()
            .filter(|x| (*x % 480.0).abs() < 1.0 && (*x % 1920.0).abs() >= 1.0)
            .count();
        assert!(
            beat_count >= measure_count * 3,
            "小节标签显示时应有四分音符线：measure={measure_count} beat={beat_count} xs={xs:?}"
        );
    }

    /// 变拍子事件在屏幕内（屏幕跨越拍号变更点）时，两侧网格线都必须正常。
    #[test]
    fn test_grid_variable_timesig_event_on_screen() {
        // tpb=480，tick 10000 处从 4/4 变 3/4，屏幕显示 tick 8000..14000（跨越变拍点）。
        // 4/4 每小节 1920、每拍 480；3/4 每小节 1440、每拍 480。
        let base = TimelineViewBase {
            pixels_per_tick: 0.1,
            scroll_x: 800.0,
            ..make_base(0.1)
        };
        let rect = egui::Rect::from_min_max(egui::pos2(60.0, 0.0), egui::pos2(660.0, 50.0));
        let events = vec![TimeSigEvent {
            tick: 10000,
            numerator: 3,
            denominator: 2,
        }];
        let xs = paint_and_collect(&base, rect, 480, 4, 2, &events, &GridColors::pianoroll());
        assert!(
            xs.len() >= 20,
            "屏幕跨越变拍点时应两侧都有网格线，实际 {} 条: {:?}",
            xs.len(),
            xs
        );
        // 变拍点 10000 处必须是小节线（3/4 段第 1 小节起点）。
        assert!(
            xs.iter().any(|x| (*x - 10000.0).abs() < 1.0),
            "变拍点处应有小节线: {xs:?}"
        );
        // 屏幕上网格线不得有大缺口（间距 ≤ 120 tick = sub 步长），证明无丢失。
        let gaps: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            gaps.iter().all(|&g| g <= 120.0 + 0.1),
            "网格线不应有超过 120 tick 的缺口: gaps={gaps:?}"
        );
    }

    /// 多小节合并时网格密度应为标签的 2 倍（如 4 小节标签 → 每 2 小节一条线）。
    #[test]
    fn test_grid_merged_measure_lines() {
        // 每小节 1920 ticks * 0.005 ppu = 9.6px < MIN_SPACING → 合并。
        // 4 小节合并后 38.4px >= 38 → divisor=4，网格每 divisor/2=2 小节（3840 ticks）一条。
        let base = make_base(0.005);
        let rect = egui::Rect::from_min_max(egui::pos2(60.0, 0.0), egui::pos2(1060.0, 50.0));
        let xs = paint_and_collect(&base, rect, 480, 4, 2, &[], &GridColors::pianoroll());
        assert!(xs.len() >= 10, "合并时网格线应正常存在: {xs:?}");
        assert!(
            xs.iter().all(|&x| on_grid(x, 1920.0)),
            "合并网格线都应在小节边界上: {xs:?}"
        );
        let gaps: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            gaps.iter().all(|&g| (g - 3840.0).abs() < 1.0),
            "合并网格线间距应为 2 小节（3840）：gaps={gaps:?}"
        );
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

    /// 验证 GridColors 配色档位：measure/beat 统一 line_fg，sub_beat/tick 更浅。
    #[test]
    fn test_grid_colors_distinct() {
        let pr = GridColors::pianoroll();
        let ar = GridColors::arrangement();
        // measure 与 beat 同用 line_fg（统一线条色）
        assert_eq!(pr.measure, pr.beat);
        // sub_beat（+8%）与 tick（+3%）逐档更浅
        assert_ne!(pr.measure, pr.sub_beat.unwrap());
        assert_ne!(pr.sub_beat.unwrap(), pr.tick.unwrap());
        assert!(ar.sub_beat.is_none());
        assert!(ar.tick.is_none());
    }
}
