use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_notification_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.cat.notification").as_ref());
    ui.add_space(8.0);

    setting_row(
        ui,
        t!("settings.notification.enabled").as_ref(),
        t!("settings.notification.enabled_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.toast_enabled).changed() {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.notification.collapse").as_ref(),
        t!("settings.notification.collapse_desc").as_ref(),
        |ui| {
            if crate::widgets::stepper::stepper(&mut settings.toast_collapse_secs)
                .range(0.0..=3600.0)
                .step(5.0)
                .suffix(" s")
                .decimals(1)
                .width(140.0)
                .show(ui)
                .changed()
            {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.notification.action_collapse").as_ref(),
        t!("settings.notification.action_collapse_desc").as_ref(),
        |ui| {
            if crate::widgets::stepper::stepper(&mut settings.toast_action_collapse_secs)
                .range(0.0..=3600.0)
                .step(5.0)
                .suffix(" s")
                .decimals(1)
                .width(140.0)
                .show(ui)
                .changed()
            {
                changed = true;
            }
        },
    );

    changed
}
