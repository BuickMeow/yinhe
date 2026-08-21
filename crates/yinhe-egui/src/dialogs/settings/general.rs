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
                let selected_label = match settings.overlap_blocked_behavior {
                    OverlapBlockedBehavior::ReplaceTarget => t!("pr_bar.blocked_replace"),
                    OverlapBlockedBehavior::DeleteOriginal => t!("pr_bar.blocked_delete"),
                    OverlapBlockedBehavior::KeepOriginal => t!("pr_bar.blocked_keep"),
                };
                egui::ComboBox::from_id_salt("blocked_behavior")
                    .selected_text(selected_label.as_ref())
                    .show_ui(ui, |ui| {
                        for &b in &[
                            OverlapBlockedBehavior::ReplaceTarget,
                            OverlapBlockedBehavior::DeleteOriginal,
                            OverlapBlockedBehavior::KeepOriginal,
                        ] {
                            let label = match b {
                                OverlapBlockedBehavior::ReplaceTarget => {
                                    t!("pr_bar.blocked_replace")
                                }
                                OverlapBlockedBehavior::DeleteOriginal => {
                                    t!("pr_bar.blocked_delete")
                                }
                                OverlapBlockedBehavior::KeepOriginal => t!("pr_bar.blocked_keep"),
                            };
                            let selected = settings.overlap_blocked_behavior == b;
                            if ui.selectable_label(selected, label.as_ref()).clicked() {
                                settings.overlap_blocked_behavior = b;
                                changed = true;
                            }
                        }
                    });
                ui.end_row();
            }
            ui.label(t!("settings.editing.quick_delete").as_ref());
            {
                use yinhe_editor_core::audio_settings::QuickDeleteMode;
                let selected_label = match settings.quick_delete_mode {
                    QuickDeleteMode::Off => t!("settings.editing.quick_delete.off"),
                    QuickDeleteMode::DoubleClick => t!("settings.editing.quick_delete.double"),
                    QuickDeleteMode::RightClick => t!("settings.editing.quick_delete.right"),
                    QuickDeleteMode::Both => t!("settings.editing.quick_delete.both"),
                };
                egui::ComboBox::from_id_salt("quick_delete_mode")
                    .selected_text(selected_label.as_ref())
                    .show_ui(ui, |ui| {
                        for &m in &[
                            QuickDeleteMode::Off,
                            QuickDeleteMode::DoubleClick,
                            QuickDeleteMode::RightClick,
                            QuickDeleteMode::Both,
                        ] {
                            let label = match m {
                                QuickDeleteMode::Off => t!("settings.editing.quick_delete.off"),
                                QuickDeleteMode::DoubleClick => {
                                    t!("settings.editing.quick_delete.double")
                                }
                                QuickDeleteMode::RightClick => {
                                    t!("settings.editing.quick_delete.right")
                                }
                                QuickDeleteMode::Both => t!("settings.editing.quick_delete.both"),
                            };
                            let selected = settings.quick_delete_mode == m;
                            if ui.selectable_label(selected, label.as_ref()).clicked() {
                                settings.quick_delete_mode = m;
                                changed = true;
                            }
                        }
                    });
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
