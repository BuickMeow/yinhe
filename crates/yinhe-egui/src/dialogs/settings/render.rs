use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::{AudioSettings, VelocityFillMode};
use crate::dialogs::settings::setting_row;

pub fn show_render_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

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
        t!("settings.render.velocity_style").as_ref(),
        t!("settings.render.velocity_style_desc").as_ref(),
        |ui| {
            let options = vec![
                (
                    VelocityFillMode::BottomUp,
                    t!("settings.render.velocity_style_bottom_up").to_string(),
                ),
                (
                    VelocityFillMode::LeftToRight,
                    t!("settings.render.velocity_style_left_right").to_string(),
                ),
                (
                    VelocityFillMode::ColorFade,
                    t!("settings.render.velocity_style_color_fade").to_string(),
                ),
            ];
            if crate::widgets::combo::combo_select_auto(
                ui,
                "velocity_fill_mode",
                &mut settings.velocity_fill_mode,
                &options,
            ) {
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
        t!("settings.render.gray_other_tracks").as_ref(),
        t!("settings.render.gray_other_tracks_desc").as_ref(),
        |ui| {
            if crate::widgets::switch::switch(ui, &mut settings.gray_other_tracks).changed() {
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
