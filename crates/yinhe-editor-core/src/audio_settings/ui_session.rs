//! 运行时 UI 会话态：设置窗口/快捷键录制/设备枚举等交互过程状态。
//!
//! 这些字段**不落盘**（挂在 [`super::AudioSettings::ui_session`] 上并整体
//! `#[serde(skip)]`），与配置本身的职责分离：配置只保留用户可持久化的选择。

/// 设置窗口与设备枚举的临时状态，随进程生命周期存活。
#[derive(Debug, Clone, Default)]
pub struct SettingsUiSession {
    /// 正在重命名的自定义主题 id（None = 未重命名）。
    pub rename_custom_id: Option<u64>,
    /// 主题重命名输入缓冲。
    pub rename_buffer: String,
    /// 设置窗口是否打开。
    pub show_settings: bool,
    /// 当前设置分类索引。
    pub settings_tab: usize,
    /// 设置搜索词。
    pub settings_search: String,
    /// 快捷键录制中（录制期间全局快捷键让位）。
    pub shortcut_recording: bool,
    /// 可用输出设备（运行时刷新）。
    pub available_devices: Vec<String>,
    /// 可用录音输入设备（运行时刷新）。
    pub available_input_devices: Vec<String>,
    /// 可用采样率（运行时刷新）。
    pub available_sample_rates: Vec<u32>,
    /// 可用 MIDI 输入端口（运行时刷新）。
    pub available_midi_inputs: Vec<String>,
}
