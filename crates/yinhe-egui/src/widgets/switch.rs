//! iOS 风格开关（Switch），用于设置页复选框的现代替代。
//!
//! - 无缓存、无阈值，纯 egui 绘制 + `animate_bool_with_time_and_easing(cubic_out)`
//! - 主题化：`on` 用 `accent_active`，`off` 用 `control_bg`，跟随明暗自动切换
//! - 尺寸 38×22，圆角轨道 + 白色圆 thumb，hover/pressed 有轻微明暗反馈

use eframe::egui;

/// iOS 风格开关。点击切换 `checked`，返回 `Response`（`changed()` 可判定是否切换）。
pub fn switch(ui: &mut egui::Ui, checked: &mut bool) -> egui::Response {
    let desired = egui::vec2(38.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    if resp.clicked() {
        *checked = !*checked;
        resp.mark_changed();
    }
    // 键盘：Space/Enter 触发（allocate 的 click 已含部分键盘，但显式处理更稳）
    if resp.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter))
    {
        *checked = !*checked;
        resp.mark_changed();
    }
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *checked, "")
    });

    if ui.is_rect_visible(rect) {
        let visuals_enabled = ui.is_enabled();
        let how_on = ui.ctx().animate_bool_with_time_and_easing(
            resp.id,
            *checked,
            0.2,
            egui::emath::easing::cubic_out,
        );
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();

        let off_fill = crate::theme::control_bg();
        let on_fill = crate::theme::accent_active();

        // hover/pressed 对当前状态的填充做微调（off/on 分别基于各自基色）
        let base_fill = if *checked { on_fill } else { off_fill };
        let filled = if !visuals_enabled {
            crate::theme::text_disabled().gamma_multiply(0.35)
        } else if pressed {
            crate::theme::pressed_color(base_fill)
        } else if hovered {
            crate::theme::hover_color(base_fill)
        } else {
            base_fill
        };
        // 未启用时不做 lerp，直接用 disabled 色；启用时在 off→on 间线性插值
        let track_fill = if !visuals_enabled {
            filled
        } else {
            // 手动对 RGBA 做 lerp，保持与 thumb 动画同步
            let off = off_fill;
            let on = on_fill;
            let on_hovered = if hovered && !pressed {
                crate::theme::hover_color(on)
            } else if pressed {
                crate::theme::pressed_color(on)
            } else {
                on
            };
            let off_hovered = if hovered && !pressed {
                crate::theme::hover_color(off)
            } else if pressed {
                crate::theme::pressed_color(off)
            } else {
                off
            };
            let r = egui::lerp(
                (off_hovered.r() as f32 / 255.0)..=(on_hovered.r() as f32 / 255.0),
                how_on,
            );
            let g = egui::lerp(
                (off_hovered.g() as f32 / 255.0)..=(on_hovered.g() as f32 / 255.0),
                how_on,
            );
            let b = egui::lerp(
                (off_hovered.b() as f32 / 255.0)..=(on_hovered.b() as f32 / 255.0),
                how_on,
            );
            let a = egui::lerp(
                (off_hovered.a() as f32 / 255.0)..=(on_hovered.a() as f32 / 255.0),
                how_on,
            );
            egui::Color32::from_rgba_premultiplied(
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
                (a * 255.0).round() as u8,
            )
        };

        let radius = rect.height() / 2.0;
        let painter = ui.painter_at(rect);
        // 轨道
        painter.rect_filled(rect, radius, track_fill);
        // 轻微内描边，增强对比（off 时用 line_fg，on 时用 track_fill 的深一点）
        let stroke_col = if *checked {
            track_fill.gamma_multiply(0.75)
        } else {
            crate::theme::line_fg().gamma_multiply(0.35)
        };
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0, stroke_col),
            egui::StrokeKind::Inside,
        );

        // thumb
        let thumb_r = radius - 2.0;
        let thumb_x = egui::lerp(
            (rect.left() + thumb_r + 2.0)..=(rect.right() - thumb_r - 2.0),
            how_on,
        );
        let thumb_center = egui::pos2(thumb_x, rect.center().y);
        // 阴影
        painter.circle_filled(
            thumb_center + egui::vec2(0.0, 1.0),
            thumb_r,
            egui::Color32::from_black_alpha(40),
        );
        painter.circle_filled(thumb_center, thumb_r, egui::Color32::WHITE);
        // 焦点环
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(1.5),
                radius + 1.5,
                egui::Stroke::new(1.5, crate::theme::accent_active().gamma_multiply(0.9)),
                egui::StrokeKind::Inside,
            );
        }
    }
    resp
}
