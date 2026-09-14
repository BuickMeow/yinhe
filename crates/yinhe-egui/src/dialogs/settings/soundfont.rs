//! 音色库设置页：**全局音色库**（所有未单独配置通道的默认音色库）。
//!
//! 单通道覆盖在设备栏 XSynth 卡片的「音色库」窗口里配置（随工程保存）。

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::config::SfEntry;

use crate::audio_settings::AudioSettings;

pub fn show_soundfont_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.cat.soundfont").as_ref());
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("settings.soundfont.hint"))
            .size(crate::theme::SMALL_FONT)
            .color(crate::theme::text_secondary()),
    );
    ui.add_space(8.0);

    // 工具栏必须在列表上方（sf_list 的滚动区占满剩余高度）。
    ui.horizontal(|ui| {
        if ui.button(t!("soundfont.add").as_ref()).clicked()
            && let Some(paths) = rfd::FileDialog::new()
                .add_filter("SoundFont", &["sf2", "sf3", "sfz"])
                .pick_files()
        {
            for path in paths {
                let name = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("SoundFont")
                    .to_string();
                settings.global_sf_config.entries.push(SfEntry {
                    path: path.to_string_lossy().to_string(),
                    name,
                    enabled: true,
                });
            }
            changed = true;
        }
        if ui.button(t!("common.clear").as_ref()).clicked() {
            settings.global_sf_config.entries.clear();
            changed = true;
        }
    });
    ui.add_space(4.0);

    changed |=
        crate::right_panel::sf_list::sf_list(ui, &mut settings.global_sf_config.entries, "global");
    changed
}
