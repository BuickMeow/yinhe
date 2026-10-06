//! 自绘外框输入框：统一 `widgets::control` 的高度/圆角/配色，
//! 内部复用 `egui::TextEdit`（光标/选区/IME 保持原生）。

use eframe::egui;
use egui_material_icons::icons::ICON_CLOSE;

use super::control;

/// 自绘外框的单行输入框。
pub fn control_text_input(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    hint: Option<&str>,
) -> egui::Response {
    control_text_input_impl(ui, text, width, id_salt, hint, false)
}

/// 同 [`control_text_input`]，带右侧 × 清除按钮（有内容时显示）。
pub fn control_text_input_clearable(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    hint: Option<&str>,
) -> egui::Response {
    control_text_input_impl(ui, text, width, id_salt, hint, true)
}

fn control_text_input_impl(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    hint: Option<&str>,
    clearable: bool,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
    let radius = control::radius(&ctx);
    let pad_x = control::pad_x(&ctx);
    let rect = ui
        .allocate_exact_size(egui::vec2(width, control::h(&ctx)), egui::Sense::hover())
        .0;
    let te_id = ui.id().with(&id_salt);
    let focused = ui.memory(|m| m.has_focus(te_id));
    control::paint_bg(
        ui.painter(),
        rect,
        radius,
        control::state_fill(
            crate::theme::control_bg(),
            ui.is_enabled(),
            ui.rect_contains_pointer(rect),
            false,
        ),
        control::control_stroke(ui.is_enabled(), focused),
    );

    // 文本行垂直居中：给 TextEdit 一个居中的行高矩形。
    let clear_w = if clearable && !text.is_empty() {
        scale(20.0)
    } else {
        0.0
    };
    let text_h = scale(18.0);
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + pad_x, rect.center().y - text_h * 0.5),
        egui::pos2(rect.max.x - pad_x - clear_w, rect.center().y + text_h * 0.5),
    );
    let mut te = egui::TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .id(te_id);
    if let Some(h) = hint {
        te = te.hint_text(h);
    }
    let mut resp = ui.put(inner, te);

    if clear_w > 0.0 {
        let x_rect = egui::Rect::from_center_size(
            egui::pos2(rect.max.x - pad_x - scale(7.0), rect.center().y),
            egui::vec2(scale(14.0), scale(14.0)),
        );
        let x_resp = ui.interact(x_rect, te_id.with("clear"), egui::Sense::click());
        let color = if x_resp.hovered() {
            crate::theme::text_primary()
        } else {
            crate::theme::text_label()
        };
        ui.painter().text(
            x_rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_CLOSE.codepoint,
            egui::FontId::new(crate::theme::ICON_FONT, ICON_CLOSE.font_family()),
            color,
        );
        if x_resp.clicked() {
            text.clear();
            resp.mark_changed();
        }
    }

    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            radius,
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
    let ctx = ui.ctx().clone();
    let radius = control::radius(&ctx);
    let pad_x = control::pad_x(&ctx);
    let pad_y = crate::scaling::scaled_font(&ctx, 4.0);
    let rect = ui
        .allocate_exact_size(egui::vec2(width, height), egui::Sense::hover())
        .0;
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + pad_x, rect.min.y + pad_y),
        egui::pos2(rect.max.x - pad_x, rect.max.y - pad_y),
    );
    let te_id = ui.id().with(&id_salt);
    let focused = ui.memory(|m| m.has_focus(te_id));
    control::paint_bg(
        ui.painter(),
        rect,
        radius,
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
            radius,
            control::control_stroke(true, true),
            egui::StrokeKind::Inside,
        );
    }
    resp
}
