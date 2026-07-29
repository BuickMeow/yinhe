//! Pianoroll 背景绘制：调式色块 + 八度横线。
//!
//! 与 `widgets/grid_lines` 的职责区分：
//! - `grid_lines` 画**竖向时间轴网格**（小节/拍/sub-beat），pianoroll 和 arrangement 共用
//! - 本模块画**横向 key 轴背景**（调内/调外/根音条带 + 八度分隔线），pianoroll 专属

use eframe::egui;

use yinhe_theme::GpuTheme;
use yinhe_types::{KeySigEvent, PianoRollView};

/// 绘制调式背景 + 八度横线。
///
/// 有调号事件时：调内音用背景色（不画），调外音用 `PR_SCALE_OUTSIDE` 暗色，根音用 `PR_ROOT_NOTE` 深蓝。
/// 无调号事件时：回退标准钢琴布局（黑键行用 `pr_black_key_row` 色带），不画调式色块。
pub fn paint(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    key_sig_events: &[KeySigEvent],
    theme: &GpuTheme,
) {
    paint_scale_background(painter, content_rect, kb_w, kh, view, key_sig_events, theme);
    paint_octave_lines(painter, content_rect, kb_w, kh, view);
}

/// 按调号区间渲染 piano roll 背景条带。
///
/// 无调号事件时回退标准钢琴布局（黑键行色带），不画调式色块。
fn paint_scale_background(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    key_sig_events: &[KeySigEvent],
    theme: &GpuTheme,
) {
    let content_left = content_rect.min.x + kb_w;
    let content_w = content_rect.width() - kb_w;
    let h = content_rect.height();
    let bottom = 128.0 * kh - view.base.scroll_y;

    // 可见 key 范围
    let key_lo = view.y_to_key(0.0).max(0) as u8;
    let key_hi = view.y_to_key(h).min(127) as u8;

    // 无调号事件：回退标准钢琴布局（画黑键行色带）
    if key_sig_events.is_empty() {
        let (bkr, bkg, bkb) = theme.pr_black_key_row;
        let bk_color = egui::Color32::from_rgb(
            (bkr * 255.0) as u8,
            (bkg * 255.0) as u8,
            (bkb * 255.0) as u8,
        );
        for key in key_lo..=key_hi {
            if !yinhe_types::is_black_key(key) {
                continue;
            }
            let y = bottom - (key as f32 + 1.0) * kh;
            let screen_y = content_rect.min.y + y;
            if screen_y + kh < content_rect.min.y || screen_y > content_rect.max.y {
                continue;
            }
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(content_left, screen_y),
                    egui::vec2(content_w, kh),
                ),
                0.0,
                bk_color,
            );
        }
        return;
    }

    // 有调号：按 tick 区间渲染
    let (tick_start, tick_end) = view.visible_tick_range(content_rect.width());
    let tick_start = tick_start.max(0.0);
    let tick_end = tick_end.max(tick_start);
    if tick_end <= tick_start {
        return;
    }

    let outside_color = crate::theme::PR_SCALE_OUTSIDE;
    let root_color = crate::theme::PR_ROOT_NOTE;
    let ppt = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;

    // 找到 tick_start 之前最后一个调号（当前生效的调号）
    let mut start_idx = 0usize;
    for (i, ev) in key_sig_events.iter().enumerate() {
        if (ev.tick as f64) <= tick_start {
            start_idx = i;
        } else {
            break;
        }
    }

    // 遍历可见调号区间
    let mut seg_start = tick_start;
    let mut idx = start_idx;
    loop {
        let (root, scale) = (key_sig_events[idx].root, key_sig_events[idx].scale);
        let seg_end = if idx + 1 < key_sig_events.len() {
            (key_sig_events[idx + 1].tick as f64).min(tick_end)
        } else {
            tick_end
        };

        if seg_start < tick_end && seg_end > tick_start {
            // 区间 x 范围（clamp 到 content 区域）
            let x_start = (content_left + seg_start as f32 * ppt - scroll_x).max(content_left);
            let x_end = (content_left + seg_end as f32 * ppt - scroll_x).min(content_rect.max.x);
            let seg_w = x_end - x_start;

            if seg_w > 0.0 {
                let pitch_mask = scale.pitch_classes(root);
                for key in key_lo..=key_hi {
                    let pc = key % 12;
                    let color = if pc == root {
                        root_color
                    } else if pitch_mask & (1u16 << pc) != 0 {
                        continue; // 调内音，用背景色（不画）
                    } else {
                        outside_color
                    };
                    let y = bottom - (key as f32 + 1.0) * kh;
                    let screen_y = content_rect.min.y + y;
                    if screen_y + kh < content_rect.min.y || screen_y > content_rect.max.y {
                        continue;
                    }
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(x_start, screen_y),
                            egui::vec2(seg_w, kh),
                        ),
                        0.0,
                        color,
                    );
                }
            }
        }

        if seg_end >= tick_end || idx + 1 >= key_sig_events.len() {
            break;
        }
        seg_start = seg_end;
        idx += 1;
    }
}

/// 绘制八度横线（每个 C 位置一条细线）。
fn paint_octave_lines(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
) {
    let octave_line = crate::theme::PR_OCTAVE_LINE;
    let content_left = content_rect.min.x + kb_w;
    let bottom = 128.0 * kh - view.base.scroll_y;
    for key in (0u8..128).step_by(12) {
        let y = bottom - key as f32 * kh; // C 的顶部
        let screen_y = content_rect.min.y + y;
        if screen_y < content_rect.min.y || screen_y > content_rect.max.y {
            continue;
        }
        painter.line_segment(
            [
                egui::pos2(content_left, screen_y),
                egui::pos2(content_rect.max.x, screen_y),
            ],
            egui::Stroke::new(1.0, octave_line),
        );
    }
}
