use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;

pub fn show_language_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.language").as_ref());
    ui.add_space(8.0);

    let row_gap = 10.0;

    ui.horizontal(|ui| {
        ui.label(t!("settings.language").as_ref());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let locales = [
                ("zh-CN", "简体中文"),
                ("zh-HK", "繁體中文（香港）"),
                ("zh-TW", "繁體中文（台灣）"),
                ("en-US", "English"),
                ("ja-JP", "日本語"),
                ("ko-KR", "한국어"),
            ];
            let locale_opt: Vec<(String, String)> = locales
                .iter()
                .map(|(c, n)| (c.to_string(), n.to_string()))
                .collect();
            if crate::widgets::combo::combo_select_auto(
                ui,
                "locale_select",
                &mut settings.locale,
                &locale_opt,
            ) {
                rust_i18n::set_locale(&settings.locale);
                changed = true;
            }
        });
    });
    ui.add_space(row_gap);

    ui.horizontal(|ui| {
        ui.label(t!("settings.midi_import.encoding").as_ref());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enc_opt: Vec<(yinhe_midi::MidiImportEncoding, String)> =
                yinhe_midi::MidiImportEncoding::ALL
                    .iter()
                    .map(|&e| (e, e.label().to_string()))
                    .collect();
            if crate::widgets::combo::combo_select_auto(
                ui,
                "midi_import_encoding",
                &mut settings.midi_import_encoding,
                &enc_opt,
            ) {
                changed = true;
            }
        });
    });
    ui.add_space(row_gap);

    changed
}
