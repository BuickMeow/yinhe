use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;

pub fn show_language_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.language").as_ref());
    ui.add_space(8.0);
    egui::Grid::new("language_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.language").as_ref());
            let locales = [
                ("zh-CN", "简体中文"),
                ("zh-HK", "繁體中文（香港）"),
                ("zh-TW", "繁體中文（台灣）"),
                ("bo-CN", "བོད་སྐད།"),
                ("en-US", "English"),
                ("de-DE", "Deutsch"),
                ("es-ES", "Español"),
                ("fr-FR", "Français"),
                ("it-IT", "Italiano"),
                ("pl-PL", "Polski"),
                ("pt-PT", "Português (Portugal)"),
                ("pt-BR", "Português (Brasil)"),
                ("ru-RU", "Русский"),
                ("tr-TR", "Türkçe"),
                ("uk-UA", "Українська"),
                ("fil-PH", "Filipino"),
                ("ja-JP", "日本語"),
                ("ko-KR", "한국어"),
                ("hi-IN", "हिन्दी"),
                ("id-ID", "Bahasa Indonesia"),
                ("ms-MY", "Bahasa Melayu"),
                ("th-TH", "ไทย"),
                ("vi-VN", "Tiếng Việt"),
                ("lo-LA", "ລາວ"),
                ("my-MM", "မြန်မာ"),
                ("km-KH", "ខ្មែរ"),
            ];
            let current = locales
                .iter()
                .find(|(code, _)| *code == settings.locale)
                .map(|(_, name)| *name)
                .unwrap_or("简体中文");
            egui::ComboBox::from_id_salt("locale_select")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (code, name) in locales {
                        let selected = settings.locale == code;
                        if ui.selectable_label(selected, name).clicked() {
                            settings.locale = code.to_string();
                            rust_i18n::set_locale(code);
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            // 音轨名编码（MIDI 文件内文本的语言编码，归类到语言设置）
            ui.label(t!("settings.midi_import.encoding").as_ref());
            egui::ComboBox::from_id_salt("midi_import_encoding")
                .selected_text(settings.midi_import_encoding.label())
                .show_ui(ui, |ui| {
                    for &enc in yinhe_midi::MidiImportEncoding::ALL {
                        let selected = settings.midi_import_encoding == enc;
                        if ui.selectable_label(selected, enc.label()).clicked() {
                            settings.midi_import_encoding = enc;
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });
    changed
}
