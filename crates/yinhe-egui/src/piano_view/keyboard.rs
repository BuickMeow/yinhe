//! Pianoroll 键盘绘制（白键/黑键 + C 音名标注）。
//!
//! 与 `bg` 的职责区分：`bg` 画 music 区背景，本模块画键盘条。
//! - 横向（默认）：键盘在左列，键沿 y 轴排布（key127 顶、key0 底）。
//!   键盘列再分左右两半：右半是真实琴键，左半只画 C 音名与八度线。
//! - 纵向（瀑布流）：键盘在底部横条，键沿 x 轴排布（key0 最左、key127 最右）。

use eframe::egui;

use yinhe_theme::GpuTheme;
use yinhe_types::{KEY_COUNT, PianoRollView};

/// 横向键盘里「音名栏」占键盘宽度的比例（左半）。
const LABEL_GUTTER_RATIO: f32 = 0.5;

/// 键盘上的临时高亮：正在演奏的键 + 拖选出的音域。
#[derive(Clone, Copy)]
pub struct KeyHighlights<'a> {
    pub playing: &'a [bool; KEY_COUNT],
    pub select: Option<(u8, u8)>,
}

/// 绘制 pianoroll 键盘条（白键在下层，黑键在上层，C 位置标注音名）。
///
/// - `keyboard_rect`：键盘条矩形的屏幕坐标。横向 = 左列（宽 `kb_w`），纵向 = 底部横条（高 `kb_w`）。
/// - `kb_w`：键盘厚度（横向 = 宽，纵向 = 高）
/// - `kh`：单键尺寸（横向 = 键高，纵向 = 键宽）
/// - `playing_keys`：正在演奏的键（按播放头位置点亮）
/// - `key_select`：在琴键上拖出的音域（闭区间），高亮显示
/// - 键位置统一用 `view.key_to_cross_px(key)`（相对键盘条起点）计算。
pub fn paint(
    painter: &egui::Painter,
    keyboard_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    theme: &GpuTheme,
    highlights: KeyHighlights<'_>,
) {
    if view.is_vertical() {
        paint_vertical(painter, keyboard_rect, kb_w, kh, view, theme, highlights);
    } else {
        paint_horizontal(painter, keyboard_rect, kb_w, kh, view, theme, highlights);
    }
}

/// 按播放头位置统计正在响的键（返回 256 位布尔表）。
///
/// 只扫描 `visible` 为 true 的轨道与当前可见的音高范围，避免每帧遍历全曲。
pub fn playing_keys_at(
    src: &dyn yinhe_types::NoteSource,
    tick: f64,
    visible: &[bool],
    (key_lo, key_hi): (u8, u8),
) -> [bool; KEY_COUNT] {
    let mut keys = [false; KEY_COUNT];
    if tick < 0.0 {
        return keys;
    }
    let tick = tick.floor() as u64;
    if tick > u32::MAX as u64 {
        return keys;
    }
    let tick = tick as u32;
    for key in key_lo..=key_hi {
        let bucket = src.key_notes(key);
        if bucket.is_empty() {
            continue;
        }
        for note in src.key_notes_in_range(key, tick, tick + 1) {
            if note.start_tick <= tick
                && note.end_tick > tick
                && visible.get(note.track as usize).copied().unwrap_or(true)
            {
                keys[key as usize] = true;
                break;
            }
        }
    }
    keys
}

/// 迷你钢琴卷帘（**纵向**，低音在下 / 高音在上）：左侧键盘列 + 右侧音符条。
///
/// 每个半音一行（与主钢琴卷帘键盘同为等分行）；调内音在右侧画出音符条。
/// 全部配色取自主题（键盘黑白键 = `GpuTheme`，音符条 = 强调色）。与 [`paint`]
/// 解耦（不依赖 `PianoRollView`/滚动），供音阶预览等场景复用。
///
/// `highlight_pc`：12 位 pitch-class bitmask（bit i = pc i 在调内；0 = 空）。
pub fn paint_mini(
    painter: &egui::Painter,
    rect: egui::Rect,
    lo_key: u8,
    hi_key: u8,
    highlight_pc: u16,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || hi_key < lo_key {
        return;
    }
    let theme = yinhe_theme::current_gpu_theme();
    let white = crate::theme::rgb_to_color32(theme.key_white);
    let black = crate::theme::rgb_to_color32(theme.key_black);
    let note_color = crate::theme::accent_active();
    let stroke = egui::Stroke::new(1.0, black);
    let is_black = |key: u8| yinhe_types::is_black_key(key);
    let in_scale = |key: u8| highlight_pc & (1u16 << (key % 12)) != 0;

    let kb_w = (rect.width() * 0.32).clamp(16.0, 40.0);
    let note_x0 = rect.min.x + kb_w;
    let note_area = egui::Rect::from_min_max(
        egui::pos2(note_x0, rect.min.y),
        egui::pos2(rect.max.x, rect.max.y),
    );
    painter.rect_filled(note_area, 0.0, crate::theme::control_bg());

    let rows = (hi_key - lo_key + 1) as f32;
    let row_h = rect.height() / rows;

    for key in lo_key..=hi_key {
        // 低音在下：行顶 y。
        let y = rect.max.y - (key - lo_key + 1) as f32 * row_h;
        // 键盘格。
        let kc = egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(kb_w, row_h));
        let kf = if is_black(key) { black } else { white };
        painter.rect_filled(kc, 0.0, kf);
        painter.rect_stroke(kc, 0.0, stroke, egui::StrokeKind::Inside);

        // 右侧音符条。
        if in_scale(key) {
            let bar = egui::Rect::from_min_size(
                egui::pos2(note_x0 + 2.0, y + 1.0),
                egui::vec2(rect.max.x - note_x0 - 3.0, (row_h - 2.0).max(1.0)),
            );
            painter.rect_filled(bar, 0.0, note_color);
        }
    }
}

