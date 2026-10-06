use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_render_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.render.heading").as_ref());
    ui.add_space(8.0);

    setting_row(
        ui,
        t!("settings.render.note_outline").as_ref(),
        t!("settings.render.note_outline_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.note_outline).changed() {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.render.min_border_width").as_ref(),
        t!("settings.render.min_border_width_desc").as_ref(),
        |ui| {
            if crate::widgets::stepper::stepper(&mut settings.min_border_width)
                .range(0.0..=5.0)
                .step(0.1)
                .decimals(2)
                .width(110.0)
                .show(ui)
                .changed()
            {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.render.gpu_cull").as_ref(),
        t!("settings.render.gpu_cull_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.use_gpu_cull).changed() {
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.render.lod").as_ref(),
        t!("settings.render.lod_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.lod_enabled).changed() {
                changed = true;
            }
        },
    );

    changed
}
