use std::sync::{Arc, Mutex};

use eframe::egui;

/// Shared export progress state, updated from the background thread.
#[derive(Clone)]
pub(crate) struct ExportProgress {
    pub visible: bool,
    pub progress: f32,
    pub status: String,
}

impl ExportProgress {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            visible: false,
            progress: 0.0,
            status: String::new(),
        }))
    }
}

/// Show the bit-depth selection dialog.
/// Returns `true` when the user clicks "导出".
pub(crate) fn show_bit_depth_dialog(
    ctx: &egui::Context,
    bit_depth: &mut yinhe_audio::export::WavBitDepth,
    open: &mut bool,
) -> bool {
    let mut started = false;

    let mut dialog_open = *open;
    egui::Window::new("导出音频")
        .open(&mut dialog_open)
        .order(egui::Order::Tooltip)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(280.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("位深度：");
                    let current = match bit_depth {
                        yinhe_audio::export::WavBitDepth::Bit16 => "16-bit",
                        yinhe_audio::export::WavBitDepth::Bit24 => "24-bit",
                        yinhe_audio::export::WavBitDepth::Bit32Float => "32-bit float",
                    };
                    egui::ComboBox::from_id_salt("export_bit_depth")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(
                                matches!(bit_depth, yinhe_audio::export::WavBitDepth::Bit16),
                                "16-bit",
                            )
                            .clicked()
                            {
                                *bit_depth = yinhe_audio::export::WavBitDepth::Bit16;
                            }
                            if ui.selectable_label(
                                matches!(bit_depth, yinhe_audio::export::WavBitDepth::Bit24),
                                "24-bit",
                            )
                            .clicked()
                            {
                                *bit_depth = yinhe_audio::export::WavBitDepth::Bit24;
                            }
                            if ui.selectable_label(
                                matches!(bit_depth, yinhe_audio::export::WavBitDepth::Bit32Float),
                                "32-bit float",
                            )
                            .clicked()
                            {
                                *bit_depth = yinhe_audio::export::WavBitDepth::Bit32Float;
                            }
                        });
                });

                ui.add_space(12.0);

                if ui.button("导出").clicked() {
                    started = true;
                }

                ui.add_space(8.0);
            });
        });

    if !dialog_open || started {
        *open = false;
    }
    started
}

/// Show the export progress overlay (spinner + progress bar + status text).
pub(crate) fn show_export_progress(ui: &egui::Ui, progress: &Arc<Mutex<ExportProgress>>) {
    let state = match progress.lock() {
        Ok(s) => s.clone(),
        Err(_) => return,
    };
    if !state.visible {
        return;
    }

    let ctx = ui.ctx();
    let area = egui::Area::new(egui::Id::new("export_progress_overlay"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .interactable(false);

    area.show(ctx, |ui| {
        egui::Frame::window(ui.style())
            .fill(egui::Color32::from_black_alpha(200))
            .corner_radius(8.0)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("导出音频中…")
                            .size(18.0)
                            .color(egui::Color32::WHITE),
                    );
                    ui.add_space(12.0);

                    // Progress bar
                    ui.add(
                        egui::ProgressBar::new(state.progress)
                            .desired_width(240.0)
                            .fill(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
                    );

                    ui.add_space(8.0);

                    if !state.status.is_empty() {
                        ui.label(
                            egui::RichText::new(&state.status)
                                .size(13.0)
                                .color(egui::Color32::LIGHT_GRAY),
                        );
                    }

                    ui.add_space(16.0);
                });
            });
    });
}
