//! Pianoroll 背景绘制：调式色块 + 八度横线。
//!
//! 与 `widgets/grid_lines` 的职责区分：
//! - `grid_lines` 画**竖向时间轴网格**（小节/拍/sub-beat），pianoroll 和 arrangement 共用
//! - 本模块画**横向 key 轴背景**（调内/调外/根音条带 + 八度分隔线），pianoroll 专属

use eframe::egui;

use yinhe_types::{KeySigEvent, PianoRollView};

/// 绘制调式背景 + 八度横线。
///
/// 按可见 tick 范围分段渲染：
/// - 第一个调号事件**之前**的区间：无调号模式（黑键行色带，无根音高亮）
/// - 每个调号事件生效区间：调内/调外音统一用黑白键条纹（黑键行条纹色、白键行背景色），
///   仅根音行用 `PR_ROOT_NOTE` 深蓝高亮
/// - 工程无任何调号事件：全部走无调号模式
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn paint(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    key_sig_events: &[KeySigEvent],
    content_opacity: f32,
) {
    let (tick_start, tick_end) = view.visible_tick_range(content_rect.width());
    let tick_start = tick_start.max(0.0);
    let tick_end = tick_end.max(tick_start);

    match key_sig_events.first() {
        None => {
            paint_black_key_rows(painter, content_rect, kb_w, kh, view, tick_start, tick_end);
        }
        Some(first) => {
            let first_tick = first.tick as f64;
            // 第一个调号事件之前的区间：无调号模式
            if tick_start < first_tick {
                paint_black_key_rows(
                    painter,
                    content_rect,
                    kb_w,
                    kh,
                    view,
                    tick_start,
                    first_tick.min(tick_end),
                );
            }
            // 第一个调号事件起：调式分段
            if tick_end > first_tick {
                paint_scale_background(
                    painter,
                    content_rect,
                    kb_w,
                    kh,
                    view,
                    key_sig_events,
                    content_opacity,
                );
            }
        }
    }
    paint_octave_lines(painter, content_rect, kb_w, kh, view);
}

/// 按调号区间渲染 piano roll 背景条带。
///
/// 调用方保证 `key_sig_events` 非空且按 tick 有序（模型层 set/insert 后均 sort）。
/// 渲染从 `max(tick_start, 第一个调号事件 tick)` 开始——之前的区间由调用方用
/// 无调号模式绘制。
fn paint_scale_background(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    key_sig_events: &[KeySigEvent],
    content_opacity: f32,
) {
    let content_left = content_rect.min.x + kb_w;
    let h = content_rect.height();
    let bottom = 128.0 * kh - view.base.scroll_y;

    // 可见 key 范围：y_to_key(0.0)=顶部 key（大值），y_to_key(h)=底部 key（小值）
    let key_hi = view.y_to_key(0.0).min(127);
    let key_lo = view.y_to_key(h);

    // 按 tick 区间渲染
    let (tick_start, tick_end) = view.visible_tick_range(content_rect.width());
    let tick_start = tick_start.max(0.0);
    let tick_end = tick_end.max(tick_start);

    // 调内/调外音统一用黑白键条纹（黑键行条纹色、白键行背景色）；根音行用蓝色高亮
    let bk_color = crate::theme::stripe_bg();
    let root_color = crate::theme::selected_bg().gamma_multiply(content_opacity);
    let ppt = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;

    // 第一个调号事件之前的区间不归这里画，段起点从 first.tick 起算
    let first_tick = key_sig_events[0].tick as f64;
    let mut seg_start = tick_start.max(first_tick);

    // 找到 seg_start 之前最后一个调号（当前生效的调号）
    let mut start_idx = 0usize;
    for (i, ev) in key_sig_events.iter().enumerate() {
        if (ev.tick as f64) <= seg_start {
            start_idx = i;
        } else {
            break;
        }
    }

    // 遍历可见调号区间
    let mut idx = start_idx;
    loop {
        let root = key_sig_events[idx].root;
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
                for key in key_lo..=key_hi {
                    let pc = key % 12;
                    let color = if pc == root {
                        root_color // 根音高亮
                    } else if yinhe_types::is_black_key(key) {
                        bk_color // 黑键行条纹（调内/调外统一）
                    } else {
                        continue; // 白键行 = 背景色
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

/// 无调号时的标准钢琴布局：画黑键行色带（无根音蓝色）。
///
/// 仅画 `[seg_start, seg_end)` tick 区间对应的 x 范围（clamp 到可见区域）。
/// 用于"工程无调号"或"第一个调号事件之前"的区间。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn paint_black_key_rows(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    seg_start: f64,
    seg_end: f64,
) {
    let content_left = content_rect.min.x + kb_w;
    let h = content_rect.height();
    let bottom = 128.0 * kh - view.base.scroll_y;

    // 可见 key 范围
    let key_hi = view.y_to_key(0.0).min(127);
    let key_lo = view.y_to_key(h);

    // 区间 x 范围（clamp 到 content 区域）
    let ppt = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;
    let x_start = (content_left + seg_start as f32 * ppt - scroll_x).max(content_left);
    let x_end = (content_left + seg_end as f32 * ppt - scroll_x).min(content_rect.max.x);
    let seg_w = x_end - x_start;
    if seg_w <= 0.0 {
        return;
    }

    let bk_color = crate::theme::stripe_bg();
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
            egui::Rect::from_min_size(egui::pos2(x_start, screen_y), egui::vec2(seg_w, kh)),
            0.0,
            bk_color,
        );
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
    let octave_line = crate::theme::grid_beat();
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
