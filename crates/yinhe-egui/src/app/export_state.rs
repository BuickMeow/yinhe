//! Export state subsystem — all audio-export-related fields and methods.

use std::sync::{Arc, Mutex};

use yinhe_audio::export::WavBitDepth;

use crate::dialogs::export::ExportProgress;
use crate::widgets::toast::model::ProgressSource;

/// All audio-export-related state, extracted from `App` to reduce the God Object.
pub(crate) struct ExportState {
    /// 是否有导出在进行（轮询 progress.finished 收尾）。
    pub running: bool,
    /// Shared progress for the export thread to report status.
    pub progress: Arc<Mutex<ExportProgress>>,
    /// Flag to signal the export thread to cancel.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Flag to signal the export thread to pause (true=paused, spin-wait in render loop).
    pub pause: Arc<std::sync::atomic::AtomicBool>,
    /// Whether the bit-depth dropdown is open.
    pub show_bit_depth: bool,
    /// Selected WAV bit depth for export.
    pub bit_depth: WavBitDepth,
    /// Number of layers to export (0 = all).
    pub layer_count: u32,
    /// Sample rate for export (0 = follow global audio settings).
    pub sample_rate: u32,
    /// 导出成功后是否写入 WAV 歌曲信息（隐私开关）。
    pub write_metadata: bool,
    /// 本次导出用户选择的输出路径（中止卡“打开文件夹”用；None 则不设按钮）。
    pub last_output_path: Option<String>,
    /// 导出成功后要写入 WAV 的歌曲信息（路径 + 元数据快照）。
    pub tags: Option<(String, yinhe_core::SongInfo)>,
}

impl ExportState {
    pub fn new() -> Self {
        Self {
            running: false,
            progress: ExportProgress::new(),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pause: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            show_bit_depth: false,
            bit_depth: WavBitDepth::Bit24,
            layer_count: 0,
            sample_rate: 0,
            write_metadata: true,
            last_output_path: None,
            tags: None,
        }
    }
}

/// 导出进度数据源：toast 渲染时 pull，不再每帧拷贝文案。
pub(crate) struct ExportToastSource {
    pub progress: Arc<Mutex<ExportProgress>>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    pub pause: Arc<std::sync::atomic::AtomicBool>,
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
        rust_i18n::t!("toast.exporting").to_string()
    }
    fn fraction(&self) -> f32 {
        self.snapshot().0
    }
    fn cancel(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        Some(self.cancel.clone())
    }
    fn pause(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        Some(self.pause.clone())
    }
}

// ── MIDI 导出（后台线程 + 进度 toast）──

/// 后台 MIDI 导出任务状态。MIDI 导出不碰音频引擎，可以独立于播放运行。
pub(crate) struct MidiExportState {
    pub running: bool,
    /// 导出线程写入、toast 每帧 pull 的实时进度。
    pub live: Arc<Mutex<MidiExportLive>>,
    /// 完成信号：`Ok(())` / 错误文本。
    pub rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// 输出路径（完成 toast 的"打开文件夹"用）。
    pub path: String,
}

pub(crate) struct MidiExportLive {
    pub progress: Option<yinhe_midi::MidiExportProgress>,
}

impl MidiExportState {
    pub fn new() -> Self {
        Self {
            running: false,
            live: Arc::new(Mutex::new(MidiExportLive { progress: None })),
            rx: None,
            path: String::new(),
        }
    }
}

/// MIDI 导出进度数据源。
pub(crate) struct MidiExportToastSource {
    pub live: Arc<Mutex<MidiExportLive>>,
}

impl MidiExportToastSource {
    fn snapshot(&self) -> Option<yinhe_midi::MidiExportProgress> {
        self.live.lock().ok().and_then(|l| l.progress)
    }
}

impl ProgressSource for MidiExportToastSource {
    fn title(&self) -> String {
        rust_i18n::t!("toast.exporting_midi").to_string()
    }

    fn fraction(&self) -> f32 {
        self.snapshot().map(|p| p.fraction()).unwrap_or(0.0)
    }

    fn cancel(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        None
    }
}

#[cfg(test)]
mod midi_export_tests {
    use super::*;

    #[test]
    fn midi_toast_pulls_live_progress() {
        let live = Arc::new(Mutex::new(MidiExportLive { progress: None }));
        let src = MidiExportToastSource {
            live: Arc::clone(&live),
        };
        assert_eq!(src.fraction(), 0.0);

        live.lock().expect("lock").progress = Some(yinhe_midi::MidiExportProgress {
            stage: yinhe_midi::MidiExportStage::Indexing,
            notes_done: 50,
            notes_total: 100,
            track_index: 0,
            total_tracks: 4,
        });
        assert!((src.fraction() - 0.2).abs() < 1e-6);

        live.lock().expect("lock").progress = Some(yinhe_midi::MidiExportProgress {
            stage: yinhe_midi::MidiExportStage::Writing,
            notes_done: 50,
            notes_total: 100,
            track_index: 1,
            total_tracks: 4,
        });
        assert!((src.fraction() - 0.7).abs() < 1e-6);
    }
}
