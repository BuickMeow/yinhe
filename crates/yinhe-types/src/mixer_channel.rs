//! 混音通道身份。
//!
//! MIDI 轨的混音通道就是它的端口/通道，音频轨就是音频通道；两者不再有
//! 独立的覆盖字段。通道再通过 `MixerParams` 的主输出表进入 Bus 或 Master。
//! 文件夹只是轨道组织容器，不属于混音通道，也不参与音频路由。

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
    Bus(crate::BusId),
    /// Master 总输出。
    Master,
}

impl MixerChannelId {
    /// 是否是可承载音符/音频片段的源通道（而不是 Bus/Master）。
    pub fn is_source(self) -> bool {
        matches!(self, Self::Midi(_) | Self::Audio(_))
    }
}
