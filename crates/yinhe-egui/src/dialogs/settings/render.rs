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
        t!("settings.render.automation_density").as_ref(),
        t!("settings.render.automation_density_desc").as_ref(),
        |ui| {
            let mut density = settings.automation_event_density as i32;
            let drag = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut density)
                    .range(1..=480)
                    .speed(0.2)
                    .suffix(" tick"),
            );
            if drag.changed() {
                settings.automation_event_density = density.max(1) as u32;
                changed = true;
            }
        },
    );

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
            let mut bw = settings.min_border_width;
            if ui
                .add(egui::Slider::new(&mut bw, 0.0..=5.0).step_by(0.5))
                .changed()
            {
                settings.min_border_width = bw;
                changed = true;
            }
        },
    );

    setting_row(
        ui,
        t!("settings.render.content_opacity").as_ref(),
        t!("settings.render.content_opacity_desc").as_ref(),
        |ui| {
            let mut op = settings.content_opacity;
            if ui.add(egui::Slider::new(&mut op, 0.0..=1.0)).changed() {
                settings.content_opacity = op;
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

    changed
}
