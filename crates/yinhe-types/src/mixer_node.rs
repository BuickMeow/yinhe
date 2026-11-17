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

/// 新建 LC 的 ID 分配起点。
///
/// 方案 C 的 LC ID 是独立的 u16 身份，不再编码 MIDI/音频源通道号；
/// 这个常量只是新 ID 的分配起点，避免与工程里已有的小 ID 碰撞。
pub const TRACK_LC_BASE: u16 = 4096;

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

/// 逻辑通道（LC）的稳定身份（独立 u16 命名空间）。
///
/// LC 是一条音频流的混音通道条，不是 MIDI 轨道。ID 不再编码源通道号，
/// 而是独立身份；角色由 [`ChannelSource`] 显式记录。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LogicalChannelId(pub u16);

impl LogicalChannelId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// LC 的来源角色（显式记录，不再从 ID 数值区间反推）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSource {
    /// MIDI 源通道 LC（AP 通道载体）；`ch` = global = port*16+channel。
    Midi(u8),
    /// 音频轨 home LC；`ch` = `TrackData::audio_channel`。
    Audio(u16),
    /// 每轨 home LC；`track` = `YinModel.tracks` 下标。
    Track(u16),
    /// AP 编组的通道。
    Ap(ApGroupId),
    /// Bus / Return（source=None 角色）。
    Bus,
    /// Master。
    Master,
    /// 旧数据未标注，加载时补齐。
    #[default]
    Unspecified,
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

/// 听感三态：正常 / 静音 / 独奏。
///
/// 不能用两个 bool：同一对象不能同时 muted + soloed；UI 单按钮也直接映射
/// 这个枚举。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audibility {
    #[default]
    Normal,
    Muted,
    Soloed,
}

impl Audibility {
    pub const fn is_muted(self) -> bool {
        matches!(self, Self::Muted)
    }

    pub const fn is_soloed(self) -> bool {
        matches!(self, Self::Soloed)
    }
}

/// AP 编组的稳定身份。
///
/// 当前上限 256（A01..P16），u16 预留扩展；AP 必须绑定一个 MIDI 通道。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ApGroupId(pub u16);

impl ApGroupId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// AP 展开出的某一条音频通道。
///
/// 方案 C MVP 每个 AP 固定一条输出，`output = 0`；后期多输出时再扩展。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ApChannelId {
    pub group: ApGroupId,
    pub output: u16,
}

impl ApChannelId {
    pub const fn new(group: ApGroupId, output: u16) -> Self {
        Self { group, output }
    }
}

/// 所有可发送目标的统一身份。
///
/// LC 与 AP 通道都是普通音频通道；Master 是唯一终点。Bus 不是独立类型，
/// 它是 `ChannelSource::None` 的通道角色，因此也落在这两类里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelId {
    Lc(LogicalChannelId),
    Ap(ApChannelId),
    /// 旧 Bus 节点在 C 发送图里的通道身份（过渡；UI 按 CH 处理）。
    Bus(BusId),
    Master,
}

impl ChannelId {
    pub const fn lc(id: LogicalChannelId) -> Self {
        Self::Lc(id)
    }

    pub const fn ap(group: ApGroupId, output: u16) -> Self {
        Self::Ap(ApChannelId { group, output })
    }

    pub const fn master() -> Self {
        Self::Master
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

impl MixerNodeId {
    /// 混音节点的 C 统一身份。
    pub const fn c_channel_id(self) -> ChannelId {
        match self {
            Self::LogicalChannel(id) => ChannelId::Lc(id),
            Self::Bus(bus) => ChannelId::Bus(bus),
            Self::Master => ChannelId::Master,
        }
    }
}

impl From<MixerNodeId> for ChannelId {
    fn from(id: MixerNodeId) -> Self {
        id.c_channel_id()
    }
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
        assert_eq!(
            MixerNodeId::LogicalChannel(LogicalChannelId(7)).c_channel_id(),
            ChannelId::Lc(LogicalChannelId(7))
        );
        assert_eq!(
            MixerNodeId::Bus(BusId(2)).c_channel_id(),
            ChannelId::Bus(BusId(2))
        );
        assert_eq!(MixerNodeId::Master.c_channel_id(), ChannelId::Master);
    }

    #[test]
    fn c_channel_ids_cover_lc_ap_and_master() {
        let lc = ChannelId::lc(LogicalChannelId(3));
        let ap = ChannelId::ap(ApGroupId(2), 0);
        assert_ne!(lc, ap);
        assert_ne!(lc, ChannelId::Master);
        assert_eq!(ap, ChannelId::Ap(ApChannelId::new(ApGroupId(2), 0)));
        assert_eq!(ChannelId::master(), ChannelId::Master);
        assert_eq!(ApGroupId::new(9).index(), 9);
    }
}
