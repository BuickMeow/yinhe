//! ComboBox 的统一样式：无边框悬停 + 固定宽度。
//!
//! 复用 `menu::menu_item_button`（`stroke(NONE)` + `available_width`）保证
//! 下拉项与 `Popup` 菜单一致：hover 时仅背景变色，文字不位移。
//! 外层 `ComboBox` 与内层 `Ui` 同时锁宽，避免 `available_width → popup 宽度 → available_width` 正反馈
//! 导致的宽度抖动与 `Area::get_best_align` 翻转。

use eframe::egui;

/// 默认宽度（与设置页常用下拉一致），调用方可按内容选 70/160/200。
pub const DEFAULT_WIDTH: f32 = 160.0;

/// 固定宽度的 ComboBox，下拉项需配合 [`combo_item`] 使用。
/// `width` 为按钮与弹窗的统一宽度，`id` 为 `ComboBox::from_id_salt` 的盐（`&str` 或任意 `Hash`）。
///
/// # Example
/// ```ignore
/// combo_box(ui, "theme", theme_label, 160.0, |ui| {
///     for (v, label) in &options {
///         if combo_item(ui, *v == selected, label).clicked() { selected = v.clone(); }
///     }
/// });
/// ```
pub fn combo_box(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
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

/// 在 `options` 中查找 `selected` 对应的显示文本（纯函数，便于单测）。
pub(crate) fn selected_label<'a, T: PartialEq>(
    options: &'a [(T, String)],
    selected: &T,
) -> Option<&'a str> {
    options
        .iter()
        .find(|(v, _)| v == selected)
        .map(|(_, s)| s.as_str())
}

/// DRY 封装：`selected: &mut T` + `options: &[(T, String)]` 一行完成 ComboBox。
/// 找不到对应文本时显示空（`debug_assert` 提示调用方 `options` 缺口），不依赖 `Debug`。
/// 下拉项无边框且固定宽度，返回是否发生改变。调用方只需处理副作用（如 `set_locale`）。
///
/// # Example
/// ```ignore
/// let opts = vec![(0u8, "Auto".to_owned()), (1, "1".to_owned())];
/// if combo_select(ui, "port", &mut port, 70.0, &opts) { /* changed */ }
/// ```
pub fn combo_select<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut T,
    width: f32,
    options: &[(T, String)],
) -> bool {
    let display = selected_label(options, selected).unwrap_or_else(|| {
        debug_assert!(
            false,
            "combo_select: selected 不在 options 中，检查 options 是否缺口"
        );
        ""
    });
    let mut changed = false;
    combo_box(ui, id, display.to_owned(), width, |ui| {
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

/// 使用 [`DEFAULT_WIDTH`] 的快捷版本（设置页/右键面板常用 160 宽）。
pub fn combo_select_auto<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut T,
    options: &[(T, String)],
) -> bool {
    combo_select(ui, id, selected, DEFAULT_WIDTH, options)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_WIDTH, selected_label};

    #[test]
    fn selected_label_found() {
        let opts = vec![(1u8, "a".to_owned()), (2, "b".to_owned())];
        assert_eq!(selected_label(&opts, &2), Some("b"));
    }

    #[test]
    fn selected_label_missing_returns_none() {
        let opts = vec![(1u8, "a".to_owned())];
        assert_eq!(selected_label(&opts, &9), None);
    }

    #[test]
    fn selected_label_empty_options() {
        let opts: Vec<(u8, String)> = vec![];
        assert_eq!(selected_label(&opts, &1), None);
    }

    #[test]
    fn default_width_is_160() {
        assert_eq!(DEFAULT_WIDTH, 160.0);
    }
}
