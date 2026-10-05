//! 自绘数字框：统一外框（`widgets::control` 的高度/圆角/配色），
//! 内部复用 `egui::DragValue` 的拖动 + 点击编辑行为。
//!
//! 另外保留中文句号「。」→「.」的折算 parser（用户中文输入法下常误触句号）。

use std::ops::RangeInclusive;

use eframe::egui;

use super::control;

/// 带中文句号折算的数值 parser，用于 `DragValue::custom_parser`。
pub fn decimal_parser(s: &str) -> Option<f64> {
    let s: String = s.replace('。', ".");
    let s: String = s
        .chars()
        .filter(|c| {
            *c == '-' || *c == '+' || *c == '.' || *c == 'e' || *c == 'E' || c.is_ascii_digit()
        })
        .collect();
    s.parse().ok()
}

/// 自绘数字框（拖动改值 + 点击进入文本编辑）。用法与 `egui::DragValue` 一致：
/// `ui.add(decimal_drag_value(&mut x).range(0..=100).speed(1.0))`。
pub fn decimal_drag_value<'a, Num: egui::emath::Numeric>(
    value: &'a mut Num,
) -> ControlDragValue<'a, Num> {
    ControlDragValue {
        value,
        range: None,
        speed: None,
        suffix: None,
        fixed_decimals: None,
    }
}

/// 自绘数字框 builder（实现 [`egui::Widget`]）。
pub struct ControlDragValue<'a, Num: egui::emath::Numeric> {
    value: &'a mut Num,
    range: Option<(f64, f64)>,
    speed: Option<f64>,
    suffix: Option<String>,
    fixed_decimals: Option<usize>,
}

impl<Num: egui::emath::Numeric> ControlDragValue<'_, Num> {
    /// 值域范围（与 `DragValue::range` 一样接受任意 `Numeric`，内部折算为 f64）。
    pub fn range<Num2: egui::emath::Numeric>(mut self, range: RangeInclusive<Num2>) -> Self {
        self.range = Some((range.start().to_f64(), range.end().to_f64()));
        self
    }
    pub fn speed(mut self, speed: impl Into<f64>) -> Self {
        self.speed = Some(speed.into());
        self
    }
    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }
    pub fn fixed_decimals(mut self, n: usize) -> Self {
        self.fixed_decimals = Some(n);
        self
    }
}

impl<Num: egui::emath::Numeric> egui::Widget for ControlDragValue<'_, Num> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut dv = egui::DragValue::new(self.value).custom_parser(decimal_parser);
        if let Some((lo, hi)) = self.range {
            dv = dv.range(lo..=hi);
        }
        if let Some(s) = self.speed {
            dv = dv.speed(s);
        }
        if let Some(s) = self.suffix {
            dv = dv.suffix(s);
        }
        if let Some(f) = self.fixed_decimals {
            dv = dv.fixed_decimals(f);
        }

        // 外框自绘；内部 DragValue 关掉自带底/描边，避免双层。
        let frame = egui::Frame::new()
            .fill(crate::theme::control_bg())
            .stroke(control::control_stroke(ui.is_enabled(), false))
            .corner_radius(control::CONTROL_RADIUS)
            .inner_margin(egui::Margin::symmetric(6, 0));
        let inner = frame.show(ui, |ui| {
            ui.spacing_mut().interact_size.y = control::CONTROL_H;
            ui.scope(|ui| {
                // 彻底关掉 DragValue 内部（含编辑态 TextEdit）的底与描边：
                // 编辑态 TextEdit 的填充取自 extreme_bg_color，必须一并透明。
                let visuals = &mut ui.style_mut().visuals;
                visuals.extreme_bg_color = egui::Color32::TRANSPARENT;
                let widgets = &mut visuals.widgets;
                for v in [
                    &mut widgets.noninteractive,
                    &mut widgets.inactive,
                    &mut widgets.hovered,
                    &mut widgets.active,
                ] {
                    v.bg_fill = egui::Color32::TRANSPARENT;
                    v.weak_bg_fill = egui::Color32::TRANSPARENT;
                    v.bg_stroke = egui::Stroke::NONE;
                }
                ui.add(dv)
            })
            .inner
        });

        let resp = inner.inner;
        if resp.has_focus() {
            ui.painter().rect_stroke(
                inner.response.rect,
                control::CONTROL_RADIUS,
                control::control_stroke(true, true),
                egui::StrokeKind::Inside,
            );
        }
        resp
    }
}
