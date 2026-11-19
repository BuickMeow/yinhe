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

/// 给自绘复选/开关注册 `Role::CheckBox` + checked 状态。
pub(crate) fn name_checkbox(resp: &egui::Response, name: impl Into<String>, checked: bool) {
    let name = name.into();
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, resp.enabled(), checked, name.clone())
    });
}

/// 给自绘 Tab 注册 `Role::Tab` + selected 状态。
///
/// 模式栏 ARRANGE/MIX/EDIT 这类「单选视图」如果只注册 Button，读屏无法知道
/// 当前在哪个视图；`Role::Tab` + `selected` 才能让读屏报「已选中」。
pub(crate) fn name_tab(resp: &egui::Response, name: impl Into<String>, selected: bool) {
    let name = name.into();
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Tab, resp.enabled(), selected, name.clone())
    });
}

/// 给命令控件写稳定的 automation id（AccessKit `author_id`）。
///
/// macOS 映射到 `AXIdentifier`，Windows UIA 映射到 `AutomationId`。
/// 读屏和外部 Agent 都应优先按这个 id 定位，而不是按会随语言变化的 label。
pub(crate) fn set_command_id(resp: &egui::Response, command_id: &str) {
    if command_id.is_empty() {
        return;
    }
    let command_id = command_id.to_owned();
    resp.ctx
        .accesskit_node_builder(resp.id, |node| node.set_author_id(command_id));
}

/// 统一焦点环：键盘用户 Tab 到自绘控件时必须能看见焦点。
pub(crate) fn paint_focus_ring(ui: &egui::Ui, rect: egui::Rect, radius: f32) {
    ui.painter().rect_stroke(
        rect.expand(1.5),
        radius + 1.5,
        egui::Stroke::new(1.5, crate::theme::accent_active().gamma_multiply(0.9)),
        egui::StrokeKind::Inside,
    );
}

/// 注册一个只读语义节点（不参与命中/焦点）。
///
/// 用于手绘、无法用 `ui.interact` 的区域：标题栏文档 tab、窗口按钮、
/// 轨道行摘要。`rect` 用逻辑坐标，读屏可以 hit-test/滚动到它。
pub(crate) fn node(
    ui: &egui::Ui,
    id: egui::Id,
    role: egui::Role,
    label: impl Into<String>,
    rect: egui::Rect,
) {
    let label = label.into();
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(role);
        node.set_label(label);
        node.set_bounds(egui::accesskit::Rect {
            x0: rect.min.x.into(),
            y0: rect.min.y.into(),
            x1: rect.max.x.into(),
            y1: rect.max.y.into(),
        });
    });
}

/// 带选中状态的只读语义节点（标题栏文档 tab 等）。
pub(crate) fn node_selected(
    ui: &egui::Ui,
    id: egui::Id,
    role: egui::Role,
    label: impl Into<String>,
    rect: egui::Rect,
    selected: bool,
) {
    let label = label.into();
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(role);
        node.set_label(label);
        node.set_bounds(egui::accesskit::Rect {
            x0: rect.min.x.into(),
            y0: rect.min.y.into(),
            x1: rect.max.x.into(),
            y1: rect.max.y.into(),
        });
        node.set_toggled(if selected {
            egui::accesskit::Toggled::True
        } else {
            egui::accesskit::Toggled::False
        });
    });
}

/// 给自绘输入控件（旋钮等滑块）注册可读名称。
pub(crate) fn name_slider(resp: &egui::Response, name: impl Into<String>) {
    let name = name.into();
    resp.ctx
        .accesskit_node_builder(resp.id, |node| node.set_label(name));
}

/// 注册一个还没有名称的可选中输入控件（单选/复选）。
pub(crate) fn unnamed_selected(resp: &egui::Response, role: egui::Role, selected: bool) {
    resp.widget_info(|| egui::WidgetInfo {
        enabled: resp.enabled(),
        ..egui::WidgetInfo::selected(role, resp.enabled(), selected, "")
    });
}
