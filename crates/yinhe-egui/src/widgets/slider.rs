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

    // 与 `switch` 完全同尺寸：高 22、圆角 11、滑块半径 = 圆角 - 2（内缩 2px）。
    let track_h = 22.0;
    let radius = track_h * 0.5;
    let thumb_r = radius - 2.0;

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
    // 缓动：目标值跳变（点击/吸附一格）时滑块平滑滑过去；拖动时也顺滑跟随。
    let disp = ui.ctx().animate_value_with_time(resp.id, t, 0.15);

    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let accent = if enabled {
            crate::theme::accent_active()
        } else {
            crate::theme::text_disabled().gamma_multiply(0.6)
        };
        // 未填充段：主题表面色胶囊（浅色主题浅、深色主题深，两端纯半圆）。
        let surface = crate::theme::control_bg();
        let knob = crate::theme::app_bg();
        let painter = ui.painter();
        painter.rect_filled(track, radius, surface);

        // 滑块中心（两端各内缩一个半径，使圆形滑块始终落在胶囊内）。
        let travel = (track.width() - 2.0 * radius).max(1.0);
        let thumb_cx = track.min.x + radius + travel * disp;

        // 已填充段：从胶囊左端到「滑块中心 + 一个半径」，两端都是半圆的胶囊；
        // 右端半圆恰被圆形滑块整块盖住，所以不会漏色。
        let fill_right = (thumb_cx + radius).min(track.max.x);
        let fill = egui::Rect::from_min_max(track.min, egui::pos2(fill_right, track.max.y));
        painter.rect_filled(fill, radius, accent);

        // 圆形滑块：主题基色 + 阴影 + 描边（与 switch 的 thumb 同款处理）。
        let hc = egui::pos2(thumb_cx, rect.center().y);
        let handle = if !enabled {
            crate::theme::text_disabled()
        } else if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(knob)
        } else if resp.hovered() {
            crate::theme::hover_color(knob)
        } else {
            knob
        };
        painter.circle_filled(
            hc + egui::vec2(0.0, 1.0),
            thumb_r,
            egui::Color32::from_black_alpha(40),
        );
        painter.circle_filled(hc, thumb_r, handle);
        painter.circle_stroke(hc, thumb_r, egui::Stroke::new(1.0, crate::theme::line_fg()));
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
