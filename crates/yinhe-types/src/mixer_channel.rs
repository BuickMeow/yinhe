//! 混音通道身份。
//!
//! MIDI 轨的混音通道就是它的端口/通道，音频轨就是音频通道；两者不再有
//! 独立的覆盖字段。通道再通过 `MixerParams` 的主输出表进入 Bus 或 Master。
//! 文件夹只是轨道组织容器，不属于混音通道，也不参与音频路由。

use serde::{Deserialize, Serialize};

use crate::LogicalChannelId;

/// 一条混音通道（Rack 设备）的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixerChannelId {
    /// MIDI 源通道（global = port*16 + channel，0..256）。
    Midi(u8),
    /// 音频通道（0..u16::MAX）。
    Audio(u16),
    /// 每轨独立 LC（Rack per-track 输出）。
    TrackLc(LogicalChannelId),
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

    /// 显式携带 LC 身份的只有 TrackLc。
    ///
    /// 方案 C 的 LC ID 是独立 u16，Midi/Audio 通道的 LC 由 `MixerParams` 的
    /// 角色映射（ChannelSource）解析，不再从 ID 数值区间反推。
    pub fn logical_channel_id(self) -> Option<LogicalChannelId> {
        match self {
            Self::TrackLc(id) => Some(id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BusId;

    #[test]
    fn only_track_lc_carries_explicit_lc_id() {
        let lc = LogicalChannelId(4096);
        assert_eq!(MixerChannelId::TrackLc(lc).logical_channel_id(), Some(lc));
        assert_eq!(MixerChannelId::Midi(9).logical_channel_id(), None);
        assert_eq!(MixerChannelId::Audio(3).logical_channel_id(), None);
        assert_eq!(MixerChannelId::Bus(BusId(2)).logical_channel_id(), None);
        assert_eq!(MixerChannelId::Master.logical_channel_id(), None);
    }

    #[test]
    fn source_channels_are_recognized() {
        assert!(MixerChannelId::Midi(0).is_source());
        assert!(MixerChannelId::Audio(1).is_source());
        assert!(!MixerChannelId::TrackLc(LogicalChannelId(1)).is_source());
        assert!(!MixerChannelId::Bus(BusId(0)).is_source());
        assert!(!MixerChannelId::Master.is_source());
    }
}
