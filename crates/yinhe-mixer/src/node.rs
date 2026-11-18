//! 混音节点与 Rack 的持久化结构。
//!
//! 方案 C 的层级：
//! ```text
//! Track -> Rack（共享乐器） -> Channel（LC / AP 通道条） -> Master
//! ```
//!
//! `MixerNode` 是 LC / AP / Master 共用的通道条；`RackParams` 只装乐器，
//! 不装推子/insert。通道身份统一为 `ChannelId`，每轨 LC 的 ID 独立分配。

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use yinhe_types::{
    ApGroupId, AutomationLane, ChannelId, ChannelSource, LogicalChannelId, RackId, RackOutputMode,
};

use crate::params::{InsertRef, StripParams};

/// 一个 Rack 的持久化参数。
///
/// Rack 是共享乐器资源：多条 Track 可以指向同一个 Rack，共享乐器链与
/// 样本内存。推子、insert、发送属于 LC，不属于 Rack。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RackParams {
    pub id: RackId,
    /// 按序叠加的乐器插件；空 = 该 Rack 使用默认内置 XSynth。
    #[serde(default)]
    pub instruments: Vec<InsertRef>,
    /// 输出拓扑：共享一条 LC，或内置音源按 Track 拆分。
    #[serde(default)]
    pub output_mode: RackOutputMode,
}

impl RackParams {
    pub fn new(id: RackId) -> Self {
        Self {
            id,
            instruments: Vec::new(),
            output_mode: RackOutputMode::Shared,
        }
    }
}

/// 方案 C 的 AP 编组。
///
/// AP 现在是独立音频通道：节点身份是 `ChannelId::Ap`，不再借用
/// 共享 MIDI LC；`rack` 仍指向共享的 `RackParams`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApGroup {
    pub id: ApGroupId,
    /// AP 绑定的 MIDI 通道（port, channel）。
    pub midi: (u8, u8),
    /// AP 的共享 Rack。
    pub rack: RackId,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

const fn default_true() -> bool {
    true
}

impl ApGroup {
    pub fn new(id: ApGroupId, midi: (u8, u8), rack: RackId) -> Self {
        Self {
            id,
            midi,
            rack,
            enabled: true,
        }
    }

    /// MIDI global channel = port * 16 + channel。
    pub fn global_midi(&self) -> u8 {
        (self.midi.0 & 0x0F) << 4 | (self.midi.1 & 0x0F)
    }
}

/// 冻结音频引用：播放时用这段离线渲染音频替代该 LC 的实时输出。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenAudio {
    /// 工程内嵌音频素材的 uuid（`YinModel.audio_sources`）。
    pub source: String,
    /// 冻结时工程 `ProjectData::revision`；revision 变化即视为失效。
    pub source_revision: u64,
    pub sample_rate: u32,
    /// 冻结从工程 sample 0 开始，长度 = 整曲。
    pub frames: u64,
}

/// 一条 LC / Master 的持久化参数。
///
/// `Deref` 到 [`StripParams`] 让推子/声像/M/S 的读写保持
/// `node.gain` 这种自然写法；`inserts` / `sends` / `output` 是节点独有字段。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerNode {
    pub id: ChannelId,
    /// 该节点的来源角色（显式记录，不从 ID 数值反推）。
    pub source: ChannelSource,
    #[serde(default)]
    pub strip: StripParams,
    #[serde(default)]
    pub inserts: Vec<InsertRef>,
    /// 主输出；`None` = 直接进 Master。
    ///
    /// MIDI / 音频 / 每轨 LC 可指向另一条 LC，参数层做环检测，
    /// 渲染图对逆序输出使用一块延迟。
    #[serde(default)]
    pub output: Option<ChannelId>,
    /// 节点自己的自动化 lane。推子/声像等混音自动化按 owner 存在这里，
    /// 不再寄宿 Master 轨。
    #[serde(default)]
    pub automation: Vec<AutomationLane>,
    /// 冻结音频；`Some` 时播放用冻结源替代实时输出。仅 LC 使用。
    #[serde(default)]
    pub freeze: Option<FrozenAudio>,
}

impl MixerNode {
    /// 构造节点：必须显式给出角色，不再有「未知来源」的中间态。
    pub fn new(id: ChannelId, source: ChannelSource) -> Self {
        Self {
            id,
            source,
            strip: StripParams::default(),
            inserts: Vec::new(),
            output: None,
            automation: Vec::new(),
            freeze: None,
        }
    }

    pub fn logical_channel(id: LogicalChannelId, source: ChannelSource) -> Self {
        Self::new(ChannelId::Lc(id), source)
    }

    pub fn master() -> Self {
        Self::new(ChannelId::Master, ChannelSource::Master)
    }
}

impl MixerNode {
    /// 节点的 C 统一身份。
    pub const fn channel_id(&self) -> ChannelId {
        self.id
    }
}

impl Deref for MixerNode {
    type Target = StripParams;

    fn deref(&self) -> &StripParams {
        &self.strip
    }
}

impl DerefMut for MixerNode {
    fn deref_mut(&mut self) -> &mut StripParams {
        &mut self.strip
    }
}
