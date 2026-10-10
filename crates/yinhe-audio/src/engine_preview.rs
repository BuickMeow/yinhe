//! 引擎的预览接线：持有预览合成器、转发预览命令、把预览音频注入混音链路。
//!
//! 预览音频不再单独叠加到主输出，而是累加进 mixer 的 per-channel 缓冲，
//! 与播放共用同一条链路（合成器 → 通道 DSP(CC7/10/11/71/74) → insert → 推子 →
//! master），因此试听反映**当前**的控制状态与混音台设置。

use std::path::PathBuf;
use std::sync::Arc;

use xsynth_core::soundfont::SoundfontBase;

use crate::engine::AudioEngine;
use crate::preview_engine::{PreviewEngine, PreviewNoteIn};

impl AudioEngine {
    /// 注入预览合成器（`init_engine` 创建后调用；测试可不设置）。
    pub(crate) fn set_preview_engine(&mut self, preview: PreviewEngine) {
        self.preview = Some(preview);
    }

    /// 是否有预览音在响（含待触发与余音）。
    pub(crate) fn previewing(&self) -> bool {
        self.preview.as_ref().is_some_and(|p| p.previewing())
    }

    pub(crate) fn preview_notes(&mut self, notes: Vec<PreviewNoteIn>, exclusive: bool) {
        if let Some(p) = self.preview.as_mut() {
            p.preview_notes(notes, exclusive);
        }
    }

    pub(crate) fn stop_preview_all(&mut self) {
        if let Some(p) = self.preview.as_mut() {
            p.stop_all();
        }
    }

    pub(crate) fn stop_preview_key(&mut self, key: u8) {
        if let Some(p) = self.preview.as_mut() {
            p.stop_key(key);
        }
    }

    /// 预览音色加载（xsynth 后端，按源通道；与主引擎共享 Arc）。
    pub(crate) fn set_preview_xsynth_soundfonts(
        &mut self,
        channel: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
    ) {
        if let Some(p) = self.preview.as_mut() {
            p.set_xsynth_soundfonts(channel, soundfonts);
        }
    }

    /// 预览音色加载（yinhe 后端，按 dense 槽位；与主引擎共享 key map）。
    pub(crate) fn load_preview_yinhe_soundfonts(
        &mut self,
        denses: &[u32],
        paths: &[PathBuf],
    ) -> Result<(), String> {
        match self.preview.as_mut() {
            Some(p) => p.load_yinhe_soundfonts(denses, paths),
            None => Ok(()),
        }
    }

    /// 把预览音频累加进 mixer 通道缓冲（在合成器写入之后、通道 DSP 之前调用）。
    pub(crate) fn inject_preview(&mut self, frames: usize) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        if preview.previewing() {
            preview.render_add(self.mixer.buffers_mut(), frames);
        }
    }
}
