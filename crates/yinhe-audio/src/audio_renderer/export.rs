//! 导出模式的渲染线程入口（start/step/finish 三阶段，见 `AudioRenderer::export`）。
//!
//! 导出在渲染线程内连续离线渲染（不推 ring、不发布播放状态），
//! 与实时路径共用同一引擎与插件实例；进度经 `ExportProgress` 通知 UI。

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::export::{ExportError, ExportJob, ExportProgress, WavBitDepth};
use crate::spawn::AudioCommand;

use super::{AudioRenderer, WAKE_SLEEP};

impl AudioRenderer {
    /// 开始导出：复位到干净起点（停止播放 + seek 0），用当前引擎（含全部
    /// insert/乐器插件与 PDC）从头离线渲染。完成状态经 `progress.finished` 通知 UI。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(super) fn start_export(
        &mut self,
        path: std::path::PathBuf,
        bit_depth: WavBitDepth,
        layer_count: Option<usize>,
        restore_layer_count: Option<usize>,
        progress: Arc<Mutex<ExportProgress>>,
        cancel: Arc<AtomicBool>,
        pause: Arc<AtomicBool>,
    ) {
        if self.export.is_some() {
            return;
        }
        cancel.store(false, Ordering::Relaxed);
        pause.store(false, Ordering::Relaxed);
        // 复位到干净起点：停止播放（清 voice/插件状态/PDC 延迟线）→ seek 0。
        self.preview_engine.stop_all();
        self.engine.handle_command(AudioCommand::Stop);
        self.clear_buffered_audio(0);
        self.engine.set_layer_count(layer_count);
        // 导出模式：最大复音数拉满到 GPU 槽位上限——导出离线渲染不受实时预算
        // 约束，voice 保留越完整听感越接近参考（密集段长 release 尾巴不丢）。
        // 实时播放仍由用户设置控制（导出设置不污染用户设置）。
        #[cfg(feature = "gpu")]
        {
            self.export_prev_max_voices = Some(self.engine.max_voices);
            self.engine
                .set_max_voices(Some(yinhe_synth::MAX_VOICE_SLOTS as usize));
        }
        self.request_chase(0);
        // GPU 路径：事件列表按 seek 后位置（0）重建（与 AudioCommand::Stop 同语义）。
        #[cfg(feature = "gpu")]
        self.engine.sync_gpu_backend();
        let main_duration = self.engine.duration_samples();
        if !self.engine.model_loaded() || main_duration == 0 {
            if let Ok(mut p) = progress.lock() {
                p.finished = true;
                p.error = Some("歌曲时长为零，没有可导出的内容".into());
                p.status = "导出失败".into();
            }
            return;
        }
        // 导出块长与实时渲染块一致（GPU 模式大块可显著减少提交/读回次数）。
        #[cfg(feature = "gpu")]
        let export_chunk_frames = if self.gpu_engine() {
            super::GPU_RENDER_CHUNK_FRAMES
        } else {
            crate::engine::ENGINE_BLOCK_FRAMES
        };
        #[cfg(not(feature = "gpu"))]
        let export_chunk_frames = crate::engine::ENGINE_BLOCK_FRAMES;
        match ExportJob::new(
            &path,
            bit_depth,
            self.engine.sample_rate,
            main_duration,
            export_chunk_frames,
            Arc::clone(&progress),
            cancel,
            pause,
        ) {
            Ok(job) => {
                // 从头播放：render 仅在 playing 时产出内容。
                self.engine
                    .handle_command(AudioCommand::Play { from_sample: 0 });
                self.export = Some(job);
                self.export_prev_layer_count = Some(restore_layer_count);
                // 让 UI 立即看到导出开始（停止外观）。
                self.publish_state();
                tracing::info!(
                    "导出开始: {} ({:.1}s, {} Hz)",
                    path.display(),
                    main_duration as f64 / self.engine.sample_rate as f64,
                    self.engine.sample_rate
                );
            }
            Err(e) => {
                if let Ok(mut p) = progress.lock() {
                    p.finished = true;
                    p.error = Some(e.to_string());
                    p.status = format!("导出失败: {e}");
                }
            }
        }
    }

    /// 导出模式每轮：推进一块（或处理取消/暂停/完成）。
    pub(super) fn step_export(&mut self) {
        enum Step {
            Continue,
            Paused,
            Done,
            Failed(ExportError),
        }
        let step = match self.export.as_mut() {
            None => return,
            Some(job) => {
                if job.cancelled() {
                    Step::Failed(ExportError::Cancelled)
                } else if job.paused() {
                    Step::Paused
                } else {
                    match job.step(&mut self.engine) {
                        Ok(true) => Step::Continue,
                        Ok(false) => Step::Done,
                        Err(e) => Step::Failed(e),
                    }
                }
            }
        };
        match step {
            Step::Continue => {}
            Step::Paused => thread::sleep(WAKE_SLEEP),
            Step::Done => self.finish_export(Ok(())),
            Step::Failed(e) => self.finish_export(Err(e)),
        }
    }

    /// 导出收尾：成功时 finalize 文件；无论成败都回到干净停止态并发布状态。
    pub(super) fn finish_export(&mut self, result: Result<(), ExportError>) {
        let Some(job) = self.export.take() else {
            return;
        };
        let progress = job.progress_handle();
        match result {
            Ok(()) => {
                let err = job.finalize().err().map(|e| e.to_string());
                if let Ok(mut p) = progress.lock() {
                    p.progress = 1.0;
                    p.error = err.clone();
                    p.finished = true;
                    p.status = if err.is_some() {
                        "写入文件失败".into()
                    } else {
                        "导出完成".into()
                    };
                }
                tracing::info!("导出完成");
            }
            Err(e) => {
                // 不 finalize：保留不完整文件（与旧导出的中止行为一致）。
                drop(job);
                if let Ok(mut p) = progress.lock() {
                    p.finished = true;
                    p.status = match &e {
                        ExportError::Cancelled => "已中止".into(),
                        _ => format!("导出失败: {e}"),
                    };
                    p.error = Some(e.to_string());
                }
                tracing::info!("导出结束（未完成）: {e}");
            }
        }
        // 回到干净停止态（导出期间未推 ring，这里确保停止后无残留），
        // 并恢复导出前的 xsynth 层数（导出设置不污染用户设置）。
        self.engine.handle_command(AudioCommand::Stop);
        if let Some(prev) = self.export_prev_layer_count.take() {
            self.engine.set_layer_count(prev);
        } else {
            // 无记录：恢复为 None（清空导出设定的层数限制）。
            self.engine.set_layer_count(None);
        }
        #[cfg(feature = "gpu")]
        if let Some(prev) = self.export_prev_max_voices.take() {
            // 恢复为导出前的等效值（0 = 自动 → None）。
            self.engine
                .set_max_voices(if prev == 0 { None } else { Some(prev) });
        }
        self.clear_buffered_audio(0);
        self.publish_state();
    }
}
