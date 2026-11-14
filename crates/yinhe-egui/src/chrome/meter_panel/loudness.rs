use eframe::egui;
use yinhe_meter::LoudnessAnalyzer;

use super::{MeterModule, db_text, lufs_text, plot_bg, small_label};
use crate::widgets::meter::{db_frac, gain_to_db};

pub(super) struct LoudnessModule {
    analyzer: LoudnessAnalyzer,
    peak: crate::mix::MeterState,
}

impl LoudnessModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            analyzer: LoudnessAnalyzer::new(sample_rate),
            peak: Default::default(),
        }
    }
}

impl MeterModule for LoudnessModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, dt: f32) {
        if !samples.is_empty() {
            self.analyzer.process(samples);
        }
        let r = self.analyzer.reading();
        self.peak.update((r.peak_l, r.peak_r), dt);
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(8.0, 4.0));
        if body.width() < 40.0 || body.height() < 30.0 {
            return;
        }
        let reading = self.analyzer.reading();
        let (level, hold) = (self.peak.level(), self.peak.hold());

        // 右侧重置积分响度。
        let reset_rect = egui::Rect::from_min_size(
            egui::pos2(body.max.x - 18.0, body.min.y),
            egui::vec2(18.0, 18.0),
        );
        let reset_resp = ui.interact(
            reset_rect,
            ui.scope_id().with("meter_loudness_reset"),
            egui::Sense::click(),
        );
        ui.painter().text(
            reset_rect.center(),
            egui::Align2::CENTER_CENTER,
            egui_material_icons::icons::ICON_RESTART_ALT.codepoint,
            egui::FontId::new(
                crate::theme::ICON_FONT_SM,
                egui_material_icons::icons::ICON_RESTART_ALT.font_family(),
            ),
            if reset_resp.hovered() {
                crate::theme::text_primary()
            } else {
                crate::theme::text_muted()
            },
        );
        crate::widgets::hint::hover(
            ui.ctx(),
            &reset_resp,
            rust_i18n::t!("hint.meter.reset_loudness"),
        );
        if reset_resp.clicked() {
            self.reset();
        }

        // 顶部三列 LUFS：M / S / I。
        let top = egui::Rect::from_min_max(
            body.min,
            egui::pos2(body.max.x - 22.0, body.min.y + body.height() * 0.52),
        );
        let values = [
            ("M", reading.momentary),
            ("S", reading.short_term),
            ("I", reading.integrated),
        ];
        for (i, (label, value)) in values.iter().enumerate() {
            let col = egui::Rect::from_min_size(
                egui::pos2(top.min.x + top.width() * i as f32 / 3.0, top.min.y),
                egui::vec2(top.width() / 3.0, top.height()),
            );
            small_label(
                ui,
                egui::pos2(col.min.x, col.min.y),
                egui::Align2::LEFT_TOP,
                *label,
                crate::theme::text_muted(),
            );
            ui.painter().text(
                egui::pos2(col.min.x, col.min.y + 12.0),
                egui::Align2::LEFT_TOP,
                lufs_text(*value),
                egui::FontId::proportional(crate::theme::SMALL_FONT + 3.0),
                crate::theme::text_primary(),
            );
        }

        // 底部：L/R peak 横条 + -14/-23 目标线。
        let meter_h = 10.0;
        let meter_y = body.max.y - meter_h;
        let meter_rect = egui::Rect::from_min_max(
            egui::pos2(body.min.x, meter_y),
            egui::pos2(body.max.x, body.max.y),
        );
        ui.painter().rect_filled(
            meter_rect,
            0.0,
            crate::theme::line_fg().gamma_multiply(0.25),
        );
        let bar_w = (meter_rect.width() - 2.0) * 0.5;
        for (i, (&v, &h)) in [level.0, level.1]
            .iter()
            .zip([hold.0, hold.1].iter())
            .enumerate()
        {
            let x0 = meter_rect.min.x + i as f32 * (bar_w + 2.0);
            let frac = db_frac(gain_to_db(v));
            if frac > 0.0 {
                let fill = egui::Rect::from_min_max(
                    egui::pos2(x0, meter_rect.max.y - meter_rect.height() * frac),
                    egui::pos2(x0 + bar_w, meter_rect.max.y),
                );
                ui.painter()
                    .rect_filled(fill, 0.0, crate::theme::accent_active());
            }
            let hf = db_frac(gain_to_db(h));
            if hf > frac + 0.002 {
                let y = meter_rect.max.y - meter_rect.height() * hf;
                ui.painter().hline(
                    egui::Rangef::new(x0, x0 + bar_w),
                    y,
                    egui::Stroke::new(1.0, crate::theme::text_primary()),
                );
            }
        }
        for target in [-14.0, -23.0] {
            let x = meter_rect.min.x + meter_rect.width() * db_frac(target);
            ui.painter().vline(
                x,
                meter_rect.y_range(),
                egui::Stroke::new(1.0, crate::theme::text_primary().gamma_multiply(0.55)),
            );
            small_label(
                ui,
                egui::pos2(x + 2.0, meter_rect.min.y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{target:.0}"),
                crate::theme::text_muted(),
            );
        }
        // 峰值数字放在目标线上方。
        small_label(
            ui,
            egui::pos2(meter_rect.min.x, meter_rect.min.y - 2.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "L {}  R {}",
                db_text(gain_to_db(level.0)),
                db_text(gain_to_db(level.1))
            ),
            crate::theme::text_secondary(),
        );
    }

    fn reset(&mut self) {
        self.analyzer.reset();
        self.peak = Default::default();
    }
}
