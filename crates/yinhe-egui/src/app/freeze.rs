//! LC 冻结：发起离线渲染、轮询结果并写回工程。
//!
//! 冻结产物是工程内嵌 `AudioSource`（WAV 字节） + `MixerNode.freeze` 引用；
//! 播放时引擎用冻结 PCM 覆盖该 LC 的 dense 缓冲，LC 推子/insert/发送仍实时。

use std::sync::Arc;

use rust_i18n::t;
use yinhe_audio::{DecodedAudio, WavePeaks};
use yinhe_mixer::FrozenAudio;
use yinhe_types::MixerChannelId;

use crate::app::App;

impl App {
    /// 发起一条 LC 的冻结。只有 MIDI / 音频 / 每轨 LC 可以冻结。
    pub(crate) fn start_lc_freeze(&mut self, idx: usize, channel: MixerChannelId) {
        if self.jobs.is_exporting() || self.jobs.is_freezing() {
            self.notifications
                .error(t!("toast.freeze_failed"), t!("toast.freeze_busy"));
            return;
        }
        if matches!(channel, MixerChannelId::Bus(_) | MixerChannelId::Master) {
            self.notifications
                .error(t!("toast.freeze_failed"), t!("toast.freeze_not_lc"));
            return;
        }
        let Some(audio) = self.audio_state.handle.as_ref() else {
            self.notifications
                .error(t!("toast.freeze_failed"), t!("toast.audio_unavailable"));
            return;
        };
        let revision = self.workspace.documents[idx].data.revision;
        let node = channel.node_id();
        {
            self.jobs.freeze.node = Some(node);
            self.jobs.freeze.revision = revision;
            self.jobs
                .freeze
                .cancel
                .store(false, std::sync::atomic::Ordering::Relaxed);
            if let Ok(mut p) = self.jobs.freeze.progress.lock() {
                p.visible = true;
                p.node = Some(channel);
                p.progress = 0.0;
                p.status.clear();
                p.finished = false;
                p.error = None;
                p.result = None;
            }
        }
        self.jobs.freeze.running = true;
        audio.handle.send(yinhe_audio::AudioCommand::FreezeStart {
            node: channel,
            progress: Arc::clone(&self.jobs.freeze.progress),
            cancel: Arc::clone(&self.jobs.freeze.cancel),
        });
    }

    /// 轮询冻结结果；完成时把 PCM 写成工程素材并设置节点 freeze。
    pub(crate) fn poll_lc_freeze(&mut self) {
        if !self.jobs.is_freezing() {
            return;
        }
        let outcome = self
            .jobs
            .freeze
            .progress
            .lock()
            .ok()
            .and_then(|mut p| p.finished.then(|| (p.error.clone(), p.result.take())));
        let Some((error, result)) = outcome else {
            return;
        };
        self.jobs.freeze.running = false;
        let node = self.jobs.freeze.node.take();
        let start_revision = self.jobs.freeze.revision;
        let cancelled = self
            .jobs
            .freeze
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed);
        if let Some(error) = error {
            let (outcome, title) = if cancelled {
                (
                    crate::widgets::toast::ProgressOutcome::Aborted,
                    t!("toast.freeze_aborted").to_string(),
                )
            } else {
                (
                    crate::widgets::toast::ProgressOutcome::Failed,
                    t!("toast.freeze_failed").to_string(),
                )
            };
            self.notifications.finish_progress(
                crate::widgets::toast::FREEZE_PROGRESS_ID,
                outcome,
                title,
                error,
                None,
            );
            return;
        }
        let Some(result) = result else {
            self.notifications.finish_progress(
                crate::widgets::toast::FREEZE_PROGRESS_ID,
                crate::widgets::toast::ProgressOutcome::Failed,
                t!("toast.freeze_failed").to_string(),
                t!("toast.freeze_empty").to_string(),
                None,
            );
            return;
        };
        let Some(node) = node else {
            return;
        };
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        {
            let doc = &self.workspace.documents[idx];
            if doc.data.revision != start_revision {
                self.notifications.finish_progress(
                    crate::widgets::toast::FREEZE_PROGRESS_ID,
                    crate::widgets::toast::ProgressOutcome::Aborted,
                    t!("toast.freeze_aborted").to_string(),
                    t!("toast.freeze_edited").to_string(),
                    None,
                );
                return;
            }
        }
        let frames = result.frames();
        let bytes =
            match yinhe_audio::encode_wav_bytes(&result.left, &result.right, result.sample_rate) {
                Ok(bytes) => bytes,
                Err(e) => {
                    self.notifications.finish_progress(
                        crate::widgets::toast::FREEZE_PROGRESS_ID,
                        crate::widgets::toast::ProgressOutcome::Failed,
                        t!("toast.freeze_failed").to_string(),
                        e,
                        None,
                    );
                    return;
                }
            };
        let uuid = uuid::Uuid::new_v4().to_string();
        let peaks = WavePeaks::build(&result.left, &result.right);
        let decoded = Arc::new(DecodedAudio {
            sample_rate: result.sample_rate,
            frames: frames as usize,
            left: Arc::from(result.left),
            right: Arc::from(result.right),
            peaks,
        });
        let duration_seconds = frames as f64 / result.sample_rate.max(1) as f64;
        {
            let doc = &mut self.workspace.documents[idx];
            doc.add_audio_source(yinhe_core::AudioSource {
                uuid: uuid.clone(),
                name: format!("freeze-{}.wav", uuid),
                data: Arc::new(bytes),
                duration_seconds,
            });
            let before = doc.capture_snapshot();
            let action = doc.set_node_freeze(
                node,
                Some(FrozenAudio {
                    source: uuid.clone(),
                    source_revision: 0,
                    sample_rate: result.sample_rate,
                    frames,
                }),
            );
            if let Some(action) = action {
                doc.push_undo(action, t!("undo.freeze_lc").as_ref(), before);
            }
        }
        if let Some(audio) = self.audio_state.handle.as_ref() {
            self.audio_library
                .push_to_engine(&audio.handle, uuid.clone(), Arc::clone(&decoded));
        }
        self.audio_library.insert_decoded(uuid.clone(), decoded);
        self.push_mixer_state_to_engine(idx);
        self.notifications.finish_progress(
            crate::widgets::toast::FREEZE_PROGRESS_ID,
            crate::widgets::toast::ProgressOutcome::Completed,
            t!("toast.freeze_done").to_string(),
            format!("{:?}", node),
            None,
        );
    }
}
