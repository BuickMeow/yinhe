use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::GlobalSfConfig;
use crate::shortcuts::Keybindings;
use yinhe_mid2::MidiImportEncoding;

/// 用户可拖拽调整的布局状态（跨会话持久化）。
/// 默认值与 yinhe-egui 的渲染逻辑一致（clamp 由渲染侧 theme 常量负责）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    /// 右侧栏宽度（px）。
    pub right_panel_width: f32,
    /// AR/PR 竖向分割比例（0~1，AR 占比）。
    pub arr_split: f32,
    /// AR 视图内 transport（轨道）面板宽度（px）。
    pub transport_panel_width: f32,
    /// Arrange 模式下是否同时显示钢琴卷帘。
    pub show_pianoroll_in_arrange: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            right_panel_width: 320.0,
            arr_split: 0.3,
            transport_panel_width: 200.0,
            show_pianoroll_in_arrange: false,
        }
    }
}

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
    /// Linear/Curve 自动化播放时，按多少 tick 间隔生成一个中间 MIDI 事件。
    /// 默认 1（每 tick 一个事件，最平滑）。值越大文件越小但越阶梯化。
    pub automation_event_density: u32,
    /// 音符描边开关（关闭可减少 GPU fill rate，提高性能）
    pub note_outline: bool,
    pub scroll_mode: u32,
    /// 最小边框宽度(像素), 0=不设下限
    pub min_border_width: f32,
    /// MIDI 导入编码
    pub midi_import_encoding: MidiImportEncoding,
    /// 实时播放是否使用 GPU 合成器（yinhe-synth）替代 xsynth。
    /// 默认关闭，仍使用 xsynth。开启后会在加载音色库时初始化 GPU 渲染路径。
    pub use_gpu_synth: bool,
    /// PR 视图音符渲染模式：true=GPU 裁剪（compute shader 做视口裁剪），
    /// false=CPU 构建（CPU 端构建视口内音符实例）。
    /// GPU 裁剪适合音符量极大的场景，但有同屏 800 万音符上限；
    /// CPU 构建无上限，但缩到最小时每帧重建开销大。
    pub use_gpu_cull: bool,
    /// UI 语言代码，如 "zh-CN"、"en-US"、"ja-JP"、"ko-KR"。
    pub locale: String,
    /// 主题标准色（用户可调，全局生效）。
    pub theme_base: yinhe_theme::base::BaseColors,
    /// 主题预设名（"dark"/"light"/"custom"）。
    pub theme_preset: String,
    /// UI 缩放倍率（egui zoom_factor，0.75~2.0，1.0 = 100%）。
    pub ui_scale: f32,
    /// 内容层背景/条纹不透明度（PR/AM 背景、AR 条纹；1.0 = 不透明，0.0 = 全透明）。
    pub content_opacity: f32,
    /// 用户可拖拽调整的布局状态（分割线宽度/比例、PR 显示开关）。
    pub layout: LayoutSettings,
    /// 用户自定义快捷键表（跨会话持久化）。
    pub keybindings: Keybindings,
    /// 文件菜单里被图钉固定的动作（顺序对应 `FileAction::ALL`，9 项）。
    /// 被固定的动作会显示在标题栏上，属于用户自定义工作区，跨会话保存。
    pub pinned_file_actions: [bool; 9],
    /// 编辑菜单里被图钉固定的动作（顺序对应 `EditAction::ALL`，10 项）。
    pub pinned_edit_actions: [bool; 10],
    /// 播放菜单里被图钉固定的"播放/暂停"动作（单个）。
    pub pinned_play_pause: bool,
    /// 播放菜单里被图钉固定的"停止"动作（单个）。
    pub pinned_stop: bool,
    #[serde(skip)]
    pub show_settings: bool,
    /// 设置页当前选中的分类（左侧导航）。
    #[serde(skip)]
    pub settings_tab: usize,
    /// 设置页搜索词（不持久化）。
    #[serde(skip)]
    pub settings_search: String,
    /// 设置页快捷键录制中（键盘事件交给录制器，全局快捷键暂停）。
    #[serde(skip)]
    pub shortcut_recording: bool,
    #[serde(skip)]
    pub available_devices: Vec<String>,
    #[serde(skip)]
    pub available_sample_rates: Vec<u32>,
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
            automation_event_density: 1,
            note_outline: true, // outline on by default (existing behavior)
            use_gpu_synth: false,
            use_gpu_cull: false, // 默认 CPU 构建
            locale: "zh-CN".to_string(),
            theme_base: yinhe_theme::base::BaseColors::DARK,
            theme_preset: "dark".to_string(),
            ui_scale: 1.0, // UI 缩放（egui zoom_factor，1.0 = 100%）
            content_opacity: 0.7,
            layout: LayoutSettings::default(),
            keybindings: Keybindings::default(),
            pinned_file_actions: [false; 9],
            pinned_edit_actions: [false; 10],
            pinned_play_pause: false,
            pinned_stop: false,
            show_settings: false,
            settings_tab: 0,
            settings_search: String::new(),
            shortcut_recording: false,
            available_devices: Vec::new(),
            available_sample_rates: Vec::new(),
        }
    }
}

fn config_path() -> PathBuf {
    crate::paths::app_config_file()
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
                        // 校验 ui_scale 在设置滑块范围（0.75~2.0）内：设置文件可能被
                        // 手改或损坏，异常值会让 egui zoom_factor 放大像素尺寸，
                        // 导致离屏纹理超过 GPU 上限而崩溃。
                        s.ui_scale = s.ui_scale.clamp(0.75, 2.0);
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
    pub fn refresh_devices(&mut self, devices: Vec<String>, rates: Vec<u32>, default_rate: u32) {
        self.available_devices = devices;
        self.available_sample_rates = rates;
        if !self.available_sample_rates.contains(&self.sample_rate) {
            self.sample_rate = default_rate;
        }
    }
}
