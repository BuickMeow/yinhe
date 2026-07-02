use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::GlobalSfConfig;
use yinhe_mid2::MidiImportEncoding;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub output_device_name: Option<String>,
    pub sample_rate: u32,
    /// Kept for migration — no longer used directly.
    pub default_sf2_path: String,
    pub global_sf_config: GlobalSfConfig,
    pub xsynth_layers: u32,
    /// Audio buffer size in frames. 0 = system default (~512 on macOS).
    pub buffer_size: u32,
    /// 0=原始, 1=整数对齐, 2=子像素偏移
    /// 0=柱状(2px竖条), 1=矩形(填充), 2=空心矩形(边框)
    pub velocity_display_mode: u32,
    /// 0=柱状, 1=折线
    pub automation_display_mode: u32,
    /// 折线模式下是否显示圆点
    pub automation_show_dots: bool,
    /// 选中音符是否变色（默认关闭，仅靠选框标识选中状态）
    pub note_selection_highlight: bool,
    pub scroll_mode: u32,
    /// 最小边框宽度(像素), 0=不设下限
    pub min_border_width: f32,
    /// MIDI 导入编码
    pub midi_import_encoding: MidiImportEncoding,
    /// 触控板震动反馈
    pub haptic_enabled: bool,
    /// 震动强度 0.0~1.0
    pub haptic_intensity: f32,
    #[serde(skip)]
    pub show_settings: bool,
    #[serde(skip)]
    available_devices: Vec<String>,
    #[serde(skip)]
    available_sample_rates: Vec<u32>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device_name: None,
            sample_rate: 48000,
            default_sf2_path: String::new(),
            global_sf_config: GlobalSfConfig::builtin_default(),
            xsynth_layers: 4,
            buffer_size: 0,
            scroll_mode: 0,
            min_border_width: 0.0,
            midi_import_encoding: MidiImportEncoding::Utf8,
            velocity_display_mode: 0,
            automation_display_mode: 0,
            automation_show_dots: true,
            note_selection_highlight: false,
            haptic_enabled: true,
            haptic_intensity: 0.5,
            show_settings: false,
            available_devices: Vec::new(),
            available_sample_rates: Vec::new(),
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

    /// Populate device lists and adjust sample rate. Called after loading
    /// or defaulting, once the host has been queried.
    pub fn refresh_devices(
        &mut self,
        devices: Vec<String>,
        rates: Vec<u32>,
        default_rate: u32,
    ) {
        self.available_devices = devices;
        self.available_sample_rates = rates;
        if !self.available_sample_rates.contains(&self.sample_rate) {
            self.sample_rate = default_rate;
        }
    }
}
