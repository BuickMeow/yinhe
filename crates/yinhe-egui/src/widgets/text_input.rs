//! 自绘外框输入框：统一 `widgets::control` 的高度/圆角/配色，
//! 内部复用 `egui::TextEdit`（光标/选区/IME 保持原生）。

use eframe::egui;

use super::control;

/// 自绘外框的单行输入框。
pub fn control_text_input(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    hint: Option<&str>,
) -> egui::Response {
    let rect = ui
        .allocate_exact_size(egui::vec2(width, control::CONTROL_H), egui::Sense::hover())
        .0;
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + control::CONTROL_PAD_X, rect.min.y + 2.0),
        egui::pos2(rect.max.x - control::CONTROL_PAD_X, rect.max.y - 2.0),
    );
    let te_id = ui.id().with(&id_salt);
    let focused = ui.memory(|m| m.has_focus(te_id));
    control::paint_bg(
        ui.painter(),
        rect,
        control::state_fill(
            crate::theme::control_bg(),
            ui.is_enabled(),
            ui.rect_contains_pointer(rect),
            false,
        ),
        control::control_stroke(ui.is_enabled(), focused),
    );

    let mut te = egui::TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .id(te_id);
    if let Some(h) = hint {
        te = te.hint_text(h);
    }
    let resp = ui.put(inner, te);
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            control::CONTROL_RADIUS,
            control::control_stroke(true, true),
            egui::StrokeKind::Inside,
        );
    }
    resp
}

/// 自绘外框的多行输入框。
pub fn control_text_input_multiline(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    height: f32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Response {
    let rect = ui
        .allocate_exact_size(egui::vec2(width, height), egui::Sense::hover())
        .0;
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + control::CONTROL_PAD_X, rect.min.y + 4.0),
        egui::pos2(rect.max.x - control::CONTROL_PAD_X, rect.max.y - 4.0),
    );
    let te_id = ui.id().with(&id_salt);
    let focused = ui.memory(|m| m.has_focus(te_id));
    control::paint_bg(
        ui.painter(),
        rect,
        control::state_fill(
            crate::theme::control_bg(),
            ui.is_enabled(),
            ui.rect_contains_pointer(rect),
            false,
        ),
        control::control_stroke(ui.is_enabled(), focused),
    );

    let resp = ui.put(
        inner,
        egui::TextEdit::multiline(text)
            .frame(egui::Frame::NONE)
            .id(te_id),
    );
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            control::CONTROL_RADIUS,
            control::control_stroke(true, true),
            egui::StrokeKind::Inside,
        );
    }
    resp
}
