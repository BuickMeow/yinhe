use eframe::egui;
use yinhe_meter::LoudnessAnalyzer;

use super::{MeterModule, lufs_text, plot_bg, small_label};

/// LUFS 显示范围：0（顶）到 -50（底）。
const MIN_LUFS: f32 = -50.0;

pub(super) struct LoudnessModule {
    analyzer: LoudnessAnalyzer,
}

impl LoudnessModule {
    pub(super) fn new(sample_rate: u32) -> Self {
        Self {
            analyzer: LoudnessAnalyzer::new(sample_rate),
        }
    }
}

fn lufs_frac(v: f32) -> f32 {
    if v.is_finite() {
        ((v - MIN_LUFS) / -MIN_LUFS).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

impl MeterModule for LoudnessModule {
    fn update(&mut self, samples: &[f32], _sample_rate: u32, _dt: f32) {
        if !samples.is_empty() {
            self.analyzer.process(samples);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        plot_bg(ui, rect);
        let body = rect.shrink2(egui::vec2(8.0, 4.0));
        if body.width() < 40.0 || body.height() < 60.0 {
            return;
        }
        let reading = self.analyzer.reading();

        // 右上角重置积分响度。
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

        // 顶部 M / S / I 数值。
        let top_h = 30.0;
        let values = [
            ("M", reading.momentary),
            ("S", reading.short_term),
            ("I", reading.integrated),
        ];
        for (i, (label, value)) in values.iter().enumerate() {
            let x = body.min.x + body.width() * i as f32 / 3.0;
            small_label(
                ui,
                egui::pos2(x, body.min.y),
                egui::Align2::LEFT_TOP,
                *label,
                crate::theme::text_muted(),
            );
            ui.painter().text(
                egui::pos2(x, body.min.y + 11.0),
                egui::Align2::LEFT_TOP,
                lufs_text(*value),
                egui::FontId::proportional(crate::theme::SMALL_FONT + 2.0),
                crate::theme::text_primary(),
            );
        }

        let meter_top = body.min.y + top_h;
        let meter_bottom = body.max.y;
        if meter_bottom - meter_top < 30.0 {
            return;
        }
        let label_w = 26.0;
        let meters_x = body.min.x + label_w;
        let meters_w = (body.max.x - 4.0) - meters_x;
        if meters_w < 16.0 {
            return;
        }

        // 左侧 LUFS 刻度。
        for db in [0.0_f32, -6.0, -12.0, -24.0, -36.0, -50.0] {
            let y = meter_bottom - (meter_bottom - meter_top) * lufs_frac(db);
            ui.painter().hline(
                egui::Rangef::new(body.min.x, body.min.x + 3.0),
                y,
                egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.5)),
            );
            small_label(
                ui,
                egui::pos2(body.min.x + 5.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{db:.0}"),
                crate::theme::text_muted(),
            );
        }

        // 两根垂直条：M（momentary）/ S（short-term）。
        let bar_w = ((meters_w - 4.0) / 2.0).max(4.0);
        for (i, value) in [reading.momentary, reading.short_term].iter().enumerate() {
            let x0 = meters_x + i as f32 * (bar_w + 4.0);
            let bar = egui::Rect::from_min_max(
                egui::pos2(x0, meter_top),
                egui::pos2(x0 + bar_w, meter_bottom),
            );
            ui.painter()
                .rect_filled(bar, 0.0, crate::theme::line_fg().gamma_multiply(0.22));
            let frac = lufs_frac(*value);
            if frac > 0.0 {
                let fill = egui::Rect::from_min_max(
                    egui::pos2(x0, meter_bottom - (meter_bottom - meter_top) * frac),
                    egui::pos2(x0 + bar_w, meter_bottom),
                );
                ui.painter()
                    .rect_filled(fill, 0.0, crate::theme::accent_active());
            }
        }

        // -14 LUFS 目标线横跨两根条。
        let target_y = meter_bottom - (meter_bottom - meter_top) * lufs_frac(-14.0);
        ui.painter().hline(
            egui::Rangef::new(meters_x, meters_x + bar_w * 2.0 + 4.0),
            target_y,
            egui::Stroke::new(1.0, crate::theme::text_primary().gamma_multiply(0.45)),
        );
        small_label(
            ui,
            egui::pos2(meters_x + bar_w * 2.0 + 4.0, target_y - 2.0),
            egui::Align2::RIGHT_BOTTOM,
            "-14",
            crate::theme::text_muted(),
        );
    }

    fn reset(&mut self) {
        self.analyzer.reset();
    }
}
