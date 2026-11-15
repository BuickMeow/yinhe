use eframe::egui;
use rust_i18n::t;
use yinhe_meter::WaveformHistory;

use super::{MeterModule, grid, plot_bg, small_label};

pub(super) struct WaveformModule {
    history: WaveformHistory,
    sample_rate: u32,
    window_secs: f32,
    /// 放大倍率：>1 显示更短时间窗。
    zoom: f32,
}

impl WaveformModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            // 历史保留 20 秒，缩放缩小时仍能看到更长时间。
            history: WaveformHistory::new(sample_rate, 20.0, 512),
            sample_rate,
            window_secs: 5.0,
            zoom: 1.0,
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
        if ui.rect_contains_pointer(rect) {
            let zoom_delta = ui.input(|i| i.zoom_delta());
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let factor = zoom_delta * (1.0 + scroll * 0.002);
            if (factor - 1.0).abs() > 1e-4 {
                self.zoom = (self.zoom * factor).clamp(0.01, 1000.0);
            }
        }
        if ui.rect_contains_pointer(rect)
            && ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            })
        {
            self.zoom = 1.0;
        }

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
        let visible_secs = (self.window_secs / self.zoom).clamp(0.05, 20.0);
        let max_columns = self.history.max_columns();
        let visible = ((max_columns as f32 * visible_secs / 20.0).round() as usize).max(1);
        let cols = self.history.columns();
        let n = cols.len();
        if n == 0 {
            return;
        }
        let half = body.height() * 0.46;
        let col_w = body.width() / visible as f32;
        // 固定时间比例、最新在右：启动时左侧留白，而不是把少量列拉伸成 5 秒。
        for (i, col) in cols.iter().enumerate() {
            let x = body.max.x - (n - i) as f32 * col_w;
            if x + col_w < body.min.x {
                continue;
            }
            let y0 = center - col.max_l.clamp(-1.0, 1.0) * half;
            let y1 = center - col.min_l.clamp(-1.0, 1.0) * half;
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x.max(body.min.x), y0.min(y1)),
                    egui::pos2((x + col_w).min(body.max.x), y0.max(y1)),
                ),
                0.0,
                crate::theme::accent_active().gamma_multiply(0.7),
            );
        }
        small_label(
            ui,
            egui::pos2(body.min.x + 2.0, body.min.y + 1.0),
            egui::Align2::LEFT_TOP,
            if visible_secs >= 1.0 {
                format!("{visible_secs:.1} s")
            } else {
                format!("{:.0} ms", visible_secs * 1000.0)
            },
            crate::theme::text_muted(),
        );
    }

    fn reset(&mut self) {
        self.history.reset();
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(t!("meter.time_window"));
            let mut secs = self.window_secs;
            egui::ComboBox::from_id_salt("meter_waveform_window")
                .selected_text(format!("{secs:.0} s"))
                .show_ui(ui, |ui| {
                    for candidate in [2.0_f32, 5.0, 10.0, 20.0] {
                        if ui
                            .selectable_value(&mut secs, candidate, format!("{candidate:.0} s"))
                            .changed()
                        {
                            self.window_secs = candidate;
                            self.history = WaveformHistory::new(self.sample_rate, candidate, 512);
                            self.zoom = 1.0;
                        }
                    }
                });
        });
    }
}
