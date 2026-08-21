//! 主题化复选框：egui 0.36 无独立 checkbox 视觉字段，对勾颜色 = widgets 三态
//! `fg_stroke`。`main_loop` 已全局覆盖为主题主文字色，此处再用 `Ui::scope`
//! 局部覆盖确保 1.5 宽更清晰（并处理 disabled 态），作用域外不受影响。

/// 主题复选框（普通场景）。
pub(crate) fn checkbox(
    ui: &mut egui::Ui,
    checked: &mut bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    check_scope(ui, |ui| ui.checkbox(checked, text)).inner
}

/// 对勾颜色作用域：覆盖 widgets 三态 fg_stroke 为主题主文字色（宽 1.5 更清晰），
/// 作用域结束后恢复。`ui.put` 场景（如音源列表行内 checkbox）也用这个。
pub(crate) fn check_scope<R>(
    ui: &mut egui::Ui,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.scope(|ui| {
        let check = crate::theme::text_primary();
        let vs = ui.visuals_mut();
        vs.widgets.inactive.fg_stroke = egui::Stroke::new(1.5, check);
        vs.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, check);
        vs.widgets.active.fg_stroke = egui::Stroke::new(1.5, check);
        // 禁用态（disabled/不可交互）也覆盖：编辑中轨道等场景为 disabled 但仍需可见
        vs.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.5, crate::theme::text_disabled());
        add(ui)
    })
}
