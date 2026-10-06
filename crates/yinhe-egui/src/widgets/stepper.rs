//! 步进框：`(-|1000|+)`——左侧减号、右侧加号，中间是可拖动/点击编辑的数值。
//!
//! 与数字框（`numeric_input`）同款外框与数值行为，只是多两侧按钮；
//! 点击按钮按 `step` 增减（可 1 / 10 / 0.1 等），支持后缀与固定小数位。
//!
//! 用法（builder）：
//! ```ignore
//! stepper(&mut value).range(0..=128).step(1.0).width(110.0).show(ui);
//! ```

use std::ops::RangeInclusive;

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD, ICON_REMOVE};

use super::{control, numeric_input};

/// 创建步进框 builder（绑定 `value`）。
pub fn stepper<Num: egui::emath::Numeric>(value: &mut Num) -> Stepper<'_, Num> {
    Stepper {
        value,
        range: None,
        step: 1.0,
        suffix: None,
        decimals: None,
        width: 110.0,
    }
}

/// 步进框 builder。
pub struct Stepper<'a, Num> {
    value: &'a mut Num,
    range: Option<(f64, f64)>,
    step: f64,
    suffix: Option<String>,
    decimals: Option<usize>,
    width: f32,
}

impl<Num: egui::emath::Numeric> Stepper<'_, Num> {
    /// 值域（接受任意 `Numeric`，内部折算为 f64）。
    pub fn range<Num2: egui::emath::Numeric>(mut self, range: RangeInclusive<Num2>) -> Self {
        self.range = Some((range.start().to_f64(), range.end().to_f64()));
        self
    }
    /// 点按加减的步长。
    pub fn step(mut self, step: impl Into<f64>) -> Self {
        self.step = step.into();
        self
    }
    /// 数值后缀（如 " tick"、" s"）。
    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }
    /// 固定小数位（显示与加减取整都用它）。
    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = Some(decimals);
        self
    }
    /// 整体宽度。
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// 绘制并交互，返回 `Response`（`changed()` 表示值变化）。
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let Stepper {
            value,
            range,
            step,
            suffix,
            decimals,
            width,
        } = self;
        let ctx = ui.ctx().clone();
        let h = control::h(&ctx);
        let radius = control::radius(&ctx);
        let (lo, hi) = range.unwrap_or((f64::NEG_INFINITY, f64::INFINITY));

        let round = |v: f64| match decimals {
            Some(n) => {
                let f = 10f64.powi(n as i32);
                (v * f).round() / f
            }
            None => v,
        };

        let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
        let id = resp.id;

        // 左中右三段紧贴，仅靠描边分界。
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
            *value = Num::from_f64(round(value.to_f64() - step).max(lo));
            changed = true;
        }
        if enabled && plus.clicked() {
            *value = Num::from_f64(round(value.to_f64() + step).min(hi));
            changed = true;
        }

        if ui.is_rect_visible(rect) {
            let base = crate::theme::btn_bg();
            let fill = control::state_fill(base, enabled, false, false);
            let stroke = control::control_stroke(enabled, false);
            // 整块胶囊（四角圆角）。
            control::paint_bg(ui.painter(), rect, radius, fill, stroke);

            // 两侧按钮的 hover/按下底：只圆外侧两角，贴合胶囊。
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

            // 段间分隔线：与描边同色同宽，占满整个高度。
            for x in [mid_rect.min.x, mid_rect.max.x] {
                ui.painter().line_segment(
                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                    stroke,
                );
            }
        }

        // 中间数值：无框 DragValue，占满数值区并居中。
        let mid_resp = ui
            .scope_builder(
                egui::UiBuilder::new().max_rect(mid_rect).layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                ),
                |ui| {
                    numeric_input::apply_frameless_drag_visuals(ui);
                    ui.spacing_mut().interact_size.y = h;
                    let mut dv = egui::DragValue::new(value)
                        .custom_parser(numeric_input::decimal_parser)
                        .speed(step);
                    if let Some((lo, hi)) = range {
                        dv = dv.range(lo..=hi);
                    }
                    if let Some(s) = &suffix {
                        dv = dv.suffix(s.clone());
                    }
                    if let Some(n) = decimals {
                        dv = dv.fixed_decimals(n);
                    }
                    ui.add(dv)
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
}
