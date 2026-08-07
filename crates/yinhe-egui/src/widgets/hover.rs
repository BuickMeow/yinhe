use eframe::egui;

/// Paint a white text overlay at the center of `resp` when hovered and not active,
/// with a unified hover/pressed background (base = app_bg).
/// Use this for hover-highlight effects on buttons/labels that show an icon or text
/// in a muted color normally, and switch to white on hover.
pub(crate) fn hover_highlight(
    ui: &egui::Ui,
    resp: &egui::Response,
    text: &str,
    font_id: egui::FontId,
    is_active: bool,
) {
    // 统一悬浮/按下底色：hover = 增益底，按住 = 按下增益底
    if resp.hovered() && !is_active {
        let bg = if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(crate::theme::app_bg())
        } else {
            crate::theme::hover_color(crate::theme::app_bg())
        };
        ui.painter().rect_filled(resp.rect, 4.0, bg);
    }
    if !is_active && resp.hovered() {
        ui.painter().text(
            resp.rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font_id,
            crate::theme::hover_text(),
        );
    }
}
