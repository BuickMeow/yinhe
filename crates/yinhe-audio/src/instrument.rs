//! 渲染线程侧的一条 MIDI 通道乐器链。
//!
//! 生命周期与混音台效果器 insert 一致（见 clap_insert.rs 线程模型）：
//! - UI/管理线程：插件实例 `activate()` 产出 [`InstrumentProcessor`]；
//! - 渲染线程：经 `AudioCommand::SetInstrumentSlot` 按 **slot_id** 增删/替换链内
//!   单个乐器，同一通道的块事件广播给链内每个乐器，输出**求和**写进该通道缓冲；
//! - 回收：替换/移除时由渲染线程退回 → UI 线程按 slot_id 交还实例 deactivate。
//!
//! 一个 MIDI 通道（`TrackData::global_channel()`）可挂**多个**乐器（叠加）；空链时
//! 该通道使用内置 XSynth。同通道的多条轨共享同一链（音符按各自 MIDI channel 路由
//! 进插件，插件自己多音色）。

use yinhe_mixer::{InstrumentProcessor, PluginEvent};

/// 链内一个乐器处理器 + 其 UI 侧槽位 id（回调 deactivate 时按 id 匹配实例）。
pub(crate) struct InstrumentProc {
    /// UI 侧分配的稳定槽位 id（同一通道内唯一）。
    pub slot_id: u64,
    /// activate 后 move 进来的处理器（渲染线程独占）。
    pub processor: Box<dyn InstrumentProcessor>,
}

/// 渲染线程侧一个通道的乐器链 + 当前块事件累积器。
pub(crate) struct InstrumentSource {
    /// 按挂载顺序排列的乐器链（输出相加）。
    pub chain: Vec<InstrumentProc>,
    /// 当前块累积的输入事件（带块内 sample offset）。`process` 前填充、
    /// 处理后清空。同一份事件广播给链内每个处理器的 `process`。
    pub events: Vec<PluginEvent>,
}

impl InstrumentSource {
    pub fn new() -> Self {
        Self {
            chain: Vec::new(),
            events: Vec::new(),
        }
    }
}
