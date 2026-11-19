//! 拖拽/悬停提示框：屏幕坐标右上方绘制多行文本。
//!
//! 各工具只负责拼接要显示的行内容，绘制和主题色统一在这里处理。

use eframe::egui;

/// 在屏幕坐标 `(x, y)` 右上方绘制多行悬浮提示。
///
/// 提示框位于鼠标右上方（水平偏移 24px、上移 24px），避免框体盖住指针。
pub(crate) fn draw_hover_tooltip(ctx: &egui::Context, lines: &[String], x: f32, y: f32) {
    if lines.is_empty() {
        return;
    }
    let painter = ctx.debug_painter();
    let font_id = egui::FontId::proportional(crate::theme::TOOLTIP_FONT);
    let gap = 24.0;
    let tooltip_x = x + gap;
    let tooltip_y = y - 24.0;
    let mut max_w = 0.0_f32;
    let line_h = 16.0;
    for line in lines {
        let galley =
            painter.layout_no_wrap(line.clone(), font_id.clone(), crate::theme::contrast_fg());
        max_w = max_w.max(galley.rect.width());
    }
    let total_h = line_h * lines.len() as f32;
    let pad = 6.0;
    let bg_rect = egui::Rect::from_min_size(
        egui::pos2(tooltip_x - pad, tooltip_y - pad),
        egui::vec2(max_w + pad * 2.0, total_h + pad * 2.0),
    );
    // 背景跟随主题：取 app_bg 再提高不透明度（暗色=近黑、亮色=近白），
    // 避免固定半透明黑在亮色主题下变成脏块；文字用 contrast_fg 已随主题反转。
    let base = crate::theme::app_bg();
    let bg = egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 235);
    painter.rect_filled(bg_rect, 4.0, bg);
    painter.rect_stroke(
        bg_rect,
        4.0,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
        egui::StrokeKind::Inside,
    );
    let mut ly = tooltip_y;
    for line in lines {
        painter.text(
            egui::pos2(tooltip_x, ly),
            egui::Align2::LEFT_TOP,
            line,
            font_id.clone(),
            crate::theme::contrast_fg(),
        );
        ly += line_h;
    }
}