fn key_row_y(keyboard_rect: egui::Rect, view: &PianoRollView, key: u8) -> f32 {
    keyboard_rect.min.y + view.key_to_cross_px(key)
}

fn visible_row(screen_y: f32, keyboard_rect: egui::Rect, kh: f32) -> bool {
    screen_y + kh >= keyboard_rect.min.y && screen_y <= keyboard_rect.max.y
}

/// 横向键盘（左列）：音名栏（左半）+ 琴键（右半）。
fn paint_horizontal(
    painter: &egui::Painter,
    keyboard_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    theme: &GpuTheme,
    highlights: KeyHighlights<'_>,
) {
    let stroke_color = crate::theme::rgb_to_color32(theme.key_black);
    // 描边宽度随键高等比：kh=12（默认）时 1px
    let stroke = egui::Stroke::new((kh * 0.0833).clamp(0.5, 2.0), stroke_color);

    let gutter_w = kb_w * LABEL_GUTTER_RATIO;
    let keys_x = keyboard_rect.min.x + gutter_w;
    let keys_w = (kb_w - gutter_w).max(0.0);
    let gutter = egui::Rect::from_min_max(
        egui::pos2(keyboard_rect.min.x, keyboard_rect.min.y),
        egui::pos2(keys_x, keyboard_rect.max.y),
    );
    painter.rect_filled(gutter, 0.0, crate::theme::control_bg());

    // 八度线 + C 音名（音名栏内）。
    let font = egui::FontId::proportional((kh * 0.5).clamp(8.0, 14.0));
    let line_color = crate::theme::line_fg();
    for idx in (0..KEY_COUNT).step_by(12) {
        let key = idx as u8;
        let screen_y = key_row_y(keyboard_rect, view, key);
        if !visible_row(screen_y, keyboard_rect, kh) {
            continue;
        }
        painter.hline(
            egui::Rangef::new(gutter.min.x, gutter.max.x),
            screen_y,
            egui::Stroke::new(1.0, line_color),
        );
        let octave = key as i32 / 12 - 1;
        painter.text(
            egui::pos2(gutter.center().x, screen_y + kh / 2.0),
            egui::Align2::CENTER_CENTER,
            format!("C{octave}"),
            font.clone(),
            crate::theme::text_disabled(),
        );
    }

    // White keys
    for idx in 0..KEY_COUNT {
        let key = idx as u8;
        if yinhe_types::is_black_key(key) {
            continue;
        }
        let screen_y = key_row_y(keyboard_rect, view, key);
        if !visible_row(screen_y, keyboard_rect, kh) {
            continue;
        }
        let key_rect =
            egui::Rect::from_min_size(egui::pos2(keys_x, screen_y), egui::vec2(keys_w, kh));
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_white));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);
    }

    // Black keys on top
    for idx in 0..KEY_COUNT {
        let key = idx as u8;
        if !yinhe_types::is_black_key(key) {
            continue;
        }
        let screen_y = key_row_y(keyboard_rect, view, key);
        if !visible_row(screen_y, keyboard_rect, kh) {
            continue;
        }
        let key_rect =
            egui::Rect::from_min_size(egui::pos2(keys_x, screen_y), egui::vec2(keys_w, kh));
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_black));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);
    }

    // 音域 / 演奏高亮（盖在黑白键之上）。
    let sel_bg = crate::theme::selected_bg();
    let accent = crate::theme::accent_active();
    for (idx, &playing) in highlights.playing.iter().enumerate() {
        let key = idx as u8;
        let screen_y = key_row_y(keyboard_rect, view, key);
        if !visible_row(screen_y, keyboard_rect, kh) {
            continue;
        }
        let key_rect =
            egui::Rect::from_min_size(egui::pos2(keys_x, screen_y), egui::vec2(keys_w, kh));
        let in_range = highlights
            .select
            .is_some_and(|(lo, hi)| key >= lo && key <= hi);
        if in_range {
            painter.rect_filled(key_rect, 0.0, sel_bg);
        }
        if playing {
            painter.rect_filled(key_rect, 0.0, accent.gamma_multiply(0.85));
        }
    }
}

