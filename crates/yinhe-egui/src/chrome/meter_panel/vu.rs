use eframe::egui;
use yinhe_meter::VuAnalyzer;

use super::{MeterModule, db_text, plot_bg, small_label};
use crate::widgets::meter::gain_to_db;

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
        // 表盘顺时针旋转 90°：中心在左侧中点，弧线从下（-90°）到上（+90°）。
        let center = egui::pos2(body.min.x + 6.0, body.center().y);
        let radius = body.height().min(body.width() * 1.4) * 0.42;
        let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * t;
        let arc_color = crate::theme::line_fg().gamma_multiply(0.5);
        let steps = 32;
        let mut prev = None;
        for i in 0..=steps {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / steps as f32;
            let p = egui::pos2(center.x + radius * a.cos(), center.y - radius * a.sin());
            if let Some(q) = prev {
                ui.painter()
                    .line_segment([q, p], egui::Stroke::new(1.0, arc_color));
            }
            prev = Some(p);
        }
        for (i, label) in ["-20", "-10", "-6", "-3", "0", "+3"].iter().enumerate() {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 5.0;
            let p = egui::pos2(
                center.x + (radius - 7.0) * a.cos(),
                center.y - (radius - 7.0) * a.sin(),
            );
            small_label(
                ui,
                p,
                egui::Align2::CENTER_CENTER,
                *label,
                crate::theme::text_muted(),
            );
        }
        let needle = egui::pos2(
            center.x + radius * angle.cos(),
            center.y - radius * angle.sin(),
        );
        ui.painter().line_segment(
            [center, needle],
            egui::Stroke::new(1.6, crate::theme::text_primary()),
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
