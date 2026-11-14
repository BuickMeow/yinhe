use eframe::egui;
use yinhe_meter::WaveformHistory;

use super::{MeterModule, grid, plot_bg, small_label};

pub(super) struct WaveformModule {
    history: WaveformHistory,
}

impl WaveformModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            history: WaveformHistory::default_for(sample_rate),
        }
    }
}

impl MeterModule for WaveformModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, _dt: f32) {
        if !samples.is_empty() {
            self.history.process(samples);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(6.0, 4.0));
        if body.width() < 20.0 || body.height() < 16.0 {
            return;
        }
        grid(ui.painter(), body, 5, 4);
        let center = body.center().y;
        ui.painter().hline(
            body.x_range(),
            center,
            egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.45)),
        );
        let cols = self.history.columns();
        let n = cols.len();
        if n == 0 {
            return;
        }
        let half = body.height() * 0.46;
        let col_w = (body.width() / n as f32).max(1.0);
        for (i, col) in cols.iter().enumerate() {
            let x = body.min.x + i as f32 * col_w;
            let y0 = center - col.max_l.clamp(-1.0, 1.0) * half;
            let y1 = center - col.min_l.clamp(-1.0, 1.0) * half;
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x, y0.min(y1)),
                    egui::pos2(x + col_w, y0.max(y1)),
                ),
                0.0,
                crate::theme::accent_active().gamma_multiply(0.7),
            );
        }
        small_label(
            ui,
            egui::pos2(body.min.x + 2.0, body.min.y + 1.0),
            egui::Align2::LEFT_TOP,
            "5 s",
            crate::theme::text_muted(),
        );
    }

    fn reset(&mut self) {
        self.history.reset();
    }
}
