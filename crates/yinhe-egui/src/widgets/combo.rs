//! ComboBox 的统一样式：无边框悬停 + 固定宽度。
//!
//! 复用 `menu::menu_item_button`（`stroke(NONE)` + `available_width`）保证
//! 下拉项与 `Popup` 菜单一致：hover 时仅背景变色，文字不位移。
//! 外层 `ComboBox` 与内层 `Ui` 同时锁宽，避免 `available_width → popup 宽度 → available_width` 正反馈
//! 导致的宽度抖动与 `Area::get_best_align` 翻转。

use eframe::egui;

/// 固定宽度的 ComboBox，下拉项需配合 [`combo_item`] 使用。
/// `width` 为按钮与弹窗的统一宽度（建议 160 / 200），调用方可按内容长度选择。
pub fn combo_box(
    ui: &mut egui::Ui,
    id: &str,
    selected_text: impl Into<egui::WidgetText>,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::InnerResponse<Option<()>> {
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected_text)
        .width(width)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add_contents(ui);
        })
}

/// 下拉项：无边框、铺满整行，与 `menu::menu_item_button` 同样式。
/// 必须在 [`combo_box`] 的闭包内调用。
pub fn combo_item(
    ui: &mut egui::Ui,
    selected: bool,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.add(crate::widgets::menu::menu_item_button(ui, selected, label))
}

/// DRY 封装：`selected: &mut T` + `options: &[(T, String)]` 一行完成 ComboBox。
/// 自动查找 `selected` 对应的显示文本（找不到则用 `format!("{:?}", selected)` 兜底），
/// 下拉项无边框且固定宽度，返回是否发生改变。
/// 调用方只需处理副作用（如 `set_locale`/`set_theme`）。
pub fn combo_select<T: PartialEq + Clone + std::fmt::Debug>(
    ui: &mut egui::Ui,
    id: &str,
    selected: &mut T,
    width: f32,
    options: &[(T, String)],
) -> bool {
    let selected_text = options
        .iter()
        .find(|(v, _)| v == selected)
        .map(|(_, s)| s.as_str())
        .unwrap_or("")
        .to_owned();
    // 空选项时兜底显示 Debug，避免 panic
    let display = if selected_text.is_empty() {
        format!("{:?}", selected)
    } else {
        selected_text
    };
    let mut changed = false;
    combo_box(ui, id, display, width, |ui| {
        for (value, label) in options {
            let is_selected = *value == *selected;
            if combo_item(ui, is_selected, label).clicked() {
                *selected = value.clone();
                changed = true;
            }
        }
    });
    changed
}
