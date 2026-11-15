use eframe::egui;
use yinhe_meter::VuAnalyzer;

use super::{MeterModule, db_text, plot_bg, small_label};
use crate::widgets::meter::gain_to_db;

/// VU 表盘刻度（dB）：从 -20 到 +3，0 之后为红色过载区。
const MARKS: &[(f32, &str)] = &[
    (-20.0, "20"),
    (-10.0, "10"),
    (-7.0, "7"),
    (-5.0, "5"),
    (-3.0, "3"),
    (-1.0, "1"),
    (0.0, "0"),
    (3.0, "+3"),
];

pub(super) struct VuModule {
    analyzer: VuAnalyzer,
}

impl VuModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            analyzer: VuAnalyzer::new(sample_rate),
        }
    }
}

impl MeterModule for VuModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, _dt: f32) {
        if !samples.is_empty() {
            self.analyzer.process(samples);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(8.0, 4.0));
        if body.width() < 30.0 || body.height() < 30.0 {
            return;
        }
        let db = gain_to_db(self.analyzer.reading().rms_mid);
        let t = ((db + 20.0) / 23.0).clamp(0.0, 1.0);

        // 标准 VU：底部中央为轴心，刻度沿上半圆弧分布。
        let center = egui::pos2(body.center().x, body.max.y - 6.0);
        let radius = body.width().min(body.height() * 1.8) * 0.42;
        let start = std::f32::consts::PI - 0.55;
        let end = 0.35;
        let angle_at = |tt: f32| start + (end - start) * tt;

        let arc_color = crate::theme::line_fg().gamma_multiply(0.55);
        let overload_color = crate::theme::danger_text();
        let steps = 36;
        let mut prev = None;
        for i in 0..=steps {
            let a = angle_at(i as f32 / steps as f32);
            let p = egui::pos2(center.x + radius * a.cos(), center.y - radius * a.sin());
            if let Some(q) = prev {
                ui.painter()
                    .line_segment([q, p], egui::Stroke::new(1.0, arc_color));
            }
            prev = Some(p);
        }
        // 0 到 +3 的过载弧段用红色。
        let zero_t = 20.0 / 23.0;
        let mut prev = None;
        for i in 0..=12 {
            let tt = zero_t + (1.0 - zero_t) * i as f32 / 12.0;
            let a = angle_at(tt);
            let p = egui::pos2(center.x + radius * a.cos(), center.y - radius * a.sin());
            if let Some(q) = prev {
                ui.painter()
                    .line_segment([q, p], egui::Stroke::new(1.6, overload_color));
            }
            prev = Some(p);
        }

        for &(mark_db, label) in MARKS {
            let tt = ((mark_db + 20.0) / 23.0).clamp(0.0, 1.0);
            let a = angle_at(tt);
            let inner = egui::pos2(
                center.x + (radius - 5.0) * a.cos(),
                center.y - (radius - 5.0) * a.sin(),
            );
            let outer = egui::pos2(
                center.x + (radius + 2.0) * a.cos(),
                center.y - (radius + 2.0) * a.sin(),
            );
            let color = if mark_db >= 0.0 {
                overload_color
            } else {
                crate::theme::text_muted()
            };
            ui.painter()
                .line_segment([inner, outer], egui::Stroke::new(1.0, color));
            let label_pos = egui::pos2(
                center.x + (radius + 9.0) * a.cos(),
                center.y - (radius + 9.0) * a.sin(),
            );
            small_label(ui, label_pos, egui::Align2::CENTER_CENTER, label, color);
        }

        let angle = angle_at(t);
        let needle = egui::pos2(
            center.x + (radius - 3.0) * angle.cos(),
            center.y - (radius - 3.0) * angle.sin(),
        );
        ui.painter().line_segment(
            [center, needle],
            egui::Stroke::new(1.8, crate::theme::text_primary()),
        );
        ui.painter()
            .circle_filled(center, 3.0, crate::theme::text_secondary());
        small_label(
            ui,
            egui::pos2(body.max.x - 2.0, body.min.y + 1.0),
            egui::Align2::RIGHT_TOP,
            format!("{} dB", db_text(db)),
            crate::theme::text_secondary(),
        );
    }

    fn reset(&mut self) {
        self.analyzer.reset();
    }
}
