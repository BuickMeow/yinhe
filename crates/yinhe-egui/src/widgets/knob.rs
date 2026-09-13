//! 自绘旋钮（Knob）：垂直拖动改变归一化值 0..1。
//!
//! 风格与 [`crate::widgets::switch`] 对齐：主题化、hover/pressed 反馈、
//! 无缓存无阈值；值弧 + 指针表达当前值。

use eframe::egui;

/// 值域对应的扫角（度）：135°（左下）顺时针扫 270° 到 405°（右下），经过顶部。
const START_DEG: f32 = 135.0;
const SWEEP_DEG: f32 = 270.0;

/// 拖动会话（存 `ctx.data` 的 temp 槽，按 widget Id 隔离）：
/// 每帧从**起始值 + 指针位移**重算，避免 `drag_delta` 累计语义导致的回弹。
#[derive(Clone, Copy)]
struct KnobSession {
    start_norm: f32,
    start_y: f32,
}

/// 旋钮（归一化值 0..1）。返回 `Response`；`changed()` 表示值被拖动改变。
pub fn knob(ui: &mut egui::Ui, value: &mut f32, diameter: f32) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(
        egui::vec2(diameter, diameter),
        egui::Sense::click_and_drag(),
    );
    let id = resp.id;
    let pointer_y = resp.interact_pointer_pos().map(|p| p.y);
    if resp.drag_started()
        && let Some(y) = pointer_y
    {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                id,
                KnobSession {
                    start_norm: *value,
                    start_y: y,
                },
            )
        });
    }
    if resp.dragged()
        && let Some(y) = pointer_y
        && let Some(session) = ui.ctx().data(|d| d.get_temp::<KnobSession>(id))
    {
        // 向上拖 = 增大；灵敏度：拖动约 2.5 倍直径覆盖全程。
        let dy = y - session.start_y;
        *value = (session.start_norm - dy / (diameter * 2.5)).clamp(0.0, 1.0);
        resp.mark_changed();
    }
    if resp.drag_stopped() {
        ui.ctx().data_mut(|d| d.remove::<KnobSession>(id));
    }
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), *value as f64, ""));

    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();
        let painter = ui.painter_at(rect);
        let center = rect.center();
        let r = diameter / 2.0 - 1.0;

        // 底座圆：hover/pressed 反馈与 switch 一致。
        let base = crate::theme::control_bg();
        let base = if !enabled {
            crate::theme::text_disabled().gamma_multiply(0.25)
        } else if pressed {
            crate::theme::pressed_color(base)
        } else if hovered {
            crate::theme::hover_color(base)
        } else {
            base
        };
        painter.circle_filled(center, r, base);
        painter.circle_stroke(
            center,
            r,
            egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.35)),
        );

        let start = START_DEG.to_radians();
        let sweep = SWEEP_DEG.to_radians();
        let accent = if !enabled {
            crate::theme::text_disabled().gamma_multiply(0.6)
        } else {
            crate::theme::accent_active()
        };

        // 值弧（从起点扫到当前值）。
        let steps = 24;
        let mut pts = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let t = *value * (i as f32 / steps as f32);
            let ang = start + sweep * t;
            pts.push(center + egui::vec2(ang.cos(), ang.sin()) * (r - 2.0));
        }
        if pts.len() >= 2 {
            painter.add(egui::Shape::line(pts, egui::Stroke::new(2.0, accent)));
        }

        // 指针。
        let ang = start + sweep * *value;
        let tip = center + egui::vec2(ang.cos(), ang.sin()) * (r - 4.0);
        let pointer_color = if enabled {
            crate::theme::text_primary()
        } else {
            crate::theme::text_disabled()
        };
        painter.line_segment([center, tip], egui::Stroke::new(1.8, pointer_color));

        // 焦点环（键盘可达性）。
        if resp.has_focus() {
            painter.circle_stroke(
                center,
                r + 1.5,
                egui::Stroke::new(1.5, crate::theme::accent_active().gamma_multiply(0.9)),
            );
        }
    }
    resp
}
