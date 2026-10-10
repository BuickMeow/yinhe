//! Pianoroll 键盘绘制（白键/黑键 + C 音名标注）。
//!
//! 与 `bg` 的职责区分：`bg` 画 music 区背景，本模块画键盘条。
//! - 横向（默认）：键盘在左列，键沿 y 轴排布（key127 顶、key0 底）
//! - 纵向（瀑布流）：键盘在底部横条，键沿 x 轴排布（key0 最左、key127 最右）

use eframe::egui;

use yinhe_theme::GpuTheme;
use yinhe_types::PianoRollView;

/// 绘制 pianoroll 键盘条（白键在下层，黑键在上层，C 位置标注音名）。
///
/// - `keyboard_rect`：键盘条矩形的屏幕坐标。横向 = 左列（宽 `kb_w`），纵向 = 底部横条（高 `kb_w`）。
/// - `kb_w`：键盘厚度（横向 = 宽，纵向 = 高）
/// - `kh`：单键尺寸（横向 = 键高，纵向 = 键宽）
/// - 键位置统一用 `view.key_to_cross_px(key)`（相对键盘条起点）计算。
pub fn paint(
    painter: &egui::Painter,
    keyboard_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    theme: &GpuTheme,
) {
    if view.is_vertical() {
        paint_vertical(painter, keyboard_rect, kb_w, kh, view, theme);
    } else {
        paint_horizontal(painter, keyboard_rect, kb_w, kh, view, theme);
    }
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

/// 横向键盘（左列）：键 y = `keyboard_rect.min.y + key_to_cross_px(key)`。
///
/// 数学上与旧公式一致：`key_to_cross_px`（横向）= `key_to_y` = `128*kh - scroll_y - (key+1)*kh`
/// = 原 `bottom - (key+1)*kh`。
fn paint_horizontal(
    painter: &egui::Painter,
    keyboard_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    view: &PianoRollView,
    theme: &GpuTheme,
) {
    let stroke_color = crate::theme::rgb_to_color32(theme.key_black);
    // 描边宽度随键高等比：kh=12（默认）时 1px
    let stroke = egui::Stroke::new((kh * 0.0833).clamp(0.5, 2.0), stroke_color);

    // White keys
    for key in 0u8..128 {
        if yinhe_types::is_black_key(key) {
            continue;
        }
        let screen_y = keyboard_rect.min.y + view.key_to_cross_px(key);
        if screen_y + kh < keyboard_rect.min.y || screen_y > keyboard_rect.max.y {
            continue;
        }
        let key_rect = egui::Rect::from_min_size(
            egui::pos2(keyboard_rect.min.x, screen_y),
            egui::vec2(kb_w, kh),
        );
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_white));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);

        // C 位置标注音名（中央 C = C4 = key 60），横排在左
        if key % 12 == 0 {
            let octave = key as i32 / 12 - 1;
            let label = format!("C{}", octave);
            painter.text(
                egui::pos2(keyboard_rect.min.x + 3.0, screen_y + kh / 2.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional((kh * 0.5).clamp(8.0, 14.0)),
                crate::theme::text_disabled(),
            );
        }
    }

    // Black keys on top
    for key in 0u8..128 {
        if !yinhe_types::is_black_key(key) {
            continue;
        }
        let screen_y = keyboard_rect.min.y + view.key_to_cross_px(key);
        if screen_y + kh < keyboard_rect.min.y || screen_y > keyboard_rect.max.y {
            continue;
        }
        let key_rect = egui::Rect::from_min_size(
            egui::pos2(keyboard_rect.min.x, screen_y),
            egui::vec2(kb_w, kh),
        );
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_black));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);
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
) {
    let stroke_color = crate::theme::rgb_to_color32(theme.key_black);
    // 描边宽度随键宽（=kh）等比：kh=12（默认）时 1px
    let stroke = egui::Stroke::new((kh * 0.0833).clamp(0.5, 2.0), stroke_color);

    // White keys
    for key in 0u8..128 {
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
        if key % 12 == 0 {
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
    for key in 0u8..128 {
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
}
