use eframe::egui;
use yinhe_types::ArRowLayout;

/// 最成熟的悬停检测：统一走 `view_interaction::pointer_hits` / `pointer_over_popup`，
/// 感知 `Order::Foreground` 的 popup 遮挡与 clip，避免裸 `hover_pos.contains` 透传。
pub fn hover_track(
    ui: &egui::Ui,
    panel_rect: egui::Rect,
    row_layout: &ArRowLayout,
    scroll_y: f32,
    lh: f32,
) -> Option<usize> {
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return None;
    }
    if !crate::view_interaction::pointer_hits(ui, panel_rect) {
        return None;
    }
    let pos = ui.input(|i| i.pointer.hover_pos())?;
    row_layout
        .hit_at_music_y(pos.y - panel_rect.min.y + scroll_y, lh)
        .map(|h| h.track())
}

/// 行级悬停：是否 hover 在 `rect` 上且未被 popup 遮挡。
/// 取代散落的 `row_rect.contains(hover_pos.unwrap_or_default())`。
pub fn is_row_hovered(ui: &egui::Ui, rect: egui::Rect) -> bool {
    crate::view_interaction::pointer_hits(ui, rect)
}

/// 图标对比色：按轨道颜色亮度选黑/白，保证 chevron/+ 在色带上可读。
///
/// 复用 [`crate::theme::contrast_text`]（拖动条轨道内数值也用同一逻辑）。
pub fn icon_contrast_color(color: [f32; 4]) -> egui::Color32 {
    let c = egui::Color32::from_rgba_unmultiplied(
        (color[0] * 255.0).round() as u8,
        (color[1] * 255.0).round() as u8,
        (color[2] * 255.0).round() as u8,
        (color[3] * 255.0).round() as u8,
    );
    crate::theme::contrast_text(c)
}
