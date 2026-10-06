//! 步进框：`(-|1000|+)`——左侧减号、右侧加号，中间是可拖动/点击编辑的数值。
//!
//! 与数字框（`numeric_input`）同款外框与数值行为，只是多两侧按钮；
//! 点击按钮按 `step` 增减（默认 1，其它控件可用 10 / 0.1 等）。

use std::ops::RangeInclusive;

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD, ICON_REMOVE};

use super::{control, numeric_input};

/// 自绘步进框。`step` 为点按加减的步长；`width` 为整体宽度。
pub fn stepper<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut Num,
    range: RangeInclusive<Num>,
    step: f64,
    width: f32,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let h = control::h(&ctx);
    let radius = control::radius(&ctx);
    let lo = range.start().to_f64();
    let hi = range.end().to_f64();

    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let id = resp.id;

    // 左右按钮各占一个正方形，中间为数值区。
    let btn_w = h;
    let minus_rect = egui::Rect::from_min_size(rect.min, egui::vec2(btn_w, h));
    let plus_rect = egui::Rect::from_min_size(
        egui::pos2(rect.max.x - btn_w, rect.min.y),
        egui::vec2(btn_w, h),
    );
    let mid_rect = egui::Rect::from_min_max(
        egui::pos2(minus_rect.max.x, rect.min.y),
        egui::pos2(plus_rect.min.x, rect.max.y),
    );

    let minus = ui.interact(minus_rect, id.with("minus"), egui::Sense::click());
    let plus = ui.interact(plus_rect, id.with("plus"), egui::Sense::click());

    let enabled = ui.is_enabled();
    let mut changed = false;
    if enabled && minus.clicked() {
        let v = value.to_f64() - step;
        *value = Num::from_f64(v.max(lo));
        changed = true;
    }
    if enabled && plus.clicked() {
        let v = value.to_f64() + step;
        *value = Num::from_f64(v.min(hi));
        changed = true;
    }

    if ui.is_rect_visible(rect) {
        let base = crate::theme::btn_bg();
        control::paint_bg(
            ui.painter(),
            rect,
            radius,
            control::state_fill(base, enabled, false, false),
            control::control_stroke(enabled, false),
        );

        // 两侧按钮的 hover/按下底：只圆外侧两角，贴合胶囊轮廓。
        let r = radius as u8;
        let corner_left = egui::CornerRadius {
            nw: r,
            sw: r,
            ne: 0,
            se: 0,
        };
        let corner_right = egui::CornerRadius {
            nw: 0,
            sw: 0,
            ne: r,
            se: r,
        };
        for (btn_rect, corners, btn) in [
            (minus_rect, corner_left, &minus),
            (plus_rect, corner_right, &plus),
        ] {
            let fill = if !enabled {
                crate::theme::text_disabled().gamma_multiply(0.25)
            } else if btn.is_pointer_button_down_on() {
                crate::theme::pressed_color(base)
            } else if btn.hovered() {
                crate::theme::hover_color(base)
            } else {
                continue;
            };
            ui.painter().rect_filled(btn_rect, corners, fill);
        }

        // 加减图标。
        let icon_color = if enabled {
            crate::theme::text_primary()
        } else {
            crate::theme::text_disabled()
        };
        for (btn_rect, icon) in [(minus_rect, ICON_REMOVE), (plus_rect, ICON_ADD)] {
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
                icon_color,
            );
        }

        // 数值区两侧的细分隔线（与加减号同色）。
        let inset = crate::scaling::scaled_font(&ctx, 4.0);
        let sep = egui::Stroke::new(crate::scaling::scaled_font(&ctx, 1.0), icon_color);
        for x in [mid_rect.min.x, mid_rect.max.x] {
            ui.painter().line_segment(
                [
                    egui::pos2(x, rect.min.y + inset),
                    egui::pos2(x, rect.max.y - inset),
                ],
                sep,
            );
        }
    }

    // 中间数值：无框 DragValue，占满数值区并居中。
    let mid_resp = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(mid_rect)
                .layout(egui::Layout::centered_and_justified(
                    egui::Direction::TopDown,
                )),
            |ui| {
                numeric_input::apply_frameless_drag_visuals(ui);
                ui.spacing_mut().interact_size.y = h;
                ui.add(
                    egui::DragValue::new(value)
                        .custom_parser(numeric_input::decimal_parser)
                        .range(lo..=hi)
                        .speed(step),
                )
            },
        )
        .inner;
    if mid_resp.changed() {
        changed = true;
    }

    if changed {
        resp.mark_changed();
    }
    resp
}
