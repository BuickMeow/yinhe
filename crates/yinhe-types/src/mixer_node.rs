//! 混音架构的稳定身份类型。
//!
//! 这里只放「身份」，不放参数：轨道、Rack、逻辑通道、Bus、Master 各自有
//! 独立的命名空间。稳定 ID 不随插入/删除/重排变化，自动化、发送、撤销和
//! 工程保存都靠它定位对象。
//!
//! 旧工程没有这些字段时，由加载/补齐逻辑按原索引生成 ID，保证行为不变。

use serde::{Deserialize, Serialize};

/// Rack 数量上限（0..256）。Rack 是共享乐器资源，不是混音节点。
pub const RACK_COUNT: usize = 256;

/// 每轨 LC 的 ID 起点。
///
/// `0..256` 留给 MIDI 源通道，`256..` 留给音频通道（可能到 65535），
/// 每轨 LC 从高位开始，避免与现有命名空间碰撞。
pub const TRACK_LC_BASE: u32 = 1_000_000;

/// Rack 的音频输出拓扑。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RackOutputMode {
    /// 整个 Rack 一条输出；所有使用该 Rack 的 Track 共享一条 LC。
    #[default]
    Shared,
    /// 内置音源按 Track 拆分音频；每条 Track 有自己的 LC。
    /// 外部单输出插件不支持该模式。
    PerTrack,
}

/// 共享乐器 Rack 的稳定身份。
///
/// 当前与源 MIDI 通道一一对应（A01 = 0，P16 = 255）；目标 B 中 Track 通过
/// `RackId` 选择乐器，多个 Track 可以共享同一个 Rack。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct RackId(pub u16);

impl RackId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    /// 源 MIDI 通道（port * 16 + channel）对应的默认 Rack。
    pub const fn from_midi_channel(channel: u8) -> Self {
        Self(channel as u16)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// 逻辑通道（LC）的稳定身份。
///
/// LC 是一条音频流的混音通道条，不是 MIDI 轨道。由 Rack 输出拓扑决定：
/// per-track 输出时一轨一条，单输出插件时整个 Rack 一条。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LogicalChannelId(pub u32);

impl LogicalChannelId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// 总线 / Return 的稳定身份。
///
/// 发送和主输出存 BusId，不存 Vec 下标；删除总线不会让后续总线「继承」
/// 前一条的路由。旧工程加载时按原索引 0..n 生成，行为与旧版一致。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct BusId(pub u16);

impl BusId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<u16> for BusId {
    fn from(raw: u16) -> Self {
        Self(raw)
    }
}

/// 混音图中一个音频节点的身份。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixerNodeId {
    /// 一条音频流的通道条。
    LogicalChannel(LogicalChannelId),
    /// 总线 / Return。
    Bus(BusId),
    /// 最终总输出。
    Master,
}

/// 自动化 lane 的归属。
///
/// 自动化不再默认「挂在轨道上」：MIDI 内容轨的 lane 属于 Track，乐器参数
/// 属于 Rack，推子/声像/发送属于 LC/Bus/Master，Tempo 属于 Conductor。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationOwner {
    Track(u16),
    Rack(RackId),
    LogicalChannel(LogicalChannelId),
    Bus(BusId),
    Master,
    Conductor,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_channel_maps_to_default_rack() {
        assert_eq!(RackId::from_midi_channel(0), RackId(0));
        assert_eq!(RackId::from_midi_channel(0x25), RackId(0x25));
        assert_eq!(RackId::from_midi_channel(255).index(), 255);
    }

    #[test]
    fn ids_are_stable_value_types() {
        assert_eq!(LogicalChannelId::new(7).index(), 7);
        assert_eq!(BusId::new(3).index(), 3);
        assert_eq!(BusId::from(9), BusId(9));
        assert_eq!(format!("{:?}", MixerNodeId::Bus(BusId(2))), "Bus(BusId(2))");
    }
}
