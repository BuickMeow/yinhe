//! Pianoroll 左侧键盘绘制（白键/黑键 + C 音名标注）。
//!
//! 与 `bg` 的职责区分：`bg` 画 music 区背景，本模块画左侧 keyboard 列。

use eframe::egui;

use yinhe_theme::GpuTheme;

/// 绘制 pianoroll 左侧键盘（白键在下层，黑键在上层，C 位置标注音名）。
///
/// - `content_rect`：pianoroll 内容区（含键盘列）
/// - `kb_w`：键盘宽度
/// - `kh`：单键高度
/// - `bottom`：键盘底部 y（128*kh - scroll_y）
/// - `h_f32`：内容区高度（用于 cull）
pub fn paint(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    kb_w: f32,
    kh: f32,
    bottom: f32,
    h_f32: f32,
    theme: &GpuTheme,
) {
    let stroke_color = crate::theme::rgb_to_color32(theme.key_black);
    // 描边宽度随 y 轴缩放（键高）等比：kh=12（默认）时 1px
    let stroke = egui::Stroke::new((kh * 0.0833).clamp(0.5, 2.0), stroke_color);

    // White keys
    for key in 0u8..128 {
        if yinhe_types::is_black_key(key) {
            continue;
        }
        let y = bottom - (key as f32 + 1.0) * kh;
        if y + kh < 0.0 || y > h_f32 {
            continue;
        }
        let screen_y = content_rect.min.y + y;
        let key_rect = egui::Rect::from_min_size(
            egui::pos2(content_rect.min.x, screen_y),
            egui::vec2(kb_w, kh),
        );
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_white));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);

        // C 位置标注音名（中央 C = C4 = key 60）
        if key % 12 == 0 {
            let octave = key / 12;
            let label = format!("C{}", octave);
            painter.text(
                egui::pos2(content_rect.min.x + 3.0, screen_y + kh / 2.0),
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
        let y = bottom - (key as f32 + 1.0) * kh;
        if y + kh < 0.0 || y > h_f32 {
            continue;
        }
        let screen_y = content_rect.min.y + y;
        let key_rect = egui::Rect::from_min_size(
            egui::pos2(content_rect.min.x, screen_y),
            egui::vec2(kb_w, kh),
        );
        painter.rect_filled(key_rect, 0.0, crate::theme::rgb_to_color32(theme.key_black));
        painter.rect_stroke(key_rect, 0.0, stroke, egui::StrokeKind::Inside);
    }
}
