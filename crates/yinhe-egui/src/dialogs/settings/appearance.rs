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

    // DPI / 界面缩放——拖动中只更新滑块显示值，松手才真正应用：
    // 拖着一半就改 zoom 会让鼠标与控件错位乱窜。
    setting_row(
        ui,
        t!("settings.appearance.ui_scale").as_ref(),
        t!("settings.appearance.ui_scale_desc").as_ref(),
        |ui| {
            let mut scale = ui
                .data(|d| d.get_temp::<f32>(ui.scope_id().with("ui_scale_drag")))
                .unwrap_or(settings.ui_scale);
            let resp = crate::widgets::slider::control_slider(
                ui,
                &mut scale,
                0.75..=2.0,
                220.0,
                Some(0.05),
                true,
            );
            // 拖动中：暂存到临时数据，不动全局缩放。
            if resp.dragged() {
                ui.data_mut(|d| d.insert_temp(ui.scope_id().with("ui_scale_drag"), scale));
            }
            // 松手（或键盘/点击直接改值）：一次性提交。
            let committed = resp.drag_stopped() || (resp.changed() && !resp.dragged());
            if committed {
                ui.data_mut(|d| d.remove::<f32>(ui.scope_id().with("ui_scale_drag")));
                if (settings.ui_scale - scale).abs() > f32::EPSILON {
                    settings.ui_scale = scale;
                    crate::scaling::apply_ui_scale(main_ctx, scale);
                    changed = true;
                }
            }
            if crate::widgets::flat::flat_button_filled_auto(
                ui,
                t!("settings.appearance.reset_scale").as_ref(),
                true,
            )
            .clicked()
            {
                ui.data_mut(|d| d.remove::<f32>(ui.scope_id().with("ui_scale_drag")));
                settings.ui_scale = 1.0;
                crate::scaling::apply_ui_scale(main_ctx, 1.0);
                changed = true;
            }
        },
    );

    // 字体大小——同上仅松手应用。
    setting_row(
        ui,
        t!("settings.appearance.font_scale").as_ref(),
        t!("settings.appearance.font_scale_desc").as_ref(),
        |ui| {
            let mut fscale = ui
                .data(|d| d.get_temp::<f32>(ui.scope_id().with("font_scale_drag")))
                .unwrap_or(settings.font_scale);
            let resp = crate::widgets::slider::control_slider(
                ui,
                &mut fscale,
                0.75..=2.0,
                220.0,
                Some(0.05),
                true,
            );
            if resp.dragged() {
                ui.data_mut(|d| d.insert_temp(ui.scope_id().with("font_scale_drag"), fscale));
            }
            let committed = resp.drag_stopped() || (resp.changed() && !resp.dragged());
            if committed {
                ui.data_mut(|d| d.remove::<f32>(ui.scope_id().with("font_scale_drag")));
                if (settings.font_scale - fscale).abs() > f32::EPSILON {
                    settings.font_scale = fscale;
                    crate::scaling::apply_font_scale(main_ctx, fscale);
                    changed = true;
                }
            }
            if crate::widgets::flat::flat_button_filled_auto(
                ui,
                t!("settings.appearance.reset_scale").as_ref(),
                true,
            )
            .clicked()
            {
                ui.data_mut(|d| d.remove::<f32>(ui.scope_id().with("font_scale_drag")));
                settings.font_scale = 1.0;
                crate::scaling::apply_font_scale(main_ctx, 1.0);
                changed = true;
            }
        },
    );

    changed
}
