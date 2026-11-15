//! 混音通道身份：Track 与 Mixer/Rack 解耦后的稳定路由目标。
//!
//! 当前引擎仍按 MIDI/音频通道混音；该类型先作为 Track→Channel 路由的
//! 数据模型与持久化身份，后续引擎改为按 MixerChannel 建图时直接复用。

use serde::{Deserialize, Serialize};

/// 一条混音通道（Rack 设备）的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixerChannelId {
    /// MIDI 源通道（global = port*16 + channel，0..256）。
    Midi(u8),
    /// 音频通道（0..u16::MAX）。
    Audio(u16),
    /// 总线 / Return 通道。
    Bus(u16),
    /// Master 总输出。
    Master,
}

impl MixerChannelId {
    /// 是否是可承载音符/音频片段的源通道（而不是 Bus/Master）。
    pub fn is_source(self) -> bool {
        matches!(self, Self::Midi(_) | Self::Audio(_))
    }
}
