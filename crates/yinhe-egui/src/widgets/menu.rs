//! 菜单项按钮的通用样式（铺满整行 + 无边框）。

use eframe::egui;

/// 菜单项按钮：铺满整行宽度 + 无边框。
///
/// 无边框原因：egui 按钮 inactive 时无边框、hover 时 1px 边框从无到有，
/// 视觉上像文字被框住内缩（位移感）；去掉边框后 hover 只剩背景色变化，
/// 文字与边缘距离恒定。
pub(crate) fn menu_item_button(
    ui: &egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Button<'static> {
    egui::Button::selectable(selected, text)
        .min_size(egui::vec2(ui.available_width(), 30.0))
        .stroke(egui::Stroke::NONE)
}
