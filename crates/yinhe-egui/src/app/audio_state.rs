//! Audio subsystem state — fields related to the audio engine and playback.

/// All audio-engine-related state, extracted from `App` to reduce the God Object.
pub(crate) struct AudioState {
    /// The active audio backend handle (None if not initialized yet).
    pub handle: Option<yinhe_audio::CpalAudioHandle>,
    /// Which document index the audio engine is currently bound to.
    /// Used to detect document switches that require an audio rebuild.
    pub active_doc: Option<usize>,
    /// Last known sample position from the audio engine, and the instant we read it.
    /// Used to interpolate cursor position between callback updates.
    pub playback_anchor: Option<(u64, std::time::Instant)>,
    /// Set when Play/Resume is sent but the audio thread hasn't acknowledged yet.
    /// Ensures request_repaint() keeps firing until is_playing() returns true.
    pub pending_playback: bool,
    /// 设备切换对话框是否需要显示。
    ///
    /// cpal `stream_error` 一旦置位就不可恢复。检测到时把这个 flag 置 true，
    /// 弹出"音频设备切换"对话框。用户选了新设备且 spawn 成功后置回 false；
    /// spawn 失败则保持 true 并把错误信息塞进 `device_switch_error`。
    /// 这样对话框的可见性和 stream_error 解耦 —— 即使新设备也 spawn 失败，
    /// 对话框仍能保持打开让用户重选，而不是因为 handle=None 就消失。
    pub device_switch_pending: bool,
    /// 上一次设备切换 spawn 失败的错误信息（仅当 `device_switch_pending` 为 true 时有意义）。
    pub device_switch_error: Option<String>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            handle: None,
            active_doc: None,
            playback_anchor: None,
            pending_playback: false,
            device_switch_pending: false,
            device_switch_error: None,
        }
    }
}
