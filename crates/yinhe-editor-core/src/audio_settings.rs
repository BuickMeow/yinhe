use serde::{Deserialize, Serialize};

use crate::config::GlobalSfConfig;
use crate::shortcuts::Keybindings;
use yinhe_midi::MidiImportEncoding;

pub mod edit;
pub mod layout;
pub mod persistence;
pub mod theme;

pub use edit::{OverlapBlockedBehavior, QuickDeleteMode};
pub use layout::LayoutSettings;
pub use theme::CustomTheme;

/// "最近修改的文件"列表上限
pub const RECENT_FILES_LIMIT: usize = 10;

fn default_toast_collapse_secs() -> Option<u32> {
    Some(5)
}

fn default_toast_action_collapse_secs() -> Option<u32> {
    Some(60)
}

fn default_toast_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub output_device_name: Option<String>,
    pub midi_input_device: Option<String>,
    pub sample_rate: u32,
    pub default_sf2_path: String,
    pub global_sf_config: GlobalSfConfig,
    pub xsynth_layers: u32,
    pub buffer_size: u32,
    pub automation_event_density: u32,
    pub note_outline: bool,
    pub allow_overlapping_notes: bool,
    pub overlap_blocked_behavior: OverlapBlockedBehavior,
    pub quick_delete_mode: QuickDeleteMode,
    pub min_border_width: f32,
    pub midi_import_encoding: MidiImportEncoding,
    pub midi_export_encoding: MidiImportEncoding,
    pub midi_export_rpn_full: bool,
    pub midi_export_curve_density: u32,
    pub midi_export_curve_interpolate: bool,
    pub midi_export_strip_empty_tracks: bool,
    pub midi_export_dedup_overlaps: bool,
    pub use_gpu_synth: bool,
    pub use_gpu_cull: bool,
    pub locale: String,
    pub theme_base: yinhe_theme::base::BaseColors,
    pub theme_preset: String,
    #[serde(default)]
    pub custom_themes: Vec<CustomTheme>,
    #[serde(default)]
    pub favorite_themes: Vec<String>,
    #[serde(skip)]
    pub rename_custom_id: Option<u64>,
    #[serde(skip)]
    pub rename_buffer: String,
    pub ui_scale: f32,
    pub font_scale: f32,
    pub content_opacity: f32,
    /// 完成通知自动收起秒数（None=不自动收起）
    #[serde(default = "default_toast_collapse_secs")]
    pub toast_collapse_secs: Option<u32>,
    /// 可操作通知自动收起秒数（None=不自动收起）
    #[serde(default = "default_toast_action_collapse_secs")]
    pub toast_action_collapse_secs: Option<u32>,
    /// 是否开启通知（关闭后不再弹出任何通知，新的也不再记入历史）
    #[serde(default = "default_toast_enabled")]
    pub toast_enabled: bool,
    pub layout: LayoutSettings,
    pub keybindings: Keybindings,
    pub pinned_file_actions: Vec<bool>,
    pub pinned_edit_actions: Vec<bool>,
    pub pinned_play_pause: bool,
    pub pinned_stop: bool,
    pub pinned_record: bool,
    pub pinned_step_input: bool,
    pub recent_files: Vec<String>,
    #[serde(skip)]
    pub show_settings: bool,
    #[serde(skip)]
    pub settings_tab: usize,
    #[serde(skip)]
    pub settings_search: String,
    #[serde(skip)]
    pub shortcut_recording: bool,
    #[serde(skip)]
    pub available_devices: Vec<String>,
    #[serde(skip)]
    pub available_sample_rates: Vec<u32>,
    #[serde(skip)]
    pub available_midi_inputs: Vec<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device_name: None,
            midi_input_device: None,
            sample_rate: 48000,
            default_sf2_path: String::new(),
            global_sf_config: GlobalSfConfig::builtin_default(),
            xsynth_layers: 4,
            buffer_size: 0,
            min_border_width: 0.0,
            midi_import_encoding: MidiImportEncoding::Utf8,
            midi_export_encoding: MidiImportEncoding::Utf8,
            midi_export_rpn_full: true,
            midi_export_curve_density: 1,
            midi_export_curve_interpolate: false,
            midi_export_strip_empty_tracks: true,
            midi_export_dedup_overlaps: false,
            automation_event_density: 1,
            note_outline: true,
            allow_overlapping_notes: true,
            overlap_blocked_behavior: OverlapBlockedBehavior::default(),
            quick_delete_mode: QuickDeleteMode::default(),
            use_gpu_synth: false,
            use_gpu_cull: false,
            locale: "zh-CN".to_string(),
            theme_base: yinhe_theme::base::BaseColors::DARK,
            theme_preset: "ink-wash".to_string(),
            custom_themes: Vec::new(),
            favorite_themes: Vec::new(),
            rename_custom_id: None,
            rename_buffer: String::new(),
            ui_scale: 1.0,
            font_scale: 1.0,
            content_opacity: 0.7,
            toast_collapse_secs: default_toast_collapse_secs(),
            toast_action_collapse_secs: default_toast_action_collapse_secs(),
            toast_enabled: default_toast_enabled(),
            layout: LayoutSettings::default(),
            keybindings: Keybindings::default(),
            pinned_file_actions: vec![false; 10],
            pinned_edit_actions: vec![false; 12],
            pinned_play_pause: false,
            pinned_stop: false,
            pinned_record: false,
            pinned_step_input: false,
            recent_files: Vec::new(),
            show_settings: false,
            settings_tab: 0,
            settings_search: String::new(),
            shortcut_recording: false,
            available_devices: Vec::new(),
            available_sample_rates: Vec::new(),
            available_midi_inputs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_enabled_defaults_true_for_old_saves() {
        // 旧存档缺字段必须能反序列化，且默认为开启
        let mut v = serde_json::to_value(AudioSettings::default()).expect("serialize default");
        v.as_object_mut()
            .expect("settings serializes to object")
            .remove("toast_enabled");
        let s: AudioSettings = serde_json::from_value(v).expect("old save without flag loads");
        assert!(s.toast_enabled);
    }

    #[test]
    fn toast_enabled_roundtrips_when_off() {
        let mut s = AudioSettings::default();
        s.toast_enabled = false;
        let json = serde_json::to_string(&s).expect("serialize");
        let back: AudioSettings = serde_json::from_str(&json).expect("deserialize");
        assert!(!back.toast_enabled);
    }
}
