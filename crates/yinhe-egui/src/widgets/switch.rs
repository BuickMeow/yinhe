//! iOS 风格开关（Switch），用于设置页复选框的现代替代。
//!
//! - 无缓存、无阈值，纯 egui 绘制 + 可打断 `cubic_out`（中途反向从当前位置平滑续播）
//! - 主题化：`on` 用 `accent_active`，`off` 用 `control_bg`，跟随明暗自动切换
//! - 尺寸 38×22，圆角轨道 + 白色圆 thumb，hover/pressed 有轻微明暗反馈

use eframe::egui;

/// 单开关的打断续播状态，存于 `ctx.data` 的 temp 槽（`Id + TypeId` 隔离，不持久化）。
#[derive(Clone)]
struct SwitchAnim {
    from: f32,
    to: f32,
    start: f64,
}

const SWITCH_DURATION: f32 = 0.2;

/// 可打断的 eased 动画：`target` 翻转时从当前显示值 `from=cur` 重新起播，保证连续且双向均为 `cubic_out`。
fn animate_switch(ctx: &egui::Context, id: egui::Id, target: bool) -> f32 {
    let target_f = if target { 1.0 } else { 0.0 };
    let now = ctx.input(|i| i.time);
    // 半帧外推，与 `egui::Context::animate_value` 保持一致，减少一帧延迟感
    let pred = ctx.input(|i| i.predicted_dt) as f64 * 0.5;
    let now_eff = now + pred;

    let (cur, needs_repaint) = ctx.data_mut(|data| {
        if let Some(anim) = data.get_temp::<SwitchAnim>(id) {
            let elapsed = (now_eff - anim.start) as f32 / SWITCH_DURATION;
            let t = elapsed.clamp(0.0, 1.0);
            let eased = egui::emath::easing::cubic_out(t);
            let cur = egui::lerp(anim.from..=anim.to, eased);
            if (anim.to - target_f).abs() > f32::EPSILON {
                // 目标翻转：从当前位置平滑反向
                data.insert_temp(
                    id,
                    SwitchAnim {
                        from: cur,
                        to: target_f,
                        start: now_eff,
                    },
                );
                (cur, true)
            } else if t < 1.0 {
                (cur, true)
            } else {
                (target_f, false)
            }
        } else {
            data.insert_temp(
                id,
                SwitchAnim {
                    from: target_f,
                    to: target_f,
                    start: now_eff,
                },
            );
            (target_f, false)
        }
    });

    if needs_repaint {
        ctx.request_repaint();
    }
    cur
}

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
        let how_on = animate_switch(ui.ctx(), resp.id, *checked);
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
