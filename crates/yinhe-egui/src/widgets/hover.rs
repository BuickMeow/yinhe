use eframe::egui;

/// 无背景图标/文字按钮（单一绘制）。
///
/// 与旧 Label + hover_highlight 双层的区别：这里先 interact 拿到 hover 状态，
/// 再 painter 一次画完文字/图标 —— hover 时直接换色，不再叠画第二层
/// （消除重复绘制与文字位置跳动）。
///
/// 三态颜色：active = accent_active；hover = 白（contrast_fg）；inactive = 传入基色。
/// 返回 `Response`（hovered/clicked/rect 供调用方使用）。
pub(crate) fn hover_button(
    ui: &mut egui::Ui,
    text: &str,
    font_id: egui::FontId,
    inactive_color: egui::Color32,
    is_active: bool,
) -> egui::Response {
    hover_button_impl(ui, text, font_id, inactive_color, is_active, 0.0)
}

/// 与 `hover_button` 相同，但文字/图标绕按钮中心旋转 `angle` 弧度。
/// 用于复用同一图标表达垂直方向（如 ☰ 旋转 90° 表示纵向瀑布流）。
pub(crate) fn hover_button_rotated(
    ui: &mut egui::Ui,
    text: &str,
    font_id: egui::FontId,
    inactive_color: egui::Color32,
    is_active: bool,
    angle: f32,
) -> egui::Response {
    hover_button_impl(ui, text, font_id, inactive_color, is_active, angle)
}

fn hover_button_impl(
    ui: &mut egui::Ui,
    text: &str,
    font_id: egui::FontId,
    inactive_color: egui::Color32,
    is_active: bool,
    angle: f32,
) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    let color = if is_active {
        crate::theme::accent_active()
    } else if resp.hovered() {
        crate::theme::contrast_fg()
    } else {
        inactive_color
    };
    if angle == 0.0 {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font_id,
            color,
        );
    } else {
        // 旋转绘制（与 selection_actions 的 ICON_FLIP 旋转同款写法）：
        // galley 已排版，直接用 TextShape 绕中心旋转。
        let pos = egui::Align2::CENTER_CENTER
            .anchor_size(rect.center(), galley.size())
            .min;
        ui.painter().add(
            egui::epaint::TextShape::new(pos, galley, color)
                .with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER),
        );
    }
    resp
}
