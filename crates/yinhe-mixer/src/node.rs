//! 混音节点与 Rack 的持久化结构。
//!
//! 目标 B 的层级：
//! ```text
//! Track -> Rack（共享乐器） -> Logical Channel（音频通道条） -> Bus -> Master
//! ```
//!
//! `MixerNode` 是 LC / Bus / Master 共用的通道条；`RackParams` 只装乐器，
//! 不装推子/insert。当前 MIDI 通道和音频通道各对应一条 LC，ID 由
//! [`MixerChannelId::logical_channel_id`](yinhe_types::MixerChannelId::logical_channel_id)
//! 确定性生成，方便 v9 工程无损迁移。

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use yinhe_types::{AutomationLane, BusId, LogicalChannelId, MixerNodeId, RackId, RackOutputMode};

use crate::params::{InsertRef, SendParams, StripParams};

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

/// 一条 LC / Bus / Master 的持久化参数。
///
/// `Deref` 到 [`StripParams`] 让推子/声像/M/S 的读写保持
/// `node.gain` 这种自然写法；`inserts` / `sends` / `output` 是节点独有字段。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerNode {
    pub id: MixerNodeId,
    #[serde(default)]
    pub strip: StripParams,
    #[serde(default)]
    pub inserts: Vec<InsertRef>,
    #[serde(default)]
    pub sends: Vec<SendParams>,
    /// 主输出；`None` = 直接进 Master。
    ///
    /// 当前 MIDI / 音频源通道使用该字段；Bus 的最终输出固定为 Master，
    /// 字段为空。目标 B 的 Bus 嵌套会复用同一个字段。
    #[serde(default)]
    pub output: Option<BusId>,
    /// 节点自己的自动化 lane。BusGain 等混音自动化按 owner 存在这里，
    /// 不再寄宿 Master 轨。
    #[serde(default)]
    pub automation: Vec<AutomationLane>,
}

impl MixerNode {
    pub fn new(id: MixerNodeId) -> Self {
        Self {
            id,
            strip: StripParams::default(),
            inserts: Vec::new(),
            sends: Vec::new(),
            output: None,
            automation: Vec::new(),
        }
    }

    pub fn logical_channel(id: LogicalChannelId) -> Self {
        Self::new(MixerNodeId::LogicalChannel(id))
    }

    pub fn bus(id: BusId) -> Self {
        Self::new(MixerNodeId::Bus(id))
    }

    pub fn master() -> Self {
        Self::new(MixerNodeId::Master)
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
