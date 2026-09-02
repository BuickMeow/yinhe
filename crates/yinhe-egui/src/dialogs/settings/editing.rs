use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::audio_settings::{OverlapBlockedBehavior, QuickDeleteMode};

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_editing_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.editing.heading").as_ref());
    ui.add_space(8.0);

    setting_row(
        ui,
        t!("settings.editing.allow_overlap").as_ref(),
        t!("settings.editing.allow_overlap_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.allow_overlapping_notes)
                .on_hover_text(t!("settings.editing.allow_overlap_hint").as_ref())
                .changed()
            {
                changed = true;
            }
        },
    );

    if !settings.allow_overlapping_notes {
        setting_row(
            ui,
            t!("settings.editing.blocked_behavior").as_ref(),
            t!("settings.editing.blocked_behavior_desc").as_ref(),
            |ui| {
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
            },
        );
    }

    setting_row(
        ui,
        t!("settings.editing.quick_delete").as_ref(),
        t!("settings.editing.quick_delete_desc").as_ref(),
        |ui| {
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
        },
    );

    changed
}
