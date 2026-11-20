//! 预览合成器后端（方案 C：XSynth 已移除，只用 yinhe CPU 预览）。
//!
//! `chase_channel_states` 计算目标位置的自动化状态，供 PR/MIDI 预览组装音符。
//! `PreviewEngine` 包裹 `YinhePreview`，与主引擎共享 key map 音色。

use std::borrow::BorrowMut;

use yinhe_mixer::ChannelBuffers;

use crate::audio_model::SortedCC;
use crate::channel::ChannelState;
use crate::channel_layout::ChannelLayout;

mod yinhe;

/// 为同一 channel 的一组目标位置增量 chase：`targets` 必须升序，只扫一遍 cc_events。
/// 返回与 `targets` 一一对应的状态快照（用于预览整组音符的目标位置自动化）。
/// 事件比较在 tick 域（cc_events 已按 tick 排序）。
///
/// 同 tick 同通道同目标取 max（与播放 dispatch 的聚合一致）；跨 tick 的同目标
/// 事件仍按事件流顺序后写覆盖。
///
/// 边界说明（Bug 8）：预览路径没有播放那样的 dispatch 兜底（seek 后 target 处事件
/// 由 dispatch 补发），因此 `tick == target` 的自动化跳变事件必须包含进来（含等号），
/// 否则恰好在音符起点的跳变永远到不了预览通道，预听听到的会是跳变前的旧值。
pub(crate) fn chase_channel_states(
    cc_events: &[SortedCC],
    channel: u32,
    targets: &[u32],
) -> Vec<ChannelState> {
    let mut states = Vec::with_capacity(targets.len());
    let mut state = ChannelState::default();
    let mut cursor = 0usize;
    for &target in targets {
        let start = cursor;
        while cursor < cc_events.len() && cc_events[cursor].tick <= target {
            cursor += 1;
        }
        // 同 tick 同通道同目标取 max（与播放 dispatch 的聚合一致）；
        // 插件参数事件不是通道事件（占位），跳过避免污染预览 chase。
        let seg = &cc_events[start..cursor];
        for &i in &crate::audio_model::aggregate_max_reps(seg, |cc| {
            cc.plugin_param.is_none() && cc.channel == channel
        }) {
            state.apply(&seg[i].event);
        }
        states.push(state);
    }
    states
}

/// 预览组中的一个音符（渲染器组装好后提交）。
pub(crate) struct PreviewNoteIn {
    pub(crate) channel: u8,
    pub(crate) key: u8,
    pub(crate) velocity: u8,
    pub(crate) duration: Option<u64>,
    /// 目标位置自动化状态。
    pub(crate) state: ChannelState,
    /// 工程级内置音源参数（预览也跟随旋钮）。
    pub(crate) synth_params: yinhe_types::SynthParams,
    /// 目标位置 sample（组内相对时值依据）。
    pub(crate) target_sample: u64,
}

/// 预览合成器后端：只用 yinhe CPU（与主引擎同族，共享 key map 音色）。
pub(crate) enum PreviewEngine {
    Yinhe(Box<yinhe::YinhePreview>),
}

impl PreviewEngine {
    pub(crate) fn new(layout: &ChannelLayout, sample_rate: u32, interpolation: u32) -> Self {
        PreviewEngine::Yinhe(Box::new(yinhe::YinhePreview::new(
            layout,
            sample_rate,
            interpolation,
        )))
    }

    /// yinhe 后端：按 dense 槽位加载 key map（进程级缓存，与主引擎共享）。
    pub(crate) fn load_yinhe_soundfonts(
        &mut self,
        denses: &[u32],
        paths: &[std::path::PathBuf],
    ) -> Result<(), String> {
        match self {
            PreviewEngine::Yinhe(p) => p.load_soundfonts(denses, paths),
        }
    }

    pub(crate) fn preview_notes(&mut self, notes: Vec<PreviewNoteIn>, exclusive: bool) {
        match self {
            PreviewEngine::Yinhe(p) => p.preview_notes(notes, exclusive),
        }
    }

    pub(crate) fn render_add<B: BorrowMut<ChannelBuffers>>(
        &mut self,
        targets: &mut [B],
        frames: usize,
    ) {
        match self {
            PreviewEngine::Yinhe(p) => p.render_add(targets, frames),
        }
    }

    pub(crate) fn previewing(&self) -> bool {
        match self {
            PreviewEngine::Yinhe(p) => p.previewing(),
        }
    }

    pub(crate) fn stop_all(&mut self) {
        match self {
            PreviewEngine::Yinhe(p) => p.stop_all(),
        }
    }

    pub(crate) fn stop_key(&mut self, key: u8) {
        match self {
            PreviewEngine::Yinhe(p) => p.stop_key(key),
        }
    }
}
