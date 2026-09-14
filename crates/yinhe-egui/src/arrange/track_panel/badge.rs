use eframe::egui;

/// 在色带图标位置画一个图标，点击直接返回 `true`（不弹菜单）。
///
/// 与 [`badge_icon_menu`] 同视觉；用于「+」直接打开添加自动化窗口。
pub fn badge_icon_button(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    codepoint: &str,
    family: egui::FontFamily,
    color: egui::Color32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let rect = egui::Rect::from_center_size(center, egui::vec2(12.0, 16.0));
    let resp = ui.interact(
        rect,
        egui::Id::new(("badge_icon_btn", id_salt)),
        egui::Sense::click(),
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, family),
        color,
    );
    resp.clicked()
}
