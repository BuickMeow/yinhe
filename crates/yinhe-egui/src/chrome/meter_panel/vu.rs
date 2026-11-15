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

        // 水平 VU：刻度线横贯底部，指针轴心在左下，从左侧刻度扫向右侧。
        let base_y = body.max.y - 10.0;
        let pivot = egui::pos2(body.min.x + body.width() * 0.18, base_y + 6.0);
        let radius = body.width() * 0.74;
        let line_color = crate::theme::line_fg().gamma_multiply(0.5);
        ui.painter()
            .hline(body.x_range(), base_y, egui::Stroke::new(1.0, line_color));

        for (i, label) in [
            "20", "10", "7", "5", "4", "3", "2", "1", "0", "+1", "+2", "+3",
        ]
        .iter()
        .enumerate()
        {
            let tt = i as f32 / 11.0;
            let x = pivot.x + (body.max.x - pivot.x) * tt;
            let hot = i >= 8;
            let color = if hot {
                crate::theme::danger_text()
            } else {
                crate::theme::text_muted()
            };
            ui.painter().vline(
                x,
                egui::Rangef::new(base_y - 4.0, base_y),
                egui::Stroke::new(1.0, color),
            );
            small_label(
                ui,
                egui::pos2(x, base_y - 6.0),
                egui::Align2::CENTER_BOTTOM,
                *label,
                color,
            );
        }

        let angle = std::f32::consts::FRAC_PI_2 + 0.25 - 1.05 * t;
        let needle = egui::pos2(
            pivot.x + radius * angle.cos(),
            pivot.y - radius * angle.sin(),
        );
        ui.painter().line_segment(
            [pivot, needle],
            egui::Stroke::new(1.8, crate::theme::text_primary()),
        );
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
