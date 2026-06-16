use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use cpal::traits::{DeviceTrait, HostTrait};

use crate::config::GlobalSfConfig;
use yinhe_midi::MidiImportEncoding;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub output_device_name: Option<String>,
    pub sample_rate: u32,
    /// Kept for migration — no longer used directly.
    pub default_sf2_path: String,
    pub global_sf_config: GlobalSfConfig,
    pub xsynth_layers: u32,
    /// 0=原始, 1=整数对齐, 2=子像素偏移
    /// 0=柱状(2px竖条), 1=矩形(填充), 2=空心矩形(边框)
    pub velocity_display_mode: u32,
    /// 0=柱状, 1=折线
    pub automation_display_mode: u32,
    /// 折线模式下是否显示圆点
    pub automation_show_dots: bool,
    pub scroll_mode: u32,
    /// 最小边框宽度(像素), 0=不设下限
    pub min_border_width: f32,
    /// MIDI 导入编码
    pub midi_import_encoding: MidiImportEncoding,
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

fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.description().ok().map(|desc| desc.to_string())).collect::<Vec<_>>())
        .unwrap_or_default()
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
            xsynth_layers: 4,
            scroll_mode: 0,
            min_border_width: 0.0,
            midi_import_encoding: MidiImportEncoding::Utf8,
            velocity_display_mode: 0,
            automation_display_mode: 0,
            automation_show_dots: true,
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

    pub fn available_devices(&self) -> &[String] {
        &self.available_devices
    }

    pub fn available_sample_rates(&self) -> &[u32] {
        &self.available_sample_rates
    }

    pub fn refresh_devices(&mut self) {
        self.available_devices = list_output_devices();
        let (_, rates) = discover_sample_rates();
        self.available_sample_rates = rates;
    }
}
