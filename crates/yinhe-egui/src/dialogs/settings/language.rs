use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_language_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.language").as_ref());
    ui.add_space(8.0);

    setting_row(
        ui,
        t!("settings.language").as_ref(),
        t!("settings.language_desc").as_ref(),
        |ui| {
            let locales = [
                ("zh-CN", "简体中文"),
                ("zh-HK", "繁體中文（香港）"),
                ("zh-TW", "繁體中文（台灣）"),
                ("en-US", "English"),
                ("ja-JP", "日本語"),
                ("ko-KR", "한국어"),
                ("fr-FR", "Français"),
                ("de-DE", "Deutsch"),
                ("es-ES", "Español"),
                ("it-IT", "Italiano"),
                ("pt-BR", "Português (Brasil)"),
                ("ru-RU", "Русский"),
                ("pl-PL", "Polski"),
                ("cs-CZ", "Čeština"),
                ("tr-TR", "Türkçe"),
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
                // 按新语言重装系统字体（CJK 主字体随语言变化）。
                crate::fonts::install(ui.ctx(), yinhe_fonts::WEIGHT_MEDIUM, &settings.locale);
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.midi_import.encoding").as_ref(),
        t!("settings.midi_import.encoding_desc").as_ref(),
        |ui| {
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
        },
    );

    changed
}
