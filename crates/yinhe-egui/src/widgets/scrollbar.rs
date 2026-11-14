//! 滚动条组件。
//!
//! - 时间轴滚动条（横向 / 纵向瀑布流）：[`horizontal`]
//! - 像素空间滚动条（AR 行高 / PR 音高）：[`vertical::show_vertical`]
//! - 值空间滚动条（自动化面板）：[`vertical::show_vertical_value`]
//! - 密度预览纹理：[`density`]
//! - 拖动防撞边：[`drag_warp`]

use eframe::egui;

use crate::theme;

mod density;
mod drag_warp;
mod horizontal;
mod vertical;

#[cfg(test)]
mod tests;

pub(crate) use drag_warp::{WarpAxis, drag_delta_with_edge_warp, finish_edge_warp};
pub(crate) use horizontal::{DensityPreview, show};
pub(crate) use vertical::{show_vertical, show_vertical_value};

// ── Constants ──

/// Height of the horizontal scrollbar band.
pub(crate) const SCROLLBAR_H: f32 = theme::SCROLLBAR_H;
/// Width of the vertical scrollbar band.
pub(crate) const SCROLLBAR_W: f32 = theme::SCROLLBAR_W;

/// 边缘缩放区宽度。
pub(super) const EDGE_WIDTH: f32 = 4.0;

/// 滚动条四色（运行时读取当前主题，不能是 const——getter 非 const fn）。
/// hover/drag 用统一悬浮/按下增益，不再单独定义绝对色。
pub(super) fn colors() -> (egui::Color32, egui::Color32, egui::Color32, egui::Color32) {
    let rect = theme::line_fg();
    (
        theme::track_bg(),
        rect,
        theme::hover_color(rect),
        theme::pressed_color(rect),
    )
}

/// 指针是否在 `rect` 内双击（Primary）。只读输入、不注册交互，
/// 避免与滚动条内部已有的 `interact` 冲突。调用方用它做「双击 fit」。
pub(crate) fn double_clicked_in(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    }) && ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| rect.contains(p))
        && !crate::view_interaction::pointer_over_popup(ui.ctx())
}
