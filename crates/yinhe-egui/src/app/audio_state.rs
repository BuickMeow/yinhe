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
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            handle: None,
            active_doc: None,
            playback_anchor: None,
            pending_playback: false,
        }
    }
}
