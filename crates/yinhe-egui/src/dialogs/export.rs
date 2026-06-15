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

/// Result from the export settings dialog.
pub(crate) struct ExportSettings {
    pub started: bool,
    pub layer_count: u32,
    pub sample_rate: u32,
}

/// Show the export settings dialog (bit depth + layer count + sample rate).
pub(crate) fn show_export_settings_dialog(
    ctx: &egui::Context,
    bit_depth: &mut yinhe_audio::export::WavBitDepth,
    layer_count: &mut u32,
    sample_rate: &mut u32,
    global_sample_rate: u32,
    open: &mut bool,
) -> ExportSettings {
    let mut result = ExportSettings {
        started: false,
        layer_count: *layer_count,
        sample_rate: *sample_rate,
    };

    let sample_rates: [u32; 5] = [0, 44100, 48000, 96000, 192000];

    let mut dialog_open = *open;
    egui::Window::new("导出音频")
        .open(&mut dialog_open)
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

                ui.horizontal(|ui| {
                    ui.label("采样率：");
                    let sr_text = if *sample_rate == 0 {
                        format!("跟随全局 ({} Hz)", global_sample_rate)
                    } else {
                        format!("{} Hz", sample_rate)
                    };
                    egui::ComboBox::from_id_salt("export_sample_rate")
                        .selected_text(&sr_text)
                        .show_ui(ui, |ui| {
                            for &sr in &sample_rates {
                                let label = if sr == 0 {
                                    format!("跟随全局 ({} Hz)", global_sample_rate)
                                } else {
                                    format!("{} Hz", sr)
                                };
                                let selected = *sample_rate == sr;
                                if ui.selectable_label(selected, label).clicked() {
                                    *sample_rate = sr;
                                }
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("XSynth层数：");
                    let mut layers = *layer_count as usize;
                    ui.add(
                        egui::DragValue::new(&mut layers)
                            .range(0..=128)
                            .speed(1.0),
                    );
                    *layer_count = layers as u32;
                    if *layer_count == 0 {
                        ui.label("无限制");
                    }
                });

                ui.add_space(12.0);

                if ui.button("导出").clicked() {
                    result.started = true;
                    result.layer_count = *layer_count;
                    result.sample_rate = *sample_rate;
                }

                ui.add_space(8.0);
            });
        });

    if !dialog_open || result.started {
        *open = false;
    }
    result
}

/// Show the export progress overlay (progress bar + status text).
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
