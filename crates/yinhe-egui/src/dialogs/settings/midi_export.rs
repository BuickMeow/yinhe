use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_midi_export_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.midi_export.heading").as_ref());
    ui.add_space(8.0);

    setting_row(
        ui,
        t!("settings.midi_export.encoding").as_ref(),
        t!("settings.midi_export.encoding_desc").as_ref(),
        |ui| {
            let enc_opt: Vec<(yinhe_midi::MidiImportEncoding, String)> =
                yinhe_midi::MidiImportEncoding::ALL
                    .iter()
                    .map(|&e| (e, e.label().to_string()))
                    .collect();
            if crate::widgets::combo::combo_select_auto(
                ui,
                "midi_export_encoding",
                &mut settings.midi_export_encoding,
                &enc_opt,
            ) {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.midi_export.curve_interpolate").as_ref(),
        t!("settings.midi_export.curve_interpolate_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.midi_export_curve_interpolate)
                .changed()
            {
                changed = true;
            }
        },
    );

    if settings.midi_export_curve_interpolate {
        setting_row(
            ui,
            t!("settings.midi_export.curve_density").as_ref(),
            t!("settings.midi_export.curve_density_desc").as_ref(),
            |ui| {
                let mut density = settings.midi_export_curve_density as i32;
                let drag = ui.add(
                    crate::widgets::numeric_input::decimal_drag_value(&mut density)
                        .range(1..=480)
                        .speed(0.2)
                        .suffix(" tick"),
                );
                if drag.changed() {
                    settings.midi_export_curve_density = density.max(1) as u32;
                    changed = true;
                }
            },
        );
    }

    setting_row(
        ui,
        t!("settings.midi_export.rpn_full").as_ref(),
        t!("settings.midi_export.rpn_full_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.midi_export_rpn_full).changed() {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.midi_export.strip_empty").as_ref(),
        t!("settings.midi_export.strip_empty_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.midi_export_strip_empty_tracks)
                .changed()
            {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.midi_export.dedup_overlaps").as_ref(),
        t!("settings.midi_export.dedup_overlaps_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.midi_export_dedup_overlaps)
                .changed()
            {
                changed = true;
            }
        },
    );

    changed
}
