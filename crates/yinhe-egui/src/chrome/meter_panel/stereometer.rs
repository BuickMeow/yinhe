use eframe::egui;
use yinhe_meter::StereoAnalyzer;

use super::{MeterModule, plot_bg, small_label};

pub(super) struct StereometerModule {
    analyzer: StereoAnalyzer,
    points: Vec<(f32, f32)>,
}

impl StereometerModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            analyzer: StereoAnalyzer::new(sample_rate),
            points: Vec::with_capacity(512),
        }
    }
}

impl MeterModule for StereometerModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, _dt: f32) {
        if !samples.is_empty() {
            self.analyzer.process(samples);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(6.0, 4.0));
        if body.width() < 30.0 || body.height() < 30.0 {
            return;
        }
        let meter_h = 12.0;
        let scope = egui::Rect::from_min_max(
            body.min,
            egui::pos2(
                body.max.x,
                (body.max.y - meter_h - 4.0).max(body.min.y + 10.0),
            ),
        );
        let center = scope.center();
        let radius = scope.width().min(scope.height()) * 0.46;
        let circle = egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.35));
        ui.painter().circle_stroke(center, radius, circle);
        ui.painter().vline(
            center.x,
            egui::Rangef::new(center.y - radius, center.y + radius),
            circle,
        );
        ui.painter().hline(
            egui::Rangef::new(center.x - radius, center.x + radius),
            center.y,
            circle,
        );

        self.analyzer.points(512, &mut self.points);
        let peak = self
            .points
            .iter()
            .fold(0.0f32, |m, (a, b)| m.max(a.abs()).max(b.abs()))
            .max(1e-4);
        let scale = radius / peak;
        let color = crate::theme::accent_active().gamma_multiply(0.65);
        for &(mid, side) in &self.points {
            // 逆时针旋转 45°：side 横向、mid 纵向，和常见 goniometer 一致。
            let p = egui::pos2(center.x + side * scale, center.y - mid * scale);
            if scope.contains(p) {
                ui.painter().circle_filled(p, 0.9, color);
            }
        }

        let reading = self.analyzer.reading();
        let corr_rect = egui::Rect::from_min_size(
            egui::pos2(body.min.x, body.max.y - meter_h),
            egui::vec2(body.width(), meter_h),
        );
        ui.painter()
            .rect_filled(corr_rect, 0.0, crate::theme::line_fg().gamma_multiply(0.25));
        let mid_x = corr_rect.center().x;
        let corr_x = mid_x + corr_rect.width() * 0.5 * reading.correlation;
        ui.painter().vline(
            mid_x,
            corr_rect.y_range(),
            egui::Stroke::new(1.0, crate::theme::text_muted()),
        );
        ui.painter().circle_filled(
            egui::pos2(corr_x, corr_rect.center().y),
            3.0,
            crate::theme::accent_active(),
        );
        small_label(
            ui,
            egui::pos2(corr_rect.min.x, corr_rect.min.y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            format!("corr {:.2}", reading.correlation),
            crate::theme::text_secondary(),
        );
        small_label(
            ui,
            egui::pos2(corr_rect.max.x, corr_rect.min.y - 1.0),
            egui::Align2::RIGHT_BOTTOM,
            format!(
                "L {:.0}%  R {:.0}%",
                (1.0 - reading.balance) * 50.0,
                (1.0 + reading.balance) * 50.0
            ),
            crate::theme::text_muted(),
        );
    }

    fn reset(&mut self) {
        self.analyzer.reset();
        self.points.clear();
    }
}
