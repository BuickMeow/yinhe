use eframe::egui;

use crate::theme;

/// 命中区外扩（像素）：可视线只有 `SPLIT_HANDLE_W`(2px)，若命中区也仅 2px
/// 则极难悬停/抓取。这里把交互矩形外扩若干像素（绘制仍用原细线），既保证
/// 易抓取，又不会像 egui 默认 `interact_radius` 那样扩到 ~5px 之外误触。
const HIT_PAD: f32 = 4.0;

/// Paint and interact with a horizontal split handle.
///
/// Returns the `Response` so the caller can inspect `dragged()`, `drag_delta()`, etc.
///
/// 只有指针真的位于（外扩后的）handle 命中区内才响应 hover/按下拖拽：egui 的
/// interact_radius 会把 handle 边缘 ~5px 外的按下也判为命中，导致拖动自动化锚点等
/// 并行交互时误拖分割线（自动化和分割线一起拖动）。这里用固定的 HIT_PAD 收紧。
/// 注意：返回的 `Response.dragged()` 在 interact_radius 扩散命中时仍可能为 true，
/// 调用方如需门控行为，请同样用 `pointer.press_origin()` 校验按下位置。
pub fn horizontal(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    rect: egui::Rect,
) -> egui::Response {
    let hit = rect.expand2(egui::vec2(0.0, HIT_PAD));
    let resp = ui.interact(hit, ui.id().with(id_salt), egui::Sense::click_and_drag());
    let on_rect = ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| hit.contains(p));
    let press_on_rect = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| hit.contains(p));
    let active = press_on_rect && resp.dragged();
    let hovered = on_rect && resp.hovered();
    ui.painter().rect_filled(
        rect,
        0.0,
        if active {
            theme::pressed_color(theme::line_fg())
        } else if hovered {
            theme::hover_color(theme::line_fg())
        } else {
            theme::line_fg()
        },
    );
    if hovered || active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    resp
}

/// Paint and interact with a vertical split handle.
///
/// Returns the `Response` so the caller can inspect `dragged()`, `drag_delta()`, etc.
///
/// 与 `horizontal` 相同：只用固定 `HIT_PAD` 命中区（不扩散到 5px 外）。
pub fn vertical(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    rect: egui::Rect,
) -> egui::Response {
    let hit = rect.expand2(egui::vec2(HIT_PAD, 0.0));
    let resp = ui.interact(hit, ui.id().with(id_salt), egui::Sense::click_and_drag());
    let on_rect = ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| hit.contains(p));
    let press_on_rect = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| hit.contains(p));
    let active = press_on_rect && resp.dragged();
    let hovered = on_rect && resp.hovered();
    ui.painter().rect_filled(
        rect,
        0.0,
        if active {
            theme::pressed_color(theme::line_fg())
        } else if hovered {
            theme::hover_color(theme::line_fg())
        } else {
            theme::line_fg()
        },
    );
    if hovered || active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    resp
}
