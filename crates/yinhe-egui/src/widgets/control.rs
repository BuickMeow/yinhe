//! 自绘控件统一基线：高度 / 圆角 / 状态配色。
//!
//! 参照 `switch` / `flat` 的做法：无缓存、无阈值，全部用 `painter` 绘制，
//! 颜色取自主题（跟随明暗）。组合框 / 输入框 / 数字框 / 拖动条共享这里的
//! 几何与配色，保证视觉一致。

use eframe::egui;

/// 标准控件高度（逻辑像素基准，随字体缩放由 [`h`] 读取）。
pub const CONTROL_H: f32 = 24.0;
/// 标准控件圆角（与 `flat::FILLED_RADIUS` 一致）。
pub const CONTROL_RADIUS: f32 = 6.0;
/// 标准控件水平内边距。
pub const CONTROL_PAD_X: f32 = 8.0;

/// 随字体缩放的标准控件高度（DPI 由 egui zoom 全局处理）。
pub fn h(ctx: &egui::Context) -> f32 {
    crate::scaling::scaled_font(ctx, CONTROL_H)
}

/// 随字体缩放的标准控件圆角。
pub fn radius(ctx: &egui::Context) -> f32 {
    crate::scaling::scaled_font(ctx, CONTROL_RADIUS)
}

/// 随字体缩放的标准控件水平内边距。
pub fn pad_x(ctx: &egui::Context) -> f32 {
    crate::scaling::scaled_font(ctx, CONTROL_PAD_X)
}

/// 三态底色：禁用 / 按下 / 悬停 / 常态。
pub fn state_fill(
    base: egui::Color32,
    enabled: bool,
    hovered: bool,
    pressed: bool,
) -> egui::Color32 {
    if !enabled {
        crate::theme::text_disabled().gamma_multiply(0.25)
    } else if pressed {
        crate::theme::pressed_color(base)
    } else if hovered {
        crate::theme::hover_color(base)
    } else {
        base
    }
}

/// 控件描边：聚焦用强调色，否则 `line_fg`（禁用更淡）。
pub fn control_stroke(enabled: bool, focused: bool) -> egui::Stroke {
    if !enabled {
        egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.35))
    } else if focused {
        egui::Stroke::new(1.5, crate::theme::accent_active())
    } else {
        egui::Stroke::new(1.0, crate::theme::line_fg())
    }
}

/// 画标准控件底（圆角矩形填充 + 内描边）。`radius` 由 [`radius`] 提供以保证随字体缩放。
pub fn paint_bg(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    fill: egui::Color32,
    stroke: egui::Stroke,
) {
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
}
