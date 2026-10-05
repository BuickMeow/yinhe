//! 自绘拖动条（Slider）：开关式长胶囊。
//!
//! 左侧已填充为强调色、右侧未填充为白色，圆形滑块；无缓动（值由用户直接拖定）。
//! 数值绘制在轨道内部（偏滑块的另一侧），两端为纯半圆。

use std::ops::RangeInclusive;

use eframe::egui;

use super::control;

/// 自绘拖动条。`width` 为整体宽度，`step` 为可选吸附步长。
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

    let track_h = 18.0;
    let radius = track_h * 0.5;

    let (rect, mut resp) = ui.allocate_exact_size(
        egui::vec2(width, control::CONTROL_H),
        egui::Sense::click_and_drag(),
    );
    let track = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.center().y - track_h * 0.5),
        egui::vec2(width, track_h),
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
        let white = egui::Color32::WHITE;
        let painter = ui.painter();

        // 未填充段：白色胶囊（两端纯半圆）。
        painter.rect_filled(track, radius, white);

        // 滑块中心（两端各内缩一个半径，使圆形滑块始终落在胶囊内）。
        let travel = (track.width() - 2.0 * radius).max(1.0);
        let thumb_cx = track.min.x + radius + travel * t;

        // 已填充段：从胶囊左端到滑块中心；左端半圆、右端直角（被滑块遮住）。
        if t > 0.0 {
            let fill_w = thumb_cx - track.min.x;
            let fill = egui::Rect::from_min_size(track.min, egui::vec2(fill_w, track_h));
            let r = radius.round() as u8;
            let radius_left = egui::CornerRadius {
                nw: r,
                sw: r,
                ne: 0,
                se: 0,
            };
            painter.rect_filled(
                fill,
                if t >= 1.0 {
                    egui::CornerRadius::same(r)
                } else {
                    radius_left
                },
                accent,
            );
        }

        // 圆形滑块：白底 + 描边（蓝白底上都可见）。
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
        painter.circle_filled(hc, radius, handle);
        painter.circle_stroke(hc, radius, egui::Stroke::new(1.0, crate::theme::line_fg()));

        // 数值：画在轨道内部偏滑块的另一侧；文字随底色调明暗。
        if show_value {
            let value_galley = painter.layout_no_wrap(
                format!("{:.2}", value.to_f64()),
                egui::FontId::proportional(crate::theme::BODY_FONT),
                crate::theme::text_primary(),
            );
            let value_left = thumb_cx > track.center().x;
            let vx = if value_left {
                track.min.x + 8.0
            } else {
                track.max.x - 8.0 - value_galley.size().x
            };
            let color = if value_left {
                crate::theme::contrast_fg()
            } else {
                crate::theme::text_primary()
            };
            painter.galley(
                egui::pos2(vx, rect.center().y - value_galley.size().y * 0.5),
                value_galley,
                color,
            );
        }

        if resp.has_focus() {
            painter.rect_stroke(
                rect,
                control::CONTROL_RADIUS,
                control::control_stroke(true, true),
                egui::StrokeKind::Inside,
            );
        }
    }
    resp
}
