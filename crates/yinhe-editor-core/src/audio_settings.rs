use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::GlobalSfConfig;
use crate::shortcuts::Keybindings;
use yinhe_midi::MidiImportEncoding;

/// 快速删除音符的方式（选择/铅笔工具双击或右键删除音符）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum QuickDeleteMode {
    /// 不启用快速删除
    #[default]
    Off,
    /// 双击删除
    DoubleClick,
    /// 右键删除
    RightClick,
    /// 双击与右键均可删除
    Both,
}

impl QuickDeleteMode {
    pub fn allows_double_click(self) -> bool {
        matches!(self, Self::DoubleClick | Self::Both)
    }
    pub fn allows_right_click(self) -> bool {
        matches!(self, Self::RightClick | Self::Both)
    }
}

/// 重叠关闭时，移动等操作发现重叠后的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OverlapBlockedBehavior {
    /// 删除原音符，替换目标音符（默认）：原消失，目标被新音符覆盖
    #[default]
    ReplaceTarget,
    /// 删除原音符，不替换目标：原消失，目标保留，新音符丢弃
    DeleteOriginal,
    /// 退回重叠音符（当前逻辑）：原保留在原位，目标保留
    KeepOriginal,
}

impl OverlapBlockedBehavior {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReplaceTarget => "替换目标",
            Self::DeleteOriginal => "仅删除原",
            Self::KeepOriginal => "退回原位",
        }
    }
}

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

fn deserialize_id<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct IdVisitor;
    impl serde::de::Visitor<'_> for IdVisitor {
        type Value = u64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("u64 or string")
        }
        fn visit_u64<E>(self, v: u64) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            Ok(v)
        }
        fn visit_i64<E>(self, v: i64) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            Ok(v as u64)
        }
        fn visit_str<E>(self, v: &str) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            v.parse().map_err(serde::de::Error::custom)
        }
        fn visit_string<E>(self, v: String) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            v.parse().map_err(serde::de::Error::custom)
        }
    }
    deserializer.deserialize_any(IdVisitor)
}

/// 用户自定义主题（本地持久化，数字 id 后台唯一，不展示）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomTheme {
    /// 唯一数字 id（自增，后台用）
    #[serde(deserialize_with = "deserialize_id")]
    pub id: u64,
    /// 显示名（用于卡片标题，多语言共用）
    pub name: String,
    /// 标准色
    pub base: yinhe_theme::base::BaseColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub output_device_name: Option<String>,
    /// MIDI 输入设备名（None = 未选择，禁用 MIDI 输入）。按名字存：
    /// CoreMIDI 端口 ID 热插拔会变，名字相对稳定。
    pub midi_input_device: Option<String>,
    /// MIDI 直通开关：打开后弹琴即通过合成器发声（不写入工程）。
    pub midi_thru: bool,
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
    /// 「允许新重叠音符」开关（PR 控制栏）。关闭后，新建/移动/复制/粘贴
    /// 产生的新音符若与已有音符重叠（同轨同键、区间相交）则被无视。
    /// 默认开（保持现状行为）。
    pub allow_overlapping_notes: bool,
    /// 重叠关闭时，移动等操作发现重叠后的处理策略。
    /// - `ReplaceTarget`（默认）：删除原音符，替换目标音符（原消失，目标被新音符覆盖）
    /// - `DeleteOriginal`：删除原音符，不替换目标（原消失，目标保留，新音符丢弃）
    /// - `KeepOriginal`：退回重叠音符（原保留在原位，目标保留）
    pub overlap_blocked_behavior: OverlapBlockedBehavior,
    /// 快速删除音符方式（选择/铅笔工具双击或右键删除）。
    pub quick_delete_mode: QuickDeleteMode,
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
    /// 用户自定义主题（持久化到本地），通过顶部取色器/复制自动创建
    #[serde(default)]
    pub custom_themes: Vec<CustomTheme>,
    /// 已收藏的主题标识（预设为 kebab 名，自定义为 id 字符串），收藏的排在网格前面
    #[serde(default)]
    pub favorite_themes: Vec<String>,
    /// 正在重命名的自定义主题 id（不持久化）
    #[serde(skip)]
    pub rename_custom_id: Option<u64>,
    /// 重命名输入缓冲（不持久化）
    #[serde(skip)]
    pub rename_buffer: String,
    /// UI 缩放倍率（egui zoom_factor，0.75~2.0，1.0 = 100%）。
    pub ui_scale: f32,
    /// 内容层背景/条纹不透明度（PR/AM 背景、AR 条纹；1.0 = 不透明，0.0 = 全透明）。
    pub content_opacity: f32,
    /// 用户可拖拽调整的布局状态（分割线宽度/比例、PR 显示开关）。
    pub layout: LayoutSettings,
    /// 用户自定义快捷键表（跨会话持久化）。
    pub keybindings: Keybindings,
    /// 文件菜单里被图钉固定的动作（顺序对应 `FileAction::ALL`，10 项）。
    /// 用 Vec 而非定长数组：旧配置（9 项）升级时可直接解析为新长度，
    /// 访问处用 `get(idx).copied().unwrap_or(false)` 兜底，不越界/pнаpanic。
    pub pinned_file_actions: Vec<bool>,
    /// 编辑菜单里被图钉固定的动作（顺序对应 `EditAction::ALL`，12 项）。
    /// 用 Vec 而非定长数组：旧配置（10 项）升级时可直接解析为新长度，
    /// 访问处用 `get(idx).copied().unwrap_or(false)` 兜底，不越界/panic。
    pub pinned_edit_actions: Vec<bool>,
    /// 播放菜单里被图钉固定的"播放/暂停"动作（单个）。
    pub pinned_play_pause: bool,
    /// 播放菜单里被图钉固定的"停止"动作（单个）。
    pub pinned_stop: bool,
    /// 播放菜单里被图钉固定的"录音"动作（单个）。
    pub pinned_record: bool,
    /// 播放菜单里被图钉固定的"步进输入"动作（单个）。
    pub pinned_step_input: bool,
    /// 最近打开/保存过的文件路径（最新在前，去重，上限 `RECENT_FILES_LIMIT` 条）。
    /// 驱动文件菜单的"最近修改的文件"子菜单（transport bar popup 与 macOS 菜单栏共用）。
    pub recent_files: Vec<String>,
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
    /// 当前系统 MIDI 输入端口列表（运行时枚举，不持久化）。
    #[serde(skip)]
    pub available_midi_inputs: Vec<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device_name: None,
            midi_input_device: None,
            midi_thru: false,
            sample_rate: 48000,
            default_sf2_path: String::new(),
            global_sf_config: GlobalSfConfig::builtin_default(),
            xsynth_layers: 4,
            buffer_size: 0,
            min_border_width: 0.0,
            midi_import_encoding: MidiImportEncoding::Utf8,
            automation_event_density: 1,
            note_outline: true, // outline on by default (existing behavior)
            allow_overlapping_notes: true, // 默认允许重叠（保持现状行为）
            overlap_blocked_behavior: OverlapBlockedBehavior::default(),
            quick_delete_mode: QuickDeleteMode::default(),
            use_gpu_synth: false,
            use_gpu_cull: false, // 默认 CPU 构建
            locale: "zh-CN".to_string(),
            theme_base: yinhe_theme::base::BaseColors::DARK,
            theme_preset: "ink-wash".to_string(),
            custom_themes: Vec::new(),
            favorite_themes: Vec::new(),
            rename_custom_id: None,
            rename_buffer: String::new(),
            ui_scale: 1.0, // UI 缩放（egui zoom_factor，1.0 = 100%）
            content_opacity: 0.7,
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

