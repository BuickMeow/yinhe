use eframe::egui;
use yinhe_meter::SpectrogramAnalyzer;

use super::{MeterModule, plot_bg, small_label};

const TEXTURE_W: usize = 512;
const TEXTURE_H: usize = 256;
const MIN_FREQ: f32 = 20.0;
const MAX_FREQ: f32 = 20_000.0;
const MIN_DB: f32 = -90.0;

pub(super) struct SpectrogramModule {
    analyzer: SpectrogramAnalyzer,
    sample_rate: u32,
    texture: Option<egui::TextureHandle>,
    dirty: bool,
}

impl SpectrogramModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            analyzer: SpectrogramAnalyzer::default_for(sample_rate),
            sample_rate,
            texture: None,
            dirty: true,
        }
    }

    fn color_for_db(db: f32) -> egui::Color32 {
        let t = ((db - MIN_DB) / -MIN_DB).clamp(0.0, 1.0);
        let bg = crate::theme::track_bg();
        let mid = crate::theme::accent_active();
        let high = crate::theme::text_primary();
        let lerp = |a: egui::Color32, b: egui::Color32, t: f32| {
            egui::Color32::from_rgb(
                (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
                (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
                (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
            )
        };
        if t < 0.55 {
            lerp(bg, mid, t / 0.55)
        } else {
            lerp(mid, high, (t - 0.55) / 0.45)
        }
    }

    fn rebuild_texture(&mut self, ctx: &egui::Context) {
        let columns = self.analyzer.columns();
        let bins = columns.front().map(Vec::len).unwrap_or(1).max(1);
        let sr = self.sample_rate as f32;
        let mut pixels = vec![crate::theme::track_bg(); TEXTURE_W * TEXTURE_H];
        let n = columns.len();
        for x in 0..TEXTURE_W {
            let col_idx = x.saturating_sub(TEXTURE_W - n);
            let Some(col) = columns.get(col_idx) else {
                continue;
            };
            for y in 0..TEXTURE_H {
                let t = 1.0 - y as f32 / (TEXTURE_H - 1) as f32;
                let freq = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t);
                let bin = ((freq / sr) * self.analyzer.fft_size() as f32) as usize;
                let bin = bin.min(bins - 1);
                let db = col.get(bin).copied().unwrap_or(MIN_DB);
                pixels[y * TEXTURE_W + x] = Self::color_for_db(db);
            }
        }
        let image = egui::ColorImage::new([TEXTURE_W, TEXTURE_H], pixels);
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
            None => {
                self.texture = Some(ctx.load_texture(
                    "meter_spectrogram",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        self.dirty = false;
    }
}

impl MeterModule for SpectrogramModule {
    fn update(&mut self, samples: &[f32], sample_rate: u32, _dt: f32) {
        self.sample_rate = sample_rate;
        if !samples.is_empty() && self.analyzer.process(samples) {
            self.dirty = true;
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        if self.dirty {
            self.rebuild_texture(ui.ctx());
        }
        let body = rect.shrink2(egui::vec2(6.0, 4.0));
        if body.width() < 20.0 || body.height() < 16.0 {
            return;
        }
        if let Some(texture) = &self.texture {
            ui.painter().image(
                texture.id(),
                body,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        // 频率参考线（不画满，避免盖住热力图）。
        for f in [100.0, 1_000.0, 10_000.0] {
            let t = (f / MIN_FREQ).ln() / (MAX_FREQ / MIN_FREQ).ln();
            let y = body.max.y - body.height() * t;
            ui.painter().hline(
                body.x_range(),
                y,
                egui::Stroke::new(1.0, crate::theme::text_primary().gamma_multiply(0.18)),
            );
            small_label(
                ui,
                egui::pos2(body.max.x - 2.0, y - 1.0),
                egui::Align2::RIGHT_BOTTOM,
                if f >= 1000.0 {
                    format!("{:.0}k", f / 1000.0)
                } else {
                    format!("{f:.0}")
                },
                crate::theme::text_muted(),
            );
        }
        small_label(
            ui,
            egui::pos2(body.min.x + 2.0, body.min.y + 1.0),
            egui::Align2::LEFT_TOP,
            "Hz",
            crate::theme::text_muted(),
        );
    }

    fn reset(&mut self) {
        self.analyzer.reset();
        self.dirty = true;
    }
}
