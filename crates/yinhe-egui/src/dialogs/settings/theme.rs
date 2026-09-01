use eframe::egui;
use rust_i18n::t;
use yinhe_theme::base::{BaseColors, Rgba};

use crate::audio_settings::AudioSettings;

/// 编辑一个标准色（调色板弹窗：RGBA/HSV 数值可切换 ↔ 主题 Rgba）。
fn edit_std_color(ui: &mut egui::Ui, label: &str, rgba: &mut Rgba) -> bool {
    ui.label(label);
    let mut c = rgba.to_color32();
    let changed = crate::widgets::color_picker::color_edit_button(ui, &mut c).changed();
    if changed {
        *rgba = Rgba::from_color32(c);
    }
    changed
}

// ── 各分类内容 ──

pub fn show_theme_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    ui.heading(t!("settings.theme.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("theme_preset_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.theme.preset").as_ref());
            // 预设列表来自 BaseColors::PRESETS，i18n key 为 settings.theme.<name>
            //（name 中的 '-' 换成 '_'），末尾追加"自定义"。
            let preset_names: Vec<(String, String)> = BaseColors::PRESETS
                .iter()
                .map(|(n, _)| {
                    let key = format!("settings.theme.{}", n.replace('-', "_"));
                    (n.to_string(), t!(key.as_str()).to_string())
                })
                .chain(std::iter::once((
                    "custom".to_string(),
                    t!("settings.theme.custom").to_string(),
                )))
                .collect();
            let current_preset = preset_names
                .iter()
                .find(|(n, _)| *n == settings.theme_preset)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| t!("settings.theme.custom").to_string());
            egui::ComboBox::from_id_salt("theme_preset")
                .selected_text(current_preset)
                .show_ui(ui, |ui| {
                    for (name, label) in preset_names {
                        let selected = settings.theme_preset == name;
                        if ui.selectable_label(selected, label).clicked() {
                            settings.theme_preset = name.clone();
                            if let Some(base) = BaseColors::preset_by_name(&name) {
                                settings.theme_base = base;
                            }
                            crate::theme::set_theme(settings.theme_base);
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });

    ui.add_space(4.0);
    let mut base = settings.theme_base;
    let mut base_changed = false;
    egui::Grid::new("theme_colors_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            base_changed |= edit_std_color(ui, t!("settings.theme.bg").as_ref(), &mut base.bg);
            ui.end_row();
            base_changed |= edit_std_color(ui, t!("settings.theme.text").as_ref(), &mut base.text);
            ui.end_row();
            base_changed |=
                edit_std_color(ui, t!("settings.theme.accent").as_ref(), &mut base.accent);
            ui.end_row();
        });
    if base_changed {
        settings.theme_base = base;
        settings.theme_preset = "custom".to_string();
        crate::theme::set_theme(base);
        changed = true;
    }

    // 界面缩放：拖动中不缩放（缩放会让滑条自身位置来回跑），松手才应用
    egui::Grid::new("theme_scale_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label(t!("settings.theme.ui_scale").as_ref());
            ui.horizontal(|ui| {
                let mut scale = settings.ui_scale;
                let resp = ui.add(
                    egui::Slider::new(&mut scale, 0.75..=2.0)
                        .step_by(0.05)
                        .show_value(true),
                );
                if resp.changed() {
                    settings.ui_scale = scale;
                    changed = true;
                }
                if resp.drag_stopped() {
                    main_ctx.set_zoom_factor(settings.ui_scale);
                }
                if ui
                    .button(t!("settings.theme.reset_scale").as_ref())
                    .clicked()
                {
                    settings.ui_scale = 1.0;
                    main_ctx.set_zoom_factor(1.0);
                    changed = true;
                }
            });
            ui.end_row();
        });
    changed
}
