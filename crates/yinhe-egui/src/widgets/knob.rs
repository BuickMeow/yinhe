//! 自绘旋钮（Knob）：垂直拖动改变归一化值 0..1。
//!
//! 风格与 [`crate::widgets::switch`] 对齐：主题化、hover/pressed 反馈、
//! 无缓存无阈值；一条环表达刻度，强调色值弧覆盖在环上。
//!
//! 两种归零点：
//! - [`knob`]：最左侧（值 0）归零，值弧从起点向右延伸；
//! - [`knob_centered`]：居中（值 0.5）归零，值弧向两侧延伸，居中时长度为 0。

use eframe::egui;

/// 值域对应的扫角（度）：135°（左下）顺时针扫 270° 到 405°（右下），经过顶部。
const START_DEG: f32 = 135.0;
const SWEEP_DEG: f32 = 270.0;

/// 刻度环 / 值弧的线宽（px）。两者共用同一半径、同一线宽，合成一条环。
const RING_W: f32 = 1.0;

/// 归零点：最左端归零，或居中归零。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ZeroMode {
    Left,
    Center,
}

/// 值弧覆盖的归一化区间（`from` → `to`）。零长度时返回 `None`，即不画值弧。
fn arc_span(value: f32, zero: ZeroMode) -> Option<(f32, f32)> {
    let v = value.clamp(0.0, 1.0);
    let (from, to) = match zero {
        ZeroMode::Left => (0.0, v),
        ZeroMode::Center => (0.5, v),
    };
    ((to - from).abs() > 1e-4).then_some((from, to))
}

/// 拖动会话（存 `ctx.data` 的 temp 槽，按 widget Id 隔离）：
/// 每帧从**起始值 + 指针位移**重算，避免 `drag_delta` 累计语义导致的回弹。
#[derive(Clone, Copy)]
struct KnobSession {
    start_norm: f32,
    start_y: f32,
}

/// 旋钮（归一化值 0..1，最左侧归零）。返回 `Response`；`changed()` 表示值被拖动改变。
pub fn knob(ui: &mut egui::Ui, value: &mut f32, diameter: f32) -> egui::Response {
    knob_impl(ui, value, diameter, ZeroMode::Left)
}

/// 居中归零旋钮（归一化值 0..1，0.5 归零，如声像 Pan）。
pub fn knob_centered(ui: &mut egui::Ui, value: &mut f32, diameter: f32) -> egui::Response {
    knob_impl(ui, value, diameter, ZeroMode::Center)
}

fn knob_impl(ui: &mut egui::Ui, value: &mut f32, diameter: f32, zero: ZeroMode) -> egui::Response {
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
        // 注意：用 `ui.painter()`（裁剪到 ui 区域）而非 `painter_at(rect)`——
        // 后者把绘制裁到 26×26 的方框内，会切掉环上的值弧、看起来像有个方框。
        let painter = ui.painter();
        let center = rect.center();
        // 以原来的**内圈**为基准：刻度环与值弧共用同一半径、同一线宽，
        // 值弧直接覆盖在内圈上；外圈值弧不再存在。
        let r_base = diameter / 2.0 - 3.5;
        let r_ring = r_base - 0.5;

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
        painter.circle_filled(center, r_base, base);
        // 刻度环：完整一圈（字体色系），值弧覆盖其上。
        painter.circle_stroke(
            center,
            r_ring,
            egui::Stroke::new(RING_W, crate::theme::text_secondary()),
        );

        let start = START_DEG.to_radians();
        let sweep = SWEEP_DEG.to_radians();
        let accent = if !enabled {
            crate::theme::text_disabled().gamma_multiply(0.6)
        } else {
            crate::theme::accent_active()
        };

        // 值弧：与刻度环同半径同线宽；居中归零时从中点向两侧延伸。
        if let Some((from, to)) = arc_span(*value, zero) {
            let steps = 24;
            let mut pts = Vec::with_capacity(steps + 1);
            for i in 0..=steps {
                let t = from + (to - from) * (i as f32 / steps as f32);
                let ang = start + sweep * t;
                pts.push(center + egui::vec2(ang.cos(), ang.sin()) * r_ring);
            }
            painter.add(egui::Shape::line(pts, egui::Stroke::new(RING_W, accent)));
        }

        // 指针（底座内）。
        let ang = start + sweep * *value;
        let tip = center + egui::vec2(ang.cos(), ang.sin()) * (r_base - 2.0);
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
                r_ring + 1.5,
                egui::Stroke::new(1.5, crate::theme::accent_active().gamma_multiply(0.9)),
            );
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(v: f32, zero: ZeroMode) -> Option<(f32, f32)> {
        arc_span(v, zero)
    }

    /// 最左归零：0 时无值弧，其余从起点 0 长到当前值。
    #[test]
    fn arc_span_left_zero() {
        assert_eq!(span(0.0, ZeroMode::Left), None);
        assert_eq!(span(0.6, ZeroMode::Left), Some((0.0, 0.6)));
        assert_eq!(span(1.0, ZeroMode::Left), Some((0.0, 1.0)));
    }

    /// 居中归零：0.5 时半环长度为 0；小于/大于 0.5 向两侧延伸。
    #[test]
    fn arc_span_center_zero() {
        assert_eq!(span(0.5, ZeroMode::Center), None);
        assert_eq!(span(0.75, ZeroMode::Center), Some((0.5, 0.75)));
        assert_eq!(span(0.25, ZeroMode::Center), Some((0.5, 0.25)));
        assert_eq!(span(0.0, ZeroMode::Center), Some((0.5, 0.0)));
        assert_eq!(span(1.0, ZeroMode::Center), Some((0.5, 1.0)));
    }
}
