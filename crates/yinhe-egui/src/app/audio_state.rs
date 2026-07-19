//! Audio subsystem state — fields related to the audio engine and playback.

use yinhe_audio::channel_layout::ChannelLayout;

/// All audio-engine-related state, extracted from `App` to reduce the God Object.
pub(crate) struct AudioState {
    /// The active audio backend handle (None if not initialized yet).
    pub handle: Option<yinhe_audio::CpalAudioHandle>,
    /// Which document index the audio engine is currently bound to.
    /// Used to detect document switches that require an audio rebuild.
    pub active_doc: Option<usize>,
    /// 当前引擎创建时使用的 `ChannelLayout` 快照。
    ///
    /// 用于在 `notify_notes_changed` / `notify_audio_model_changed` 里检测
    /// channel 激活状态是否翻转：若 `layout.differs_from_counts(model)` 为 true，
    /// 说明有 channel 的激活状态变了（0→1 或 1→0），必须 teardown + 重建引擎。
    /// 否则可走便宜的 `UpdateNotes` / `ReloadNotes` 路径。
    ///
    /// `None` 表示引擎尚未 spawn，下一帧 `rebuild_audio_if_needed` 会用新 model
    /// 重新算 layout 并填入此字段。
    pub last_channel_layout: Option<ChannelLayout>,
    /// Last known sample position from the audio engine, and the instant we read it.
    /// Used to interpolate cursor position between callback updates.
    pub playback_anchor: Option<(u64, std::time::Instant)>,
    /// Set when Play/Resume is sent but the audio thread hasn't acknowledged yet.
    /// Ensures request_repaint() keeps firing until is_playing() returns true.
    pub pending_playback: bool,
    /// 设备切换对话框是否需要显示。
    ///
    /// 两种触发场景：
    /// - cpal `stream_error` 置位（设备热拔/驱动崩溃，流已死，必须切换）
    /// - 设备列表变更（插拔耳机，流还活着，可选切换）
    ///
    /// 用户选了新设备且 spawn 成功后置回 false；
    /// spawn 失败则保持 true 并把错误信息塞进 `device_switch_error`。
    pub device_switch_pending: bool,
    /// true = 流已死（stream_error），必须切换或退出，对话框不显示"保持当前设备"按钮。
    /// false = 设备列表变更（插拔耳机），流还活着，对话框显示"保持当前设备"按钮。
    pub device_switch_required: bool,
    /// 上一次设备切换 spawn 失败的错误信息（仅当 `device_switch_pending` 为 true 时有意义）。
    pub device_switch_error: Option<String>,
    /// 上一次轮询到的系统输出设备列表，用于检测设备插拔。
    /// 空 Vec 表示还没初始化过（首次轮询只记录、不触发对话框）。
    pub last_known_devices: Vec<String>,
    /// 上一次轮询设备列表的时间。每秒轮询一次，避免每帧调用 cpal 枚举。
    pub last_device_poll: Option<std::time::Instant>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            handle: None,
            active_doc: None,
            last_channel_layout: None,
            playback_anchor: None,
            pending_playback: false,
            device_switch_pending: false,
            device_switch_required: false,
            device_switch_error: None,
            last_known_devices: Vec::new(),
            last_device_poll: None,
        }
    }
}
