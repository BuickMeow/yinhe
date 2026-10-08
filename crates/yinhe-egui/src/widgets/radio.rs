//! 主题化单选圆点（自绘）：外圈 + 选中实心内点，随字体缩放。
//!
//! 供「添加自动化」窗口的整行单选与 `xsynth_config` 的模式选择等复用，
//! 替代 egui 原生 `radio_value`（原生样式不随主题/字号统一）。

use eframe::egui;

/// 圆点半径（逻辑像素，随字体缩放）。
pub(crate) fn radius(ctx: &egui::Context) -> f32 {
    crate::scaling::scaled_font(ctx, 7.0)
}

/// 纯绘制：在 `center` 画一个半径 `r` 的单选圆点（无交互）。
pub(crate) fn paint_radio(
    painter: &egui::Painter,
    center: egui::Pos2,
    r: f32,
    selected: bool,
    hovered: bool,
) {
    let stroke_color = if hovered {
        crate::theme::text_secondary()
    } else {
        crate::theme::text_label()
    };
    painter.circle_stroke(center, r, egui::Stroke::new(1.5, stroke_color));
    if selected {
        painter.circle_filled(center, r * 0.55, crate::theme::accent_active());
    }
}

/// 独立单选控件：占用 `2r` 方形，可点击（只返回 Response，不改值）。
pub(crate) fn radio(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let r = radius(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(r * 2.0, r * 2.0), egui::Sense::click());
    paint_radio(ui.painter(), rect.center(), r, selected, resp.hovered());
    resp
}

/// 主题化 `radio_value`：圆点在左、文本在右；点击圆点或文本都设置 `*current = value`。
pub(crate) fn radio_value<T: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let selected = *current == value;
    ui.horizontal(|ui| {
        let dot = radio(ui, selected);
        let label = ui.label(text);
        let mut resp = dot.union(label);
        if resp.clicked() {
            *current = value;
            resp.mark_changed();
        }
        resp
    })
    .inner
}
