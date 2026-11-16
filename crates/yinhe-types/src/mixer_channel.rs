//! 混音通道身份。
//!
//! MIDI 轨的混音通道就是它的端口/通道，音频轨就是音频通道；两者不再有
//! 独立的覆盖字段。通道再通过 `MixerParams` 的主输出表进入 Bus 或 Master。
//! 文件夹只是轨道组织容器，不属于混音通道，也不参与音频路由。

use serde::{Deserialize, Serialize};

use crate::{LogicalChannelId, MixerNodeId};

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

    /// 目标 A → 目标 B 的逻辑通道身份映射。
    ///
    /// 当前 MIDI 通道条和音频通道条各自就是一条 LC；迁移期用确定性 ID：
    /// - MIDI `ch` → `LC(ch)`
    /// - 音频 `ch` → `LC(256 + ch)`
    /// - BUS / Master 不是 LC。
    pub fn logical_channel_id(self) -> Option<LogicalChannelId> {
        match self {
            Self::Midi(ch) => Some(LogicalChannelId(u32::from(ch))),
            Self::Audio(ch) => Some(LogicalChannelId(
                crate::MIDI_CHANNEL_COUNT as u32 + u32::from(ch),
            )),
            Self::Bus(_) | Self::Master => None,
        }
    }

    /// 混音图中的节点身份。
    pub fn node_id(self) -> MixerNodeId {
        match self {
            Self::Midi(ch) => MixerNodeId::LogicalChannel(LogicalChannelId(u32::from(ch))),
            Self::Audio(ch) => MixerNodeId::LogicalChannel(LogicalChannelId(
                crate::MIDI_CHANNEL_COUNT as u32 + u32::from(ch),
            )),
            Self::Bus(bus) => MixerNodeId::Bus(bus),
            Self::Master => MixerNodeId::Master,
        }
    }
}

impl MixerNodeId {
    /// 迁移期反查：LC ID 落在 MIDI/音频区间时还原为源通道。
    ///
    /// 目标 B 真正落地后，Track 会直接存 LC ID，这个反查只服务于旧路径。
    pub fn channel_id(self) -> Option<MixerChannelId> {
        match self {
            Self::LogicalChannel(id) => {
                let raw = id.0;
                if raw < crate::MIDI_CHANNEL_COUNT as u32 {
                    Some(MixerChannelId::Midi(raw as u8))
                } else {
                    let audio = raw - crate::MIDI_CHANNEL_COUNT as u32;
                    u16::try_from(audio).ok().map(MixerChannelId::Audio)
                }
            }
            Self::Bus(bus) => Some(MixerChannelId::Bus(bus)),
            Self::Master => Some(MixerChannelId::Master),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BusId;

    #[test]
    fn logical_channel_ids_are_deterministic_for_legacy_sources() {
        assert_eq!(
            MixerChannelId::Midi(9).logical_channel_id(),
            Some(LogicalChannelId(9))
        );
        assert_eq!(
            MixerChannelId::Audio(3).logical_channel_id(),
            Some(LogicalChannelId(259))
        );
        assert_eq!(MixerChannelId::Bus(BusId(2)).logical_channel_id(), None);
        assert_eq!(MixerChannelId::Master.logical_channel_id(), None);
    }

    #[test]
    fn node_ids_roundtrip_legacy_sources() {
        for ch in [0u16, 9, 255] {
            let id = MixerChannelId::Midi(ch as u8);
            assert_eq!(id.node_id().channel_id(), Some(id));
        }
        for ch in [0u16, 31, u16::MAX] {
            let id = MixerChannelId::Audio(ch);
            assert_eq!(id.node_id().channel_id(), Some(id));
        }
        let bus = MixerChannelId::Bus(BusId(7));
        assert_eq!(bus.node_id().channel_id(), Some(bus));
        assert_eq!(
            MixerChannelId::Master.node_id().channel_id(),
            Some(MixerChannelId::Master)
        );
    }
}
