use eframe::egui;

/// 无背景图标/文字按钮（单一绘制）。
///
/// 与旧 Label + hover_highlight 双层的区别：这里先 interact 拿到 hover 状态，
/// 再 painter 一次画完文字/图标 —— hover 时直接换色，不再叠画第二层
/// （消除重复绘制与文字位置跳动）。
///
/// 四态颜色：active = accent_active；hover 亮走 text_primary/暗走白；pressed 比 hover 回退一档；inactive = 传入基色。
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

/// hover 类控件的统一四态配色：active = accent_active；按下/悬停按暗色/亮色分支；
/// 常态 = 传入基色。`hover_button` 与自绘组合按钮（文字+图标，如音轨名）共用同一
/// 逻辑，保证时间码右侧控件配色方案一致。
pub(crate) fn hover_button_color(
    resp: &egui::Response,
    inactive_color: egui::Color32,
    is_active: bool,
) -> egui::Color32 {
    if is_active {
        crate::theme::accent_active()
    } else if resp.is_pointer_button_down_on() {
        if crate::theme::dark_mode() {
            crate::theme::text_primary()
        } else {
            crate::theme::text_primary().gamma_multiply(0.88)
        }
    } else if resp.hovered() {
        if crate::theme::dark_mode() {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_primary()
        }
    } else {
        inactive_color
    }
}

fn hover_button_impl(
    ui: &mut egui::Ui,
    text: &str,
    font_id: egui::FontId,
    inactive_color: egui::Color32,
    is_active: bool,
    angle: f32,
) -> egui::Response {
    // 用 PLACEHOLDER 排版：glyph 不带固定色，绘制时由 TextShape 的 fallback
    // 颜色统一着色（旋转分支与自绘组合按钮复用该 galley 时不会残留白色）。
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), font_id.clone(), egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    let color = hover_button_color(&resp, inactive_color, is_active);
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
