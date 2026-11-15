//! 文件位置设置页：为每类文件对话框单独指定默认起始目录。

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::audio_settings::FileDialogKind;

use crate::audio_settings::AudioSettings;
use crate::dialogs::settings::setting_row_plain;

fn kind_label(kind: FileDialogKind) -> String {
    let key = match kind {
        FileDialogKind::Open => "settings.files.open",
        FileDialogKind::ImportAudio => "settings.files.import_audio",
        FileDialogKind::ImportMidi => "settings.files.import_midi",
        FileDialogKind::CoverImage => "settings.files.cover",
        FileDialogKind::SoundFont => "settings.files.soundfont",
        FileDialogKind::SaveProject => "settings.files.save_project",
        FileDialogKind::ExportMidi => "settings.files.export_midi",
        FileDialogKind::ExportAudio => "settings.files.export_audio",
        FileDialogKind::ExportArchive => "settings.files.export_archive",
    };
    t!(key).to_string()
}

pub fn show_files_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.cat.files").as_ref());
    ui.add_space(8.0);

    for kind in FileDialogKind::ALL {
        setting_row_plain(ui, &kind_label(kind), |ui| {
            // 右侧布局下先加的在最右：清除 → 选择 → 路径。
            if settings.dialog_dir(kind).is_some()
                && crate::widgets::flat::flat_button(ui, t!("common.clear").as_ref()).clicked()
            {
                settings.dialog_dirs.remove(&kind);
                changed = true;
            }
            if crate::widgets::flat::flat_button(ui, t!("settings.files.choose").as_ref()).clicked()
            {
                let start = settings.dialog_dir(kind).map(str::to_owned);
                let picked = crate::file_dialog::with_dir(start.as_deref()).pick_folder();
                if let Some(dir) = picked {
                    settings
                        .dialog_dirs
                        .insert(kind, dir.to_string_lossy().to_string());
                    changed = true;
                }
            }
            match settings.dialog_dir(kind) {
                Some(dir) => {
                    ui.add_sized(
                        [ui.available_width(), crate::widgets::control::h(ui.ctx())],
                        egui::Label::new(
                            egui::RichText::new(dir)
                                .size(crate::theme::SMALL_FONT)
                                .color(crate::theme::text_secondary()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(dir);
                }
                None => {
                    ui.label(
                        egui::RichText::new(t!("settings.files.system_default").as_ref())
                            .size(crate::theme::SMALL_FONT)
                            .color(crate::theme::text_muted()),
                    );
                }
            }
        });
    }

    changed
}
