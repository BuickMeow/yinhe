use eframe::egui;

/// Paint an 18x18 inline button with a one-letter label and click handling.
pub fn draw_inline_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    active: bool,
    active_color: egui::Color32,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, egui::Sense::click());
    let hovered = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();

    let (fill, text_col) = if active {
        let f = if pressed {
            crate::theme::pressed_color(active_color)
        } else if hovered {
            crate::theme::hover_color(active_color)
        } else {
            active_color
        };
        (f, egui::Color32::BLACK)
    } else {
        let base = crate::theme::btn_bg();
        let f = if pressed {
            crate::theme::pressed_color(base)
        } else if hovered {
            crate::theme::hover_color(base)
        } else {
            base
        };
        (f, crate::theme::text_secondary())
    };

    painter.rect_filled(rect, 3.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        text_col,
    );

    resp
}
