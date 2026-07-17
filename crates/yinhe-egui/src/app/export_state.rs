//! Export state subsystem — all audio-export-related fields and methods.

use std::sync::{Arc, Mutex, mpsc};

use yinhe_audio::export::WavBitDepth;

use crate::dialogs::export::{ExportCompleted, ExportProgress};

/// All audio-export-related state, extracted from `App` to reduce the God Object.
pub(crate) struct ExportState {
    /// Receiver for the async export result.
    pub rx: Option<mpsc::Receiver<Result<(String, f64, f64), String>>>,
    /// Shared progress for the export thread to report status.
    pub progress: Arc<Mutex<ExportProgress>>,
    /// Flag to signal the export thread to cancel.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Result of a completed export (shown as a dialog until dismissed).
    pub completed: Option<ExportCompleted>,
    /// Whether the bit-depth dropdown is open.
    pub show_bit_depth: bool,
    /// Selected WAV bit depth for export.
    pub bit_depth: WavBitDepth,
    /// Number of layers to export (0 = all).
    pub layer_count: u32,
    /// Sample rate for export (0 = follow global audio settings).
    pub sample_rate: u32,
}

impl ExportState {
    pub fn new() -> Self {
        Self {
            rx: None,
            progress: ExportProgress::new(),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            completed: None,
            show_bit_depth: false,
            bit_depth: WavBitDepth::Bit24,
            layer_count: 0,
            sample_rate: 0,
        }
    }
}
