use std::path::PathBuf;

use eframe::egui;
use egui_material_icons::icons::*;
use serde::{Deserialize, Serialize};

use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub output_device_name: Option<String>,
    pub sample_rate: u32,
    pub default_sf2_path: String,
    #[serde(skip)]
    pub show_settings: bool,
    #[serde(skip)]
    available_devices: Vec<String>,
    #[serde(skip)]
    available_sample_rates: Vec<u32>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        let available_devices = list_output_devices();
        let default_device = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.name().ok());
        let sample_rates = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.default_output_config().ok())
            .map(|cfg| vec![cfg.sample_rate()])
            .unwrap_or_else(|| vec![44100, 48000, 96000]);

        Self {
            output_device_name: default_device,
            sample_rate: 48000,
            default_sf2_path: String::new(),
            show_settings: false,
            available_devices,
            available_sample_rates: sample_rates,
        }
    }
}

fn config_path() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yinhe");
    std::fs::create_dir_all(&dir).ok();
    dir.join("settings.json")
}

impl AudioSettings {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(json) => match serde_json::from_str::<AudioSettings>(&json) {
                    Ok(mut s) => {
                        s.available_devices = list_output_devices();
                        s.available_sample_rates = cpal::default_host()
                            .default_output_device()
                            .and_then(|d| d.default_output_config().ok())
                            .map(|cfg| vec![cfg.sample_rate()])
                            .unwrap_or_else(|| vec![44100, 48000, 96000]);
                        return s;
                    }
                    Err(e) => {
                        eprintln!("Failed to parse settings: {}", e);
                    }
                },
                Err(e) => {
                    eprintln!("Failed to read settings file: {}", e);
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = config_path();
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    eprintln!("Failed to save settings: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to serialize settings: {}", e);
            }
        }
    }
}

fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect::<Vec<_>>())
        .unwrap_or_default()
}

pub fn show(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    if !settings.show_settings {
        return false;
    }

    let mut changed = false;
    let mut should_close = false;

    let resp = egui::Window::new("设置")
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .default_height(400.0)
        .show(ui.ctx(), |ui| {
            ui.heading("音频");
            ui.add_space(8.0);

            egui::Grid::new("audio_settings_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("输出设备");
                    let current_device = settings
                        .output_device_name
                        .as_deref()
                        .unwrap_or("默认设备");
                    egui::ComboBox::from_id_salt("output_device")
                        .selected_text(current_device)
                        .show_ui(ui, |ui| {
                            for device_name in &settings.available_devices {
                                let selected =
                                    settings.output_device_name.as_ref() == Some(device_name);
                                if ui.selectable_label(selected, device_name).clicked() {
                                    settings.output_device_name = Some(device_name.clone());
                                    changed = true;
                                }
                            }
                            let is_default = settings.output_device_name.is_none();
                            if ui.selectable_label(is_default, "默认设备").clicked() {
                                settings.output_device_name = None;
                                changed = true;
                            }
                        });
                    ui.end_row();

                    ui.label("采样率");
                    let sr_label = format!("{} Hz", settings.sample_rate);
                    egui::ComboBox::from_id_salt("sample_rate")
                        .selected_text(&sr_label)
                        .show_ui(ui, |ui| {
                            for &sr in &settings.available_sample_rates {
                                let selected = settings.sample_rate == sr;
                                if ui
                                    .selectable_label(selected, format!("{} Hz", sr))
                                    .clicked()
                                {
                                    settings.sample_rate = sr;
                                    changed = true;
                                }
                            }
                        });
                    ui.end_row();
                });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            ui.heading("音色库");
            ui.add_space(8.0);

            egui::Grid::new("sf_settings_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("默认 SF2 路径");
                    ui.horizontal(|ui| {
                        let path_text = if settings.default_sf2_path.is_empty() {
                            "未设置".to_string()
                        } else {
                            let display = std::path::Path::new(&settings.default_sf2_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| settings.default_sf2_path.clone());
                            display
                        };
                        ui.label(
                            egui::RichText::new(&path_text).color(
                                if settings.default_sf2_path.is_empty() {
                                    egui::Color32::from_gray(100)
                                } else {
                                    egui::Color32::from_gray(200)
                                },
                            ),
                        );
                        if ui.button("选择...").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("SoundFont", &["sf2", "sf3", "sfz"])
                                .pick_file()
                            {
                                settings.default_sf2_path = path.to_string_lossy().to_string();
                                changed = true;
                            }
                        }
                        if !settings.default_sf2_path.is_empty()
                            && ui
                                .button(ICON_DELETE.rich_text().size(14.0))
                                .on_hover_text("清除")
                                .clicked()
                        {
                            settings.default_sf2_path.clear();
                            changed = true;
                        }
                    });
                    ui.end_row();
                });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("刷新设备列表").clicked() {
                    settings.available_devices = list_output_devices();
                }
                if ui.button("关闭").clicked() {
                    should_close = true;
                }
            });
        });

    if should_close || resp.as_ref().map_or(false, |r| r.response.should_close()) {
        settings.show_settings = false;
    }

    if changed {
        settings.save();
    }

    changed
}
