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
        t!("settings.auto_save").as_ref(),
        t!("settings.auto_save_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.auto_save_enabled).changed() {
                changed = true;
            }
        },
    );

    let intervals: [u64; 5] = [60, 300, 600, 900, 1800];
    let current = settings.auto_save_interval_secs;
    let enabled = settings.auto_save_enabled;
    let current_label = t!("settings.auto_save.interval_minutes", n = current / 60).to_string();
    setting_row(ui, t!("settings.auto_save_interval").as_ref(), "", |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            crate::widgets::combo::combo_box(
                ui,
                "auto_save_interval",
                current_label.as_str(),
                140.0,
                |ui| {
                    for secs in intervals {
                        let label = t!("settings.auto_save.interval_minutes", n = secs / 60);
                        if crate::widgets::combo::combo_item(ui, current == secs, label.as_ref())
                            .clicked()
                        {
                            settings.auto_save_interval_secs = secs;
                            changed = true;
                        }
                    }
                },
            );
        });
    });

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
