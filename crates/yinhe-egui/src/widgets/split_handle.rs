use eframe::egui;

use crate::theme;

/// Paint and interact with a horizontal split handle.
///
/// Returns the `Response` so the caller can inspect `dragged()`, `drag_delta()`, etc.
pub fn horizontal(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    rect: egui::Rect,
) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(id_salt), egui::Sense::click_and_drag());
    ui.painter().rect_filled(
        rect,
        0.0,
        if resp.hovered() || resp.dragged() {
            theme::SPLIT_HOVER
        } else {
            theme::SPLIT_DEFAULT
        },
    );
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    resp
}

/// Paint and interact with a vertical split handle.
///
/// Returns the `Response` so the caller can inspect `dragged()`, `drag_delta()`, etc.
pub fn vertical(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    rect: egui::Rect,
) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(id_salt), egui::Sense::click_and_drag());
    ui.painter().rect_filled(
        rect,
        0.0,
        if resp.hovered() || resp.dragged() {
            theme::V_SPLIT_HOVER
        } else {
            theme::V_SPLIT_DEFAULT
        },
    );
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    resp
}
