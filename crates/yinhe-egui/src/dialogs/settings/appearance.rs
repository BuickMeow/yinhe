use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row;

pub fn show_appearance_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.appearance.heading").as_ref());
    ui.add_space(8.0);

    // DPI / 界面缩放——仅松手才写入并应用，避免拖动中 zoom 导致鼠标错位乱窜
    setting_row(
        ui,
        t!("settings.appearance.ui_scale").as_ref(),
        t!("settings.appearance.ui_scale_desc").as_ref(),
        |ui| {
            let mut scale = settings.ui_scale;
            let resp = ui.add(
                egui::Slider::new(&mut scale, 0.75..=2.0)
                    .step_by(0.05)
                    .show_value(true),
            );
            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                settings.ui_scale = scale;
                crate::scaling::apply_ui_scale(main_ctx, scale);
                changed = true;
            }
            if ui
                .button(t!("settings.appearance.reset_scale").as_ref())
                .clicked()
            {
                settings.ui_scale = 1.0;
                crate::scaling::apply_ui_scale(main_ctx, 1.0);
                changed = true;
            }
        },
    );

    // 字体大小——同上仅松手应用
    setting_row(
        ui,
        t!("settings.appearance.font_scale").as_ref(),
        t!("settings.appearance.font_scale_desc").as_ref(),
        |ui| {
            let mut fscale = settings.font_scale;
            let resp = ui.add(
                egui::Slider::new(&mut fscale, 0.75..=2.0)
                    .step_by(0.05)
                    .show_value(true),
            );
            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                settings.font_scale = fscale;
                crate::scaling::apply_font_scale(main_ctx, fscale);
                changed = true;
            }
            if ui
                .button(t!("settings.appearance.reset_scale").as_ref())
                .clicked()
            {
                settings.font_scale = 1.0;
                crate::scaling::apply_font_scale(main_ctx, 1.0);
                changed = true;
            }
        },
    );

    changed
}
