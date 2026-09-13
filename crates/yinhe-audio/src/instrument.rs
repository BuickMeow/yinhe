//! 渲染线程侧的一个乐器插件实例。
//!
//! 生命周期与混音台效果器 insert 一致（见 clap_insert.rs 线程模型）：
//! - UI/管理线程：插件实例 `activate()` 产出 [`InstrumentProcessor`]；
//! - 渲染线程：经 `AudioCommand::SetInstrument` 安装，按乐器通道路由 MIDI 事件，
//!   每块调用 `process` 并把输出写进该乐器 dense 通道的混音台缓冲；
//! - 回收：替换/移除时由渲染线程退回 → UI 线程交还实例 deactivate。
//!
//! 一个乐器通道（`TrackData::instrument_channel`）对应一个实例；多条乐器轨共享
//! 同一乐器通道 = 共享同一实例（音符按各自 MIDI channel 路由进插件，插件自己多音色）。

use yinhe_mixer::{InstrumentProcessor, PluginEvent};

/// 渲染线程侧的乐器实例 + 当前块事件累积器。
pub(crate) struct InstrumentSource {
    /// 乐器通道号（0 起，与 `TrackData::instrument_channel` 对齐）。回收退回 UI 时携带。
    pub channel: u16,
    /// activate 后 move 进来的处理器（渲染线程独占）。
    pub processor: Box<dyn InstrumentProcessor>,
    /// 当前块累积的输入事件（带块内 sample offset）。`process` 前填充、
    /// 处理后清空。后端（CLAP/VST3）会按 `time` 排序，这里无需保证顺序。
    pub events: Vec<PluginEvent>,
}

impl InstrumentSource {
    pub fn new(channel: u16, processor: Box<dyn InstrumentProcessor>) -> Self {
        Self {
            channel,
            processor,
            events: Vec::new(),
        }
    }
}
