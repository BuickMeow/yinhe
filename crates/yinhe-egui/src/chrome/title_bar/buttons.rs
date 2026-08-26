//! 非 macOS 平台的窗口控制按钮（关闭/最大化/最小化）绘制。
//! 点击判定由 title_bar 的手动 press/release 状态机负责，这里只画外观。

use eframe::egui;

/// Paint the three window control buttons (close, maximize, minimize) for non-macOS platforms.
pub(crate) fn paint_window_buttons(
    ui: &egui::Ui,
    rects: &(egui::Rect, egui::Rect, egui::Rect),
    maximized: bool,
) {
    let painter = ui.painter();
    let (close_rect, maximize_rect, minimize_rect) = rects;

    // ── Close button (red on hover) ──
    let close_hover = close_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let close_pressed =
        close_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let close_bg = if close_pressed {
        crate::theme::pressed_color(crate::theme::danger())
    } else if close_hover {
        crate::theme::danger()
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*close_rect, 0.0, close_bg);
    // X icon
    let x_color = if close_hover {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_label()
    };
    let cx = close_rect.center();
    let x_size = 8.0;
    let x_half = x_size / 2.0;
    let x1 = egui::pos2(cx.x - x_half, cx.y - x_half);
    let x2 = egui::pos2(cx.x + x_half, cx.y + x_half);
    let x3 = egui::pos2(cx.x + x_half, cx.y - x_half);
    let x4 = egui::pos2(cx.x - x_half, cx.y + x_half);
    painter.line_segment([x1, x2], (1.5, x_color));
    painter.line_segment([x3, x4], (1.5, x_color));

    // ── Maximize button ──
    let max_hover = maximize_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let max_pressed =
        max_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let max_bg = if max_pressed {
        crate::theme::pressed_color(crate::theme::app_bg())
    } else if max_hover {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*maximize_rect, 0.0, max_bg);
    let max_color = if max_hover {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_label()
    };
    let mcx = maximize_rect.center();
    let m_size = 9.0;
    if maximized {
        let r1 = egui::Rect::from_center_size(
            egui::pos2(mcx.x - 2.0, mcx.y - 2.0),
            egui::vec2(m_size - 2.0, m_size - 2.0),
        );
        let r2 = egui::Rect::from_center_size(
            egui::pos2(mcx.x + 2.0, mcx.y + 2.0),
            egui::vec2(m_size - 2.0, m_size - 2.0),
        );
        painter.rect_stroke(r1, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
        painter.rect_stroke(r2, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
    } else {
        let r = egui::Rect::from_center_size(mcx, egui::vec2(m_size, m_size));
        painter.rect_stroke(r, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
    }

    // ── Minimize button ──
    let min_hover = minimize_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let min_pressed =
        min_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let min_bg = if min_pressed {
        crate::theme::pressed_color(crate::theme::app_bg())
    } else if min_hover {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*minimize_rect, 0.0, min_bg);
    let min_color = if min_hover {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_label()
    };
    let mn_cx = minimize_rect.center();
    let line_y = mn_cx.y;
    painter.line_segment(
        [
            egui::pos2(mn_cx.x - 5.0, line_y),
            egui::pos2(mn_cx.x + 5.0, line_y),
        ],
        (1.5, min_color),
    );
}
