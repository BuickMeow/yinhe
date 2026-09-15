//! 渲染线程侧的一个乐器插件实例。
//!
//! 生命周期与混音台效果器 insert 一致（见 clap_insert.rs 线程模型）：
//! - UI/管理线程：插件实例 `activate()` 产出 [`InstrumentProcessor`]；
//! - 渲染线程：经 `AudioCommand::SetInstrument` 安装，按 MIDI 通道路由事件，
//!   每块调用 `process` 并把输出写进该 MIDI 通道的混音台缓冲；
//! - 回收：替换/移除时由渲染线程退回 → UI 线程交还实例 deactivate。
//!
//! 一个 MIDI 通道（`TrackData::global_channel()`）挂载一个乐器实例；未挂插件时
//! 该通道使用内置 XSynth。同通道的多条轨共享同一实例（音符按各自 MIDI channel
//! 路由进插件，插件自己多音色）。

use yinhe_mixer::{InstrumentProcessor, PluginEvent};

/// 渲染线程侧的乐器实例 + 当前块事件累积器。
pub(crate) struct InstrumentSource {
    /// MIDI 全局通道（0..256，与 `TrackData::global_channel()` 对齐）。回收退回 UI 时携带。
    pub channel: u8,
    /// activate 后 move 进来的处理器（渲染线程独占）。
    pub processor: Box<dyn InstrumentProcessor>,
    /// 当前块累积的输入事件（带块内 sample offset）。`process` 前填充、
    /// 处理后清空。后端（CLAP/VST3）会按 `time` 排序，这里无需保证顺序。
    pub events: Vec<PluginEvent>,
}

impl InstrumentSource {
    pub fn new(channel: u8, processor: Box<dyn InstrumentProcessor>) -> Self {
        Self {
            channel,
            processor,
            events: Vec::new(),
        }
    }
}
