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
/// 原来在 AM 末行与无自动化主行两处重复，此处 DRY。
pub fn icon_contrast_color(color: [f32; 4]) -> egui::Color32 {
    let lum = color[0] * 0.299 + color[1] * 0.587 + color[2] * 0.114;
    if lum > 0.55 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}
