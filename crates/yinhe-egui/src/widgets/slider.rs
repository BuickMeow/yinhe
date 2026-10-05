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

    // 开关式长轨道：胶囊底 + 圆形滑块。无缓动（值由用户直接拖定）。
    let track_h = 18.0;
    let radius = track_h * 0.5;

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
    // 右侧放不下数字时放左侧。
    let value_left = show_value && (width - value_w) < 80.0;
    let track_w = (width - value_w).max(30.0);

    let (rect, mut resp) = ui.allocate_exact_size(
        egui::vec2(width, control::CONTROL_H),
        egui::Sense::click_and_drag(),
    );
    let track_x = if value_left && show_value {
        rect.min.x + value_w
    } else {
        rect.min.x
    };
    let track = egui::Rect::from_min_size(
        egui::pos2(track_x, rect.center().y - track_h * 0.5),
        egui::vec2(track_w.min(rect.max.x - track_x).max(1.0), track_h),
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
        let white = crate::theme::contrast_fg();

        // 未填充段（右白）
        ui.painter().rect_filled(track, radius, white);
        // 已填充段（左蓝，到滑块中心）
        let thumb_cx = track.min.x + radius + (track.width() - 2.0 * radius).max(1.0) * t;
        let fill = egui::Rect::from_min_size(
            track.min,
            egui::vec2((thumb_cx - track.min.x).max(radius), track_h),
        );
        ui.painter().rect_filled(fill, radius, accent);

        // 圆形滑块（白 + 描边，蓝白底上都可见）
        let hc = egui::pos2(thumb_cx, rect.center().y);
        let handle = if !enabled {
            crate::theme::text_disabled()
        } else if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(white)
        } else if resp.hovered() {
            crate::theme::hover_color(white)
        } else {
            white
        };
        ui.painter().circle_filled(hc, radius, handle);
        ui.painter()
            .circle_stroke(hc, radius, egui::Stroke::new(1.0, crate::theme::line_fg()));

        if show_value {
            let vx = if value_left {
                rect.min.x
            } else {
                track.max.x + 8.0
            };
            ui.painter().galley(
                egui::pos2(vx, rect.center().y - value_galley.size().y * 0.5),
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
