//! 渲染线程的 LC 冻结入口（start / step / finish）。

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use yinhe_types::MixerChannelId;

use crate::freeze::{FreezeError, FreezeJob, FreezeProgress};
use crate::spawn::AudioCommand;

use super::AudioRenderer;

impl AudioRenderer {
    /// 开始冻结一条 LC：停止当前播放，从 0 离线渲染整条时间轴，
    /// 只捕获目标 dense 通道的 pre-LC 干声。
    pub(super) fn start_freeze(
        &mut self,
        node: MixerChannelId,
        progress: Arc<Mutex<FreezeProgress>>,
        cancel: Arc<AtomicBool>,
    ) {
        if self.freeze.is_some() || self.export.is_some() {
            if let Ok(mut p) = progress.lock() {
                p.failed_before_start("已有离线任务在运行");
            }
            return;
        }
        let dense = match node {
            MixerChannelId::Midi(ch) => self.engine.channel_layout.dense_for(ch as usize),
            MixerChannelId::Audio(ch) => self.engine.channel_layout.audio_dense_for(ch),
            MixerChannelId::TrackLc(id) => self
                .engine
                .channel_layout
                .track_lc_dense_for(id)
                .map(|(_, dense)| dense)
                .unwrap_or(u32::MAX),
            MixerChannelId::Bus(_) | MixerChannelId::Master => u32::MAX,
        };
        if dense == u32::MAX {
            if let Ok(mut p) = progress.lock() {
                p.failed_before_start("该通道没有可冻结的音频流");
            }
            return;
        }
        let main_duration = self.engine.duration_samples();
        if !self.engine.model_loaded() || main_duration == 0 {
            if let Ok(mut p) = progress.lock() {
                p.failed_before_start("歌曲时长为零，没有可冻结的内容");
            }
            return;
        }

        cancel.store(false, Ordering::Relaxed);
        self.engine.stop_preview_all();
        self.engine.handle_command(AudioCommand::Stop);
        self.clear_buffered_audio(0);
        self.request_chase(0);
        self.engine.begin_freeze_capture(dense as usize);
        self.engine
            .handle_command(AudioCommand::Play { from_sample: 0 });
        self.engine.export_mode = true;
        self.freeze = Some(FreezeJob::new(
            node,
            self.engine.sample_rate,
            main_duration,
            crate::engine::ENGINE_BLOCK_FRAMES,
            progress,
            cancel,
        ));
        self.publish_state();
    }

    /// 冻结模式每轮：推进一块（或处理取消/完成）。
    pub(super) fn step_freeze(&mut self) {
        enum Step {
            Continue,
            Done,
            Cancelled,
        }
        let step = match self.freeze.as_mut() {
            None => return,
            Some(job) => match job.step(&mut self.engine) {
                Ok(true) => Step::Continue,
                Ok(false) => Step::Done,
                Err(FreezeError::Cancelled) => Step::Cancelled,
            },
        };
        match step {
            Step::Continue => {}
            Step::Done => self.finish_freeze(),
            Step::Cancelled => {
                let Some(job) = self.freeze.take() else {
                    return;
                };
                job.abort(&mut self.engine, "已中止");
                self.restore_after_freeze();
            }
        }
    }

    fn finish_freeze(&mut self) {
        let Some(job) = self.freeze.take() else {
            return;
        };
        job.complete(&mut self.engine);
        self.restore_after_freeze();
    }

    /// 冻结收尾：恢复导出模式、停止引擎并回到干净停止态。
    fn restore_after_freeze(&mut self) {
        self.engine.export_mode = false;
        self.engine.handle_command(AudioCommand::Stop);
        self.clear_buffered_audio(0);
        self.publish_state();
    }
}

impl FreezeProgress {
    /// 启动前失败：任务没有进入渲染线程。
    pub(crate) fn failed_before_start(&mut self, message: &str) {
        self.visible = false;
        self.finished = true;
        self.error = Some(message.into());
    }
}
