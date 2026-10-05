//! 自绘拖动条（Slider）：统一 `widgets::control` 的高度/配色。
//!
//! 轨道 + 已填充段 + 圆形滑块；点击/拖动轨道即设值，可选步进吸附与右侧数值。

use std::ops::RangeInclusive;

use eframe::egui;

use super::control;

/// 自绘拖动条。`width` 为整体宽度（含右侧数值区），`step` 为可选吸附步长。
pub fn control_slider<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut Num,
    range: RangeInclusive<Num>,
    width: f32,
    step: Option<f64>,
    show_value: bool,
) -> egui::Response {
    let min = range.start().to_f64();
    let max = range.end().to_f64();
    let span = (max - min).max(f64::EPSILON);

    let value_font = egui::FontId::proportional(crate::theme::BODY_FONT);
    let value_galley = ui.painter().layout_no_wrap(
        format!("{:.2}", value.to_f64()),
        value_font,
        crate::theme::text_primary(),
    );
    let value_w = if show_value {
        value_galley.size().x + 8.0
    } else {
        0.0
    };
    let track_w = (width - value_w).max(20.0);

    let (rect, mut resp) = ui.allocate_exact_size(
        egui::vec2(width, control::CONTROL_H),
        egui::Sense::click_and_drag(),
    );
    let track = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.center().y - 2.0),
        egui::vec2(track_w, 4.0),
    );

    let mut v = value.to_f64();
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let t = (((pos.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0)) as f64;
        let mut nv = min + span * t;
        if let Some(s) = step
            && s > 0.0
        {
            nv = (nv / s).round() * s;
        }
        v = nv.clamp(min, max);
    }
    if v != value.to_f64() {
        *value = Num::from_f64(v);
        resp.mark_changed();
    }
    let t = (((value.to_f64() - min) / span).clamp(0.0, 1.0)) as f32;

    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let accent = if enabled {
            crate::theme::accent_active()
        } else {
            crate::theme::text_disabled().gamma_multiply(0.6)
        };
        ui.painter()
            .rect_filled(track, 2.0, crate::theme::control_bg());
        let fill =
            egui::Rect::from_min_size(track.min, egui::vec2(track.width() * t, track.height()));
        ui.painter().rect_filled(fill, 2.0, accent);

        let hr = 5.0;
        let hc = egui::pos2(track.min.x + track.width() * t, rect.center().y);
        let handle = if !enabled {
            crate::theme::text_disabled()
        } else if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(accent)
        } else if resp.hovered() {
            crate::theme::hover_color(accent)
        } else {
            accent
        };
        ui.painter().circle_filled(hc, hr, handle);
        ui.painter().circle_stroke(
            hc,
            hr,
            egui::Stroke::new(1.0, crate::theme::contrast_fg().gamma_multiply(0.4)),
        );

        if show_value {
            ui.painter().galley(
                egui::pos2(
                    track.max.x + 8.0,
                    rect.center().y - value_galley.size().y * 0.5,
                ),
                value_galley,
                crate::theme::text_primary(),
            );
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect,
                control::CONTROL_RADIUS,
                control::control_stroke(true, true),
                egui::StrokeKind::Inside,
            );
        }
    }
    resp
}
