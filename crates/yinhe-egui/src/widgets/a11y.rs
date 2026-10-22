//! 自绘控件的无障碍辅助：注册 AccessKit 角色与名称。

use eframe::egui;

/// 给自绘按钮注册 `Role::Button` 与可读名称。
///
/// 自绘按钮通常用 `allocate_exact_size` + `painter`，默认不会进入 AccessKit
/// 树；`widget_info` 会创建节点，`accessible_name` 再覆盖可能来自图标字形的
/// 占位标签。
pub(crate) fn name_button(resp: &egui::Response, name: impl Into<String>) {
    let name = name.into();
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, resp.enabled(), name.clone())
    });
}

/// 注册一个还没有名称的输入控件；外层可用 [`egui::Ui::name_inputs`] 补名称。
pub(crate) fn unnamed_input(resp: &egui::Response, role: egui::Role) {
    resp.widget_info(|| egui::WidgetInfo {
        enabled: resp.enabled(),
        ..egui::WidgetInfo::new(role)
    });
}

/// 注册一个还没有名称的可选中输入控件（单选/复选）。
pub(crate) fn unnamed_selected(resp: &egui::Response, role: egui::Role, selected: bool) {
    resp.widget_info(|| egui::WidgetInfo {
        enabled: resp.enabled(),
        ..egui::WidgetInfo::selected(role, resp.enabled(), selected, "")
    });
}
