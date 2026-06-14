use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use cpal::traits::{DeviceTrait, HostTrait};

use crate::right_panel::config::GlobalSfConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub output_device_name: Option<String>,
    pub sample_rate: u32,
    /// Kept for migration — no longer used directly.
    pub default_sf2_path: String,
    pub global_sf_config: GlobalSfConfig,
    /// 0=原始, 1=整数对齐, 2=子像素偏移
    /// 0=柱状(2px竖条), 1=矩形(填充), 2=空心矩形(边框)
    pub velocity_display_mode: u32,
    pub scroll_mode: u32,
    /// 最小边框宽度(像素), 0=不设下限
    pub min_border_width: f32,
    #[serde(skip)]
    pub show_settings: bool,
    #[serde(skip)]
    available_devices: Vec<String>,
    #[serde(skip)]
    available_sample_rates: Vec<u32>,
}

/// Query the default output device for its default sample rate and all
/// supported sample rates. Falls back to `(48000, [44100, 48000, 96000])`
/// when no device is available.
fn discover_sample_rates() -> (u32, Vec<u32>) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        return (48000, vec![44100, 48000, 96000]);
    };

    let default_rate = device
        .default_output_config()
        .ok()
        .map(|cfg| cfg.sample_rate())
        .unwrap_or(48000);

    let supported_rates: Vec<u32> = device
        .supported_output_configs()
        .ok()
        .map(|configs| {
            configs
                .flat_map(|cfg| {
                    let min = cfg.min_sample_rate();
                    let max = cfg.max_sample_rate();
                    (min..=max).step_by(1000)
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();

    if supported_rates.is_empty() {
        (default_rate, vec![default_rate])
    } else {
        (default_rate, supported_rates)
    }
}

impl Default for AudioSettings {
    fn default() -> Self {
        let available_devices = list_output_devices();
        let default_device = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.description().ok().map(|desc| desc.to_string()));
        let (default_rate, available_sample_rates) = discover_sample_rates();

        Self {
            output_device_name: default_device,
            sample_rate: default_rate,
            default_sf2_path: String::new(),
            global_sf_config: GlobalSfConfig::builtin_default(),
            scroll_mode: 0,
            min_border_width: 0.0,
            velocity_display_mode: 0,
            show_settings: false,
            available_devices,
            available_sample_rates,
        }
    }
}

fn config_path() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yinhe");
    std::fs::create_dir_all(&dir).ok();
    dir.join("yinhe_settings.json")
}

impl AudioSettings {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(json) => match serde_json::from_str::<AudioSettings>(&json) {
                    Ok(mut s) => {
                        s.available_devices = list_output_devices();
                        let (default_rate, available_sample_rates) = discover_sample_rates();
                        s.available_sample_rates = available_sample_rates;
                        // Update sample rate to device default if the saved
                        // value is not in the newly discovered list.
                        if !s.available_sample_rates.contains(&s.sample_rate) {
                            s.sample_rate = default_rate;
                        }
                        // Migrate old default_sf2_path into global config
                        if !s.default_sf2_path.is_empty() && s.global_sf_config.ports[0].is_empty()
                        {
                            s.global_sf_config = std::mem::take(&mut s.global_sf_config)
                                .with_fallback_path(&s.default_sf2_path);
                        }
                        return s;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse settings: {}", e);
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read settings file: {}", e);
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
                    tracing::error!("Failed to save settings: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize settings: {}", e);
            }
        }
    }
}

fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.description().ok().map(|desc| desc.to_string())).collect::<Vec<_>>())
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
                    let current_device =
                        settings.output_device_name.as_deref().unwrap_or("默认设备");
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

            ui.heading("渲染");
            ui.add_space(8.0);

            egui::Grid::new("render_settings_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("滚动模式");
                    let mode_names = ["原始", "整数对齐", "子像素偏移"];
                    let current = settings.scroll_mode as usize;
                    egui::ComboBox::from_id_salt("scroll_mode")
                        .selected_text(mode_names[current])
                        .show_ui(ui, |ui| {
                            for (i, name) in mode_names.iter().enumerate() {
                                let selected = settings.scroll_mode == i as u32;
                                if ui.selectable_label(selected, *name).clicked() {
                                    settings.scroll_mode = i as u32;
                                    changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    ui.label("Velocity显示");
                    let vel_names = ["柱状", "矩形", "空心矩形"];
                    let current_vel = settings.velocity_display_mode as usize;
                    egui::ComboBox::from_id_salt("velocity_display_mode")
                        .selected_text(vel_names[current_vel])
                        .show_ui(ui, |ui| {
                            for (i, name) in vel_names.iter().enumerate() {
                                let selected = settings.velocity_display_mode == i as u32;
                                if ui.selectable_label(selected, *name).clicked() {
                                    settings.velocity_display_mode = i as u32;
                                    changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    ui.label("最小边框宽度");
                    let mut bw = settings.min_border_width;
                    if ui
                        .add(egui::Slider::new(&mut bw, 0.0..=5.0).step_by(0.5))
                        .changed()
                    {
                        settings.min_border_width = bw;
                        changed = true;
                    }
                    ui.end_row();
                });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("刷新设备列表").clicked() {
                    settings.available_devices = list_output_devices();
                    let (_, rates) = discover_sample_rates();
                    settings.available_sample_rates = rates;
                }
                if ui.button("关闭").clicked() {
                    should_close = true;
                }
            });
        });

    if should_close || resp.as_ref().is_some_and(|r| r.response.should_close()) {
        settings.show_settings = false;
    }

    if changed {
        settings.save();
    }

    changed
}
