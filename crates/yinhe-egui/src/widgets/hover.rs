use eframe::egui;

/// 无背景图标/文字按钮（单一绘制）。
///
/// 与旧 Label + hover_highlight 双层的区别：这里先 interact 拿到 hover 状态，
/// 再 painter 一次画完文字/图标 —— hover 时直接换色，不再叠画第二层
/// （消除重复绘制与文字位置跳动）。
///
/// 三态颜色：active = accent_active；hover = 白（hover_text）；inactive = 传入基色。
/// 返回 `Response`（hovered/clicked/rect 供调用方使用）。
pub(crate) fn hover_button(
    ui: &mut egui::Ui,
    text: &str,
    font_id: egui::FontId,
    inactive_color: egui::Color32,
    is_active: bool,
) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    let color = if is_active {
        crate::theme::accent_active()
    } else if resp.hovered() {
        crate::theme::hover_text()
    } else {
        inactive_color
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font_id,
        color,
    );
    resp
}
