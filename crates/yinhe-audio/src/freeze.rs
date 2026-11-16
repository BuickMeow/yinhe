//! LC 冻结离线渲染（FreezeJob）。
//!
//! 冻结的粒度是 Logical Channel：离线跑完整条时间轴，只捕获目标 LC 的
//! **pre-LC 干声**（通道 DSP 之后、LC gain/insert/send 之前）。播放时用这段
//! PCM 覆盖该 LC 的 dense 缓冲，推子/insert/发送仍然实时生效。
//!
//! 与导出一样，冻结复用实时引擎与全部插件实例，在渲染线程内连续离线渲染。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use yinhe_types::MixerChannelId;

use crate::engine::AudioEngine;

/// 冻结任务的共享进度与结果。UI 轮询 `finished` 后取走 `result`。
pub struct FreezeProgress {
    pub visible: bool,
    pub node: Option<MixerChannelId>,
    pub progress: f32,
    pub status: String,
    pub finished: bool,
    pub error: Option<String>,
    /// 完成后的冻结 PCM（已按引擎采样率渲染，未限幅）。
    pub result: Option<FrozenRender>,
}

impl FreezeProgress {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            visible: false,
            node: None,
            progress: 0.0,
            status: String::new(),
            finished: false,
            error: None,
            result: None,
        }))
    }

    fn reset(&mut self, node: MixerChannelId) {
        self.visible = true;
        self.node = Some(node);
        self.progress = 0.0;
        self.status.clear();
        self.finished = false;
        self.error = None;
        self.result = None;
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.finished = true;
        self.visible = false;
        self.error = Some(message.into());
    }
}

/// 冻结渲染产物：整条时间轴的立体声 PCM。
pub struct FrozenRender {
    pub node: MixerChannelId,
    pub sample_rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl FrozenRender {
    pub fn frames(&self) -> u64 {
        self.left.len().min(self.right.len()) as u64
    }
}

/// 冻结失败原因（仅取消；其他错误在任务启动前已挡住）。
pub(crate) enum FreezeError {
    Cancelled,
}

/// 渲染线程的冻结任务。
pub(crate) struct FreezeJob {
    node: MixerChannelId,
    sample_rate: u32,
    main_duration: u64,
    rendered: u64,
    buf: Vec<f32>,
    progress: Arc<Mutex<FreezeProgress>>,
    cancel: Arc<AtomicBool>,
}

impl FreezeJob {
    pub(crate) fn new(
        node: MixerChannelId,
        sample_rate: u32,
        main_duration: u64,
        chunk_frames: usize,
        progress: Arc<Mutex<FreezeProgress>>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        if let Ok(mut p) = progress.lock() {
            p.reset(node);
        }
        Self {
            node,
            sample_rate,
            main_duration,
            rendered: 0,
            buf: vec![0.0; chunk_frames * 2],
            progress,
            cancel,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 渲染下一块；返回 false = 时间轴已全部渲染完。
    pub(crate) fn step(&mut self, engine: &mut AudioEngine) -> Result<bool, FreezeError> {
        if self.cancelled() {
            return Err(FreezeError::Cancelled);
        }
        if self.rendered >= self.main_duration {
            return Ok(false);
        }
        let chunk = self.buf.len() / 2;
        let n = ((self.main_duration - self.rendered) as usize).min(chunk);
        let buf = &mut self.buf[..n * 2];
        buf.fill(0.0);
        engine.render(buf);
        self.rendered += n as u64;

        if let Ok(mut p) = self.progress.lock() {
            p.progress = (self.rendered as f64 / self.main_duration as f64) as f32;
        }
        Ok(true)
    }

    /// 收尾：取回捕获 PCM，写入进度结果。
    pub(crate) fn complete(self, engine: &mut AudioEngine) {
        let result = engine.finish_freeze_capture().map(|capture| FrozenRender {
            node: self.node,
            sample_rate: self.sample_rate,
            left: capture.left,
            right: capture.right,
        });
        if let Ok(mut p) = self.progress.lock() {
            p.progress = 1.0;
            p.finished = true;
            p.visible = false;
            p.result = result;
        }
    }

    /// 取消/失败收尾：丢弃捕获 PCM 并写错误。
    pub(crate) fn abort(self, engine: &mut AudioEngine, message: &str) {
        engine.cancel_freeze_capture();
        if let Ok(mut p) = self.progress.lock() {
            p.fail(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_layout::ChannelLayout;

    fn engine() -> AudioEngine {
        AudioEngine::new(48_000, ChannelLayout::from_mask(vec![true]))
    }

    #[test]
    fn cancelled_job_reports_cancel() {
        let mut engine = engine();
        let cancel = Arc::new(AtomicBool::new(true));
        let mut job = FreezeJob::new(
            MixerChannelId::Midi(0),
            48_000,
            4_800,
            512,
            FreezeProgress::new(),
            cancel,
        );
        assert!(matches!(job.step(&mut engine), Err(FreezeError::Cancelled)));
    }

    #[test]
    fn complete_publishes_captured_pcm() {
        let mut engine = engine();
        let progress = FreezeProgress::new();
        engine.begin_freeze_capture(0);
        {
            let capture = engine.freeze_capture.as_mut().expect("capture");
            capture.left.extend_from_slice(&[0.1, 0.2, 0.3]);
            capture.right.extend_from_slice(&[0.4, 0.5, 0.6]);
        }
        let job = FreezeJob::new(
            MixerChannelId::Midi(0),
            48_000,
            3,
            512,
            Arc::clone(&progress),
            Arc::new(AtomicBool::new(false)),
        );
        job.complete(&mut engine);

        let p = progress.lock().expect("progress");
        assert!(p.finished);
        let result = p.result.as_ref().expect("result");
        assert_eq!(result.frames(), 3);
        assert_eq!(result.left, vec![0.1, 0.2, 0.3]);
        assert_eq!(result.right, vec![0.4, 0.5, 0.6]);
    }
}
