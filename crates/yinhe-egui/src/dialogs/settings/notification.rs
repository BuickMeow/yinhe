use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

/// 自动收起时长选项：秒数（None=不自动收起）。
fn collapse_options() -> Vec<(Option<u32>, String)> {
    vec![
        (Some(0), t!("settings.notification.dur_now").to_string()),
        (Some(5), t!("settings.notification.dur_5s").to_string()),
        (Some(10), t!("settings.notification.dur_10s").to_string()),
        (Some(15), t!("settings.notification.dur_15s").to_string()),
        (Some(20), t!("settings.notification.dur_20s").to_string()),
        (Some(30), t!("settings.notification.dur_30s").to_string()),
        (Some(60), t!("settings.notification.dur_60s").to_string()),
        (Some(300), t!("settings.notification.dur_5min").to_string()),
        (
            Some(1800),
            t!("settings.notification.dur_30min").to_string(),
        ),
        (None, t!("settings.notification.dur_never").to_string()),
    ]
}

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
            changed |= crate::widgets::combo::combo_select(
                ui,
                "toast_collapse_secs",
                &mut settings.toast_collapse_secs,
                200.0,
                &collapse_options(),
            );
        },
    );

    setting_row(
        ui,
        t!("settings.notification.action_collapse").as_ref(),
        t!("settings.notification.action_collapse_desc").as_ref(),
        |ui| {
            changed |= crate::widgets::combo::combo_select(
                ui,
                "toast_action_collapse_secs",
                &mut settings.toast_action_collapse_secs,
                200.0,
                &collapse_options(),
            );
        },
    );

    changed
}
