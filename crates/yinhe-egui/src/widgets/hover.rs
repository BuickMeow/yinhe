use eframe::egui;

/// Paint a white text overlay at the center of `resp` when hovered and not active.
/// Use this for hover-highlight effects on buttons/labels that show an icon or text
/// in a muted color normally, and switch to white on hover.
/// 无背景色设计：mode bar / transport 工具栏等图标按钮统一不加悬浮/按下底色。
pub(crate) fn hover_highlight(
    ui: &egui::Ui,
    resp: &egui::Response,
    text: &str,
    font_id: egui::FontId,
    is_active: bool,
) {
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