/// "最近修改的文件"列表上限。
pub const RECENT_FILES_LIMIT: usize = 10;

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
                        // 迁移旧 pinned_edit_actions 长度（10 → 12，新增去重两项）
                        if s.pinned_edit_actions.len() < 12 {
                            s.pinned_edit_actions.resize(12, false);
                        }
                        // 清理历史脏数据：早期版本把压缩包内文件（entry 名如 "track.mid"）
                        // 直接写入 recent_files，其为相对路径且永久不存在。
                        // 过滤掉所有非绝对路径的残留条目（正常 recent 均为绝对路径）。
                        let before_len = s.recent_files.len();
                        s.recent_files
                            .retain(|p| std::path::Path::new(p).is_absolute());
                        if s.recent_files.len() != before_len {
                            // 异步落盘由调用方在下次 save 时完成；此处仅内存清理。
                            tracing::info!(
                                "清理 recent_files 残留相对路径 {} 条",
                                before_len - s.recent_files.len()
                            );
                        }
                        // 危险色/警告色已固定为常量，矫正旧配置中的自定义值
                        s.theme_base.danger = yinhe_theme::base::FIXED_DANGER;
                        s.theme_base.warning = yinhe_theme::base::FIXED_WARNING;
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

    /// 记录一个最近打开/保存的文件：去重置顶，超长截断。
    /// 返回列表是否发生变化（调用方决定是否立即 save）。
    pub fn push_recent_file(&mut self, path: &str) -> bool {
        let before = self.recent_files.clone();
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(RECENT_FILES_LIMIT);
        self.recent_files != before
    }

    /// 从最近文件列表移除（如文件已不存在）。
    pub fn remove_recent_file(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 最近文件列表：去重、置顶、超长截断、已在首位时不上报变化、移除。
    #[test]
    fn recent_files_dedup_cap_and_remove() {
        let mut s = AudioSettings::default();
        for i in 0..12 {
            assert!(s.push_recent_file(&format!("/tmp/{i}.yin")));
        }
        assert_eq!(s.recent_files.len(), RECENT_FILES_LIMIT);
        assert_eq!(s.recent_files[0], "/tmp/11.yin");

        // 重复路径：置顶且去重
        assert!(s.push_recent_file("/tmp/5.yin"));
        assert_eq!(s.recent_files[0], "/tmp/5.yin");
        assert_eq!(
            s.recent_files.iter().filter(|p| *p == "/tmp/5.yin").count(),
            1
        );
        // 已在首位：列表不变，不上报变化（避免多余 save）
        assert!(!s.push_recent_file("/tmp/5.yin"));

        s.remove_recent_file("/tmp/5.yin");
        assert!(!s.recent_files.iter().any(|p| p == "/tmp/5.yin"));
    }
}
