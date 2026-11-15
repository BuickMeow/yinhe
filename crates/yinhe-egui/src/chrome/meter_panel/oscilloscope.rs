use eframe::egui;
use rust_i18n::t;
use yinhe_meter::ScopeBuffer;

use super::{MeterModule, grid, plot_bg};

pub(super) struct OscilloscopeModule {
    buffer: ScopeBuffer,
    sample_rate: u32,
    window_frames: usize,
    window_ms: f32,
    samples: Vec<f32>,
    left: Vec<egui::Pos2>,
    right: Vec<egui::Pos2>,
}

impl OscilloscopeModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            buffer: ScopeBuffer::default_for(sample_rate),
            sample_rate,
            window_frames: ((sample_rate as usize * 20) / 1000).max(64),
            window_ms: 20.0,
            samples: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
        }
    }
}

impl MeterModule for OscilloscopeModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, _dt: f32) {
        if !samples.is_empty() {
            self.buffer.push(samples);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(6.0, 4.0));
        if body.width() < 20.0 || body.height() < 20.0 {
            return;
        }
        grid(ui.painter(), body, 4, 4);
        let center = body.center().y;
        ui.painter().hline(
            body.x_range(),
            center,
            egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.45)),
        );

        self.buffer
            .triggered_snapshot(self.window_frames, &mut self.samples);
        let frames = self.samples.len() / 2;
        if frames < 2 {
            return;
        }
        self.left.clear();
        self.right.clear();
        self.left.reserve(frames);
        self.right.reserve(frames);
        let amp = body.height() * 0.45;
        for (i, frame) in self.samples.as_chunks::<2>().0.iter().enumerate() {
            let x = body.min.x + body.width() * i as f32 / (frames - 1) as f32;
            self.left
                .push(egui::pos2(x, center - frame[0].clamp(-1.0, 1.0) * amp));
            self.right
                .push(egui::pos2(x, center - frame[1].clamp(-1.0, 1.0) * amp));
        }
        // 波形必须裁在模块内容区内，不能画到标题栏/相邻面板上。
        let painter = ui.painter().with_clip_rect(body);
        painter.add(egui::Shape::line(
            std::mem::take(&mut self.left),
            egui::Stroke::new(1.2, crate::theme::accent_active()),
        ));
        painter.add(egui::Shape::line(
            std::mem::take(&mut self.right),
            egui::Stroke::new(1.0, crate::theme::text_secondary().gamma_multiply(0.8)),
        ));
    }

    fn reset(&mut self) {
        self.buffer.reset();
        self.samples.clear();
        self.left.clear();
        self.right.clear();
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(t!("meter.time_window"));
            let mut ms = self.window_ms;
            egui::ComboBox::from_id_salt("meter_scope_window")
                .selected_text(format!("{ms:.0} ms"))
                .show_ui(ui, |ui| {
                    for candidate in [5.0_f32, 10.0, 20.0, 50.0] {
                        if ui
                            .selectable_value(&mut ms, candidate, format!("{candidate:.0} ms"))
                            .changed()
                        {
                            self.window_ms = candidate;
                            self.window_frames =
                                ((self.sample_rate as f32 * candidate) / 1000.0) as usize;
                            self.window_frames = self.window_frames.max(64);
                        }
                    }
                });
        });
    }
}
