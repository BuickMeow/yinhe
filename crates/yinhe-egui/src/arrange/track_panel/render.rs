use eframe::egui;

use yinhe_types::ArRowLayout;

/// 详情/紧凑模式的文字字号，统一 clamp，避免散落魔法数字
pub fn detail_font(lh: f32) -> egui::FontId {
    egui::FontId::proportional((lh * 0.25).clamp(9.0, 13.0))
}
pub fn compact_font(lh: f32) -> egui::FontId {
    egui::FontId::proportional((lh * 0.45).clamp(8.0, 14.0))
}

/// 行背景：条纹 → 选中 → 悬停，三层叠加，复用 `hover::is_row_hovered` 的成熟实现
pub fn draw_row_background(
    ui: &egui::Ui,
    painter: &egui::Painter,
    row_rect: egui::Rect,
    row_idx: usize,
    is_selected: bool,
    stripe_even: egui::Color32,
) {
    if row_idx.is_multiple_of(2) {
        painter.rect_filled(row_rect, 0.0, stripe_even);
    }
    if is_selected {
        painter.rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
    } else if super::hover::is_row_hovered(ui, row_rect) {
        painter.rect_filled(
            row_rect,
            0.0,
            crate::theme::hover_color(crate::theme::app_bg()),
        );
    }
}

/// 色带（窄条）绘制，高 14px，色值由调用方传入
pub fn draw_badge(
    painter: &egui::Painter,
    row_rect: egui::Rect,
    lh: f32,
    color32: egui::Color32,
) -> egui::Rect {
    let badge_w = 14.0_f32;
    let badge_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(badge_w, lh));
    painter.rect_filled(badge_rect, 0.0, color32);
    badge_rect
}

/// 可视范围与总行数计算，抽自 `show` 顶部，避免在 `show` 中散落 clamp 逻辑
pub fn visible_range(scroll_y: f32, panel_h: f32, lh: f32, total_rows: usize) -> (usize, usize) {
    let first = (scroll_y / lh).floor().max(0.0) as usize;
    let last = ((scroll_y + panel_h) / lh).ceil().max(0.0) as usize + 1;
    (first.min(total_rows), last.min(total_rows))
}

/// 根据 `row_layout` 计算每轨主行矩形，用于拖拽插入线
pub fn build_item_rects(
    row_layout: &ArRowLayout,
    panel_rect: egui::Rect,
    panel_w: f32,
    lh: f32,
    scroll_y: f32,
    num_tracks: usize,
) -> Vec<egui::Rect> {
    let mut rects = Vec::with_capacity(num_tracks);
    for idx in 0..num_tracks {
        let y = panel_rect.min.y + row_layout.track_y(idx, lh) - scroll_y;
        rects.push(egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, lh),
        ));
    }
    rects
}
