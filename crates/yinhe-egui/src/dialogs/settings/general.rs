use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::audio_settings::OverlapBlockedBehavior;

use crate::audio_settings::AudioSettings;

pub fn show_general_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.general.heading").as_ref());
    ui.add_space(8.0);

    // ── 编辑：重叠音符策略 ──
    ui.heading(t!("settings.editing.heading").as_ref());
    ui.add_space(4.0);
    egui::Grid::new("editing_overlap_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.editing.allow_overlap").as_ref());
            if crate::widgets::checkbox::checkbox(
                ui,
                &mut settings.allow_overlapping_notes,
                t!("settings.editing.allow_overlap_hint").as_ref(),
            )
            .changed()
            {
                changed = true;
            }
            ui.end_row();
            if !settings.allow_overlapping_notes {
                ui.label(t!("settings.editing.blocked_behavior").as_ref());
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
                ui.end_row();
            }
            ui.label(t!("settings.editing.quick_delete").as_ref());
            {
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
            }
            ui.end_row();
        });
    ui.add_space(8.0);

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
            // Preserve runtime fields (device lists, etc.)
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
