//! Pianoroll 背景绘制：调式色块 + 八度分隔线。
//!
//! 与 `widgets/grid_lines` 的职责区分：
//! - `grid_lines` 画**竖向时间轴网格**（小节/拍/sub-beat），pianoroll 和 arrangement 共用
//! - 本模块画**横向 key 轴背景**（调内/调外/根音条带 + 八度分隔线），pianoroll 专属
//! - 纵向（瀑布流）视角下，key 轴沿 x 排布、时间轴沿 y 排布，色带/八度线随之转置

use eframe::egui;

use yinhe_types::{KeySigEvent, Orientation, PianoRollView};

/// 绘制调式背景 + 八度分隔线。
///
/// 横向（默认）：按可见 tick 范围沿 x 轴分段渲染，key 行沿 y 轴：
/// - 第一个调号事件**之前**的区间：无调号模式（黑键行色带，无根音高亮）
/// - 每个调号事件生效区间：调内/调外音统一用黑白键条纹（黑键行条纹色、白键行背景色），
///   仅根音行用 `PR_ROOT_NOTE` 深蓝高亮
/// - 工程无任何调号事件：全部走无调号模式
/// - 八度线 = C 顶部横线
///
/// 纵向（瀑布流）：时间轴沿 y（tick0 在顶部、向下增大），key 列沿 x（key0 最左）；
/// 分段/条纹/根音逻辑与横向一致，仅坐标转置，八度线 = C 左缘竖线。
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
    match view.orientation() {
        Orientation::Horizontal => paint_horizontal(
            painter,
            content_rect,
            kb_w,
            kh,
            view,
            key_sig_events,
            content_opacity,
        ),
        Orientation::Vertical => paint_vertical(
            painter,
            content_rect,
            kb_w,
            kh,
            view,
            key_sig_events,
            content_opacity,
        ),
    }
}

/// 横向（默认）背景：保留原逐像素公式，content_rect 含键盘列，时间轴 x 起点 = `content_rect.min.x + kb_w`。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn paint_horizontal(
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

/// 按调号区间渲染 piano roll 背景条带（横向）。
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

/// 无调号时的标准钢琴布局：画黑键行色带（无根音蓝色）（横向）。
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

/// 绘制八度横线（每个 C 位置一条细线）（横向）。
fn paint_octave_lines(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
) {
    let octave_line = crate::theme::line_fg();
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

// ── 纵向（瀑布流）：时间轴沿 y（tick0 顶部、向下增大），音高沿 x（key0 左、key127 右）──

/// 纵向背景主体：content_rect 即音乐区（无键盘列）。
/// 可见 key 列与可见 tick 区间只求一次，供无调号段与调号分段共用。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn paint_vertical(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    _kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    key_sig_events: &[KeySigEvent],
    content_opacity: f32,
) {
    let (key_lo, key_hi) = view.visible_cross_range(content_rect.width());
    let (tick_start, tick_end) = view.visible_main_range(content_rect.height());
    let tick_start = tick_start.max(0.0);
    let tick_end = tick_end.max(tick_start);

    match key_sig_events.first() {
        None => {
            paint_black_key_rows_v(
                painter,
                content_rect,
                kh,
                view,
                key_lo,
                key_hi,
                tick_start,
                tick_end,
            );
        }
        Some(first) => {
            let first_tick = first.tick as f64;
            // 第一个调号事件之前的区间：无调号模式
            if tick_start < first_tick {
                paint_black_key_rows_v(
                    painter,
                    content_rect,
                    kh,
                    view,
                    key_lo,
                    key_hi,
                    tick_start,
                    first_tick.min(tick_end),
                );
            }
            // 第一个调号事件起：调式分段
            if tick_end > first_tick {
                paint_scale_background_v(
                    painter,
                    content_rect,
                    kh,
                    view,
                    key_lo,
                    key_hi,
                    key_sig_events,
                    tick_start,
                    tick_end,
                    content_opacity,
                );
            }
        }
    }
    paint_octave_lines_v(painter, content_rect, view);
}

/// 按调号区间渲染纵向色带：每个可见 key 是竖列（x，宽 kh），tick 区间沿 y（高度为主轴像素差）。
///
/// 调用方保证 `key_sig_events` 非空且按 tick 有序。段起点从 `max(tick_start, first.tick)` 起算，
/// 之前的区间由调用方用无调号模式绘制。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn paint_scale_background_v(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kh: f32,
    view: &PianoRollView,
    key_lo: u8,
    key_hi: u8,
    key_sig_events: &[KeySigEvent],
    tick_start: f64,
    tick_end: f64,
    content_opacity: f32,
) {
    let bk_color = crate::theme::stripe_bg();
    let root_color = crate::theme::selected_bg().gamma_multiply(content_opacity);

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
            // 区间 y 范围（clamp 到 content 区域）
            let y_start =
                (content_rect.min.y + view.tick_to_main_px(seg_start)).max(content_rect.min.y);
            let y_end =
                (content_rect.min.y + view.tick_to_main_px(seg_end)).min(content_rect.max.y);
            let seg_h = y_end - y_start;

            if seg_h > 0.0 {
                for key in key_lo..=key_hi {
                    let pc = key % 12;
                    let color = if pc == root {
                        root_color // 根音高亮
                    } else if yinhe_types::is_black_key(key) {
                        bk_color // 黑键列条纹（调内/调外统一）
                    } else {
                        continue; // 白键列 = 背景色
                    };
                    let x = content_rect.min.x + view.key_to_cross_px(key);
                    if x + kh < content_rect.min.x || x > content_rect.max.x {
                        continue;
                    }
                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::pos2(x, y_start), egui::vec2(kh, seg_h)),
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

/// 无调号时的纵向黑键列色带（无根音蓝色）。
///
/// 仅画 `[seg_start, seg_end)` tick 区间对应的 y 范围（clamp 到可见区域）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn paint_black_key_rows_v(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kh: f32,
    view: &PianoRollView,
    key_lo: u8,
    key_hi: u8,
    seg_start: f64,
    seg_end: f64,
) {
    if seg_end <= seg_start {
        return;
    }
    // 区间 y 范围（clamp 到 content 区域）
    let y_start = (content_rect.min.y + view.tick_to_main_px(seg_start)).max(content_rect.min.y);
    let y_end = (content_rect.min.y + view.tick_to_main_px(seg_end)).min(content_rect.max.y);
    let seg_h = y_end - y_start;
    if seg_h <= 0.0 {
        return;
    }

    let bk_color = crate::theme::stripe_bg();
    for key in key_lo..=key_hi {
        if !yinhe_types::is_black_key(key) {
            continue;
        }
        let x = content_rect.min.x + view.key_to_cross_px(key);
        if x + kh < content_rect.min.x || x > content_rect.max.x {
            continue;
        }
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, y_start), egui::vec2(kh, seg_h)),
            0.0,
            bk_color,
        );
    }
}

/// 绘制八度竖线（每个 C 左缘一条细线，从顶部画到底部）（纵向）。
fn paint_octave_lines_v(painter: &egui::Painter, content_rect: egui::Rect, view: &PianoRollView) {
    let octave_line = crate::theme::line_fg();
    for key in (0u8..128).step_by(12) {
        let x = content_rect.min.x + view.key_to_cross_px(key); // C 的左缘
        if x < content_rect.min.x || x > content_rect.max.x {
            continue;
        }
        painter.line_segment(
            [
                egui::pos2(x, content_rect.min.y),
                egui::pos2(x, content_rect.max.y),
            ],
            egui::Stroke::new(1.0, octave_line),
        );
    }
}