/// 纵向键盘（底部横条）：键 x = `keyboard_rect.min.x + key_to_cross_px(key)`，键宽 `kh`、键长 `kb_w`。
///
/// 音名沿键中心旋转竖排（-90°），key0 最左 → key127 最右。
fn paint_vertical(
    painter: &egui::Painter,
    keyboard_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    theme: &GpuTheme,
    highlights: KeyHighlights<'_>,
) {
    let stroke_color = crate::theme::rgb_to_color32(theme.key_black);
    // 描边宽度随键宽（=kh）等比：kh=12（默认）时 1px
    let stroke = egui::Stroke::new((kh * 0.0833).clamp(0.5, 2.0), stroke_color);

    // White keys
    for idx in 0..KEY_COUNT {
        let key = idx as u8;
        if yinhe_types::is_black_key(key) {
            continue;
        }
        let x = keyboard_rect.min.x + view.key_to_cross_px(key);
        if x + kh < keyboard_rect.min.x || x > keyboard_rect.max.x {
            continue;
        }
        let key_rect =
            egui::Rect::from_min_size(egui::pos2(x, keyboard_rect.min.y), egui::vec2(kh, kb_w));
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_white));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);

        // C 位置标注音名（中央 C = C4 = key 60），竖排标注
        if key.is_multiple_of(12) {
            let octave = key as i32 / 12 - 1;
            let label = format!("C{}", octave);
            let font_id = egui::FontId::proportional((kh * 0.5).clamp(8.0, 14.0));
            let color = crate::theme::text_disabled();
            let galley = painter.layout_no_wrap(label, font_id, color);
            let pos = egui::pos2(x + kh / 2.0, keyboard_rect.min.y + kb_w / 2.0);
            painter.add(
                egui::epaint::TextShape::new(pos, galley, color).with_angle_and_anchor(
                    -std::f32::consts::FRAC_PI_2,
                    egui::Align2::CENTER_CENTER,
                ),
            );
        }
    }

    // Black keys on top
    for idx in 0..KEY_COUNT {
        let key = idx as u8;
        if !yinhe_types::is_black_key(key) {
            continue;
        }
        let x = keyboard_rect.min.x + view.key_to_cross_px(key);
        if x + kh < keyboard_rect.min.x || x > keyboard_rect.max.x {
            continue;
        }
        let key_rect =
            egui::Rect::from_min_size(egui::pos2(x, keyboard_rect.min.y), egui::vec2(kh, kb_w));
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_black));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);
    }

    // 音域 / 演奏高亮。
    let sel_bg = crate::theme::selected_bg();
    let accent = crate::theme::accent_active();
    for (idx, &playing) in highlights.playing.iter().enumerate() {
        let key = idx as u8;
        let x = keyboard_rect.min.x + view.key_to_cross_px(key);
        if x + kh < keyboard_rect.min.x || x > keyboard_rect.max.x {
            continue;
        }
        let key_rect =
            egui::Rect::from_min_size(egui::pos2(x, keyboard_rect.min.y), egui::vec2(kh, kb_w));
        let in_range = highlights
            .select
            .is_some_and(|(lo, hi)| key >= lo && key <= hi);
        if in_range {
            painter.rect_filled(key_rect, 0.0, sel_bg);
        }
        if playing {
            painter.rect_filled(key_rect, 0.0, accent.gamma_multiply(0.85));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 播放头落在音符内 → 该键点亮；换到下一个音符后只点亮后者。
    #[test]
    fn playing_keys_follow_playhead() {
        let midi =
            yinhe_test_helpers::make_midi(vec![(60, 0, 480, 0, 100), (64, 480, 960, 1, 100)]);
        let keys = playing_keys_at(&midi, 100.0, &[true, true], (0, 127));
        assert!(keys[60]);
        assert!(!keys[64]);

        let keys = playing_keys_at(&midi, 480.0, &[true, true], (0, 127));
        assert!(!keys[60]);
        assert!(keys[64]);
    }

    /// 隐藏轨道的音符不点亮，且可见音高范围外不扫描。
    #[test]
    fn playing_keys_respect_visibility_and_range() {
        let midi = yinhe_test_helpers::make_midi(vec![(64, 480, 960, 1, 100)]);
        let hidden = playing_keys_at(&midi, 500.0, &[true, false], (0, 127));
        assert!(!hidden[64]);
        let out_of_range = playing_keys_at(&midi, 500.0, &[true, true], (0, 60));
        assert!(!out_of_range[64]);
    }
}
