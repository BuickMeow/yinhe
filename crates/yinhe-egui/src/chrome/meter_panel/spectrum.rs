use eframe::egui;
use yinhe_meter::{SpectrumAnalyzer, spectrum::DEFAULT_FFT_SIZE};

use super::{MeterModule, grid, plot_bg, small_label};

const COLUMNS: usize = 256;
const MIN_FREQ: f32 = 20.0;
const MAX_FREQ: f32 = 20_000.0;
const MIN_DB: f32 = -90.0;

pub(super) struct SpectrumModule {
    analyzer: SpectrumAnalyzer,
    smooth: Vec<f32>,
    outline: Vec<egui::Pos2>,
    last_dt: f32,
}

impl SpectrumModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            analyzer: SpectrumAnalyzer::new(sample_rate, DEFAULT_FFT_SIZE),
            smooth: vec![MIN_DB; COLUMNS],
            outline: Vec::with_capacity(COLUMNS),
            last_dt: 1.0 / 60.0,
        }
    }

    fn x_for_freq(rect: egui::Rect, freq: f32) -> f32 {
        let t = (freq.clamp(MIN_FREQ, MAX_FREQ) / MIN_FREQ).ln() / (MAX_FREQ / MIN_FREQ).ln();
        rect.min.x + rect.width() * t
    }
}

impl MeterModule for SpectrumModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, dt: f32) {
        self.last_dt = dt.max(1e-4);
        if !samples.is_empty() {
            self.analyzer.process(samples);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(6.0, 4.0));
        if body.width() < 20.0 || body.height() < 16.0 {
            return;
        }
        for f in [100.0, 1_000.0, 10_000.0] {
            let x = Self::x_for_freq(body, f);
            small_label(
                ui,
                egui::pos2(x + 2.0, body.max.y - 1.0),
                egui::Align2::LEFT_BOTTOM,
                if f >= 1000.0 {
                    format!("{:.0}k", f / 1000.0)
                } else {
                    format!("{f:.0}")
                },
                crate::theme::text_muted(),
            );
        }

        let bins = self.analyzer.magnitudes_db();
        if bins.is_empty() {
            return;
        }
        let alpha = 1.0 - (-self.last_dt / 0.08).exp();
        for i in 0..COLUMNS {
            let f0 = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(i as f32 / COLUMNS as f32);
            let f1 = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf((i + 1) as f32 / COLUMNS as f32);
            let b0 = ((f0 / self.analyzer.sample_rate() as f32) * self.analyzer.fft_size() as f32)
                .floor() as usize;
            let b1 = ((f1 / self.analyzer.sample_rate() as f32) * self.analyzer.fft_size() as f32)
                .ceil() as usize;
            let b1 = b1.min(bins.len()).max(b0 + 1).min(bins.len());
            let target = if b0 < bins.len() {
                bins[b0..b1].iter().copied().fold(MIN_DB, f32::max)
            } else {
                MIN_DB
            };
            self.smooth[i] += (target - self.smooth[i]) * alpha;
        }

        // 逐列画矩形而不是一个凹多边形：egui 对凹多边形填充会从首个顶点
        // 扇形三角化，出现大量从左下角发散的斜线。
        self.outline.clear();
        let col_w = body.width() / COLUMNS as f32 + 0.5;
        let fill = crate::theme::accent_active().gamma_multiply(0.20);
        for i in 0..COLUMNS {
            let db = self.smooth[i].clamp(MIN_DB, 0.0);
            let y = body.max.y - body.height() * (db - MIN_DB) / -MIN_DB;
            let x = body.min.x + body.width() * i as f32 / (COLUMNS - 1) as f32;
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x, y),
                    egui::pos2((x + col_w).min(body.max.x), body.max.y),
                ),
                0.0,
                fill,
            );
            self.outline.push(egui::pos2(x, y));
        }
        // 网格画在填充之上：浅色主题下填充会盖住底部网格。
        grid(ui.painter(), body, 6, 4);
        ui.painter().add(egui::Shape::line(
            std::mem::take(&mut self.outline),
            egui::Stroke::new(1.0, crate::theme::accent_active()),
        ));
        self.outline.reserve(COLUMNS);
    }

    fn reset(&mut self) {
        self.analyzer.reset();
        self.smooth.fill(MIN_DB);
    }
}
