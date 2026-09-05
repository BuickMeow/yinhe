//! Export state subsystem — all audio-export-related fields and methods.

use std::sync::{Arc, Mutex, mpsc};

use yinhe_audio::export::WavBitDepth;

use crate::dialogs::export::{ExportCompleted, ExportProgress};
use crate::widgets::toast::model::ProgressSource;

/// 导出线程完成消息：`Ok((输出路径, 耗时秒, 倍速))` 或 `Err(错误信息)`。
pub(crate) type ExportResultMsg = Result<(String, f64, f64), String>;

/// All audio-export-related state, extracted from `App` to reduce the God Object.
pub(crate) struct ExportState {
    /// Receiver for the async export result.
    pub rx: Option<mpsc::Receiver<ExportResultMsg>>,
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
    /// 本次导出用户选择的输出路径（中止卡“打开文件夹”用；None 则不设按钮）。
    pub last_output_path: Option<String>,
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
            last_output_path: None,
        }
    }
}

/// 导出进度数据源：toast 渲染时 pull，不再每帧拷贝文案。
pub(crate) struct ExportToastSource {
    pub progress: Arc<Mutex<ExportProgress>>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl ExportToastSource {
    fn snapshot(&self) -> (f32, String) {
        self.progress
            .lock()
            .ok()
            .map(|st| {
                let frac = st.progress.clamp(0.0, 1.0);
                let label = if st.status.is_empty() {
                    format!("{:.0}%", frac * 100.0)
                } else {
                    st.status.clone()
                };
                (frac, label)
            })
            .unwrap_or((0.0, String::new()))
    }
}

impl ProgressSource for ExportToastSource {
    fn title(&self) -> String {
        "正在导出".to_string()
    }
    fn message(&self) -> String {
        self.snapshot().1
    }
    fn fraction(&self) -> f32 {
        self.snapshot().0
    }
    fn detail(&self) -> String {
        self.snapshot().1
    }
    fn cancel(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        Some(self.cancel.clone())
    }
}
