use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_general_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.general.heading").as_ref());
    ui.add_space(8.0);

    setting_row(
        ui,
        t!("settings.factory_reset").as_ref(),
        t!("settings.factory_reset_desc").as_ref(),
        |ui| {
            if ui
                .button(
                    egui::RichText::new(t!("settings.factory_reset").as_ref())
                        .color(crate::theme::danger_text()),
                )
                .clicked()
            {
                let default_settings = AudioSettings::default();
                let devices = std::mem::take(&mut settings.available_devices);
                let rates = std::mem::take(&mut settings.available_sample_rates);
                *settings = default_settings;
                settings.available_devices = devices;
                settings.available_sample_rates = rates;
                changed = true;
            }
        },
    );

    changed
}
