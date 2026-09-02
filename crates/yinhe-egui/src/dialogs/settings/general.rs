use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::audio_settings::OverlapBlockedBehavior;

use crate::audio_settings::AudioSettings;

pub fn show_general_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.general.heading").as_ref());
    ui.add_space(8.0);

    ui.heading(t!("settings.editing.heading").as_ref());
    ui.add_space(4.0);

    let row_gap = 10.0;

    // 允许重叠音符
    ui.horizontal(|ui| {
        ui.label(t!("settings.editing.allow_overlap").as_ref());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.allow_overlapping_notes)
                .on_hover_text(t!("settings.editing.allow_overlap_hint").as_ref())
                .changed()
            {
                changed = true;
            }
        });
    });
    ui.add_space(row_gap);

    if !settings.allow_overlapping_notes {
        ui.horizontal(|ui| {
            ui.label(t!("settings.editing.blocked_behavior").as_ref());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let blocked_opt = vec![
                    (
                        OverlapBlockedBehavior::ReplaceTarget,
                        t!("pr_bar.blocked_replace").to_string(),
                    ),
                    (
                        OverlapBlockedBehavior::DeleteOriginal,
                        t!("pr_bar.blocked_delete").to_string(),
                    ),
                    (
                        OverlapBlockedBehavior::KeepOriginal,
                        t!("pr_bar.blocked_keep").to_string(),
                    ),
                ];
                if crate::widgets::combo::combo_select_auto(
                    ui,
                    "blocked_behavior",
                    &mut settings.overlap_blocked_behavior,
                    &blocked_opt,
                ) {
                    changed = true;
                }
            });
        });
        ui.add_space(row_gap);
    }

    // 快速删除
    ui.horizontal(|ui| {
        ui.label(t!("settings.editing.quick_delete").as_ref());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            use yinhe_editor_core::audio_settings::QuickDeleteMode;
            let quick_opt = vec![
                (
                    QuickDeleteMode::Off,
                    t!("settings.editing.quick_delete.off").to_string(),
                ),
                (
                    QuickDeleteMode::DoubleClick,
                    t!("settings.editing.quick_delete.double").to_string(),
                ),
                (
                    QuickDeleteMode::RightClick,
                    t!("settings.editing.quick_delete.right").to_string(),
                ),
                (
                    QuickDeleteMode::Both,
                    t!("settings.editing.quick_delete.both").to_string(),
                ),
            ];
            if crate::widgets::combo::combo_select_auto(
                ui,
                "quick_delete_mode",
                &mut settings.quick_delete_mode,
                &quick_opt,
            ) {
                changed = true;
            }
        });
    });
    ui.add_space(row_gap);

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.horizontal(|ui| {
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
    });
    changed
}
