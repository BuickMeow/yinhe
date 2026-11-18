//! 混音台持久化参数（serde，进工程文件）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use yinhe_types::{
    ApGroupId, Audibility, ChannelId, ChannelSource, LogicalChannelId, MixerChannelId, MixerNodeId,
    RackId, TRACK_LC_BASE,
};

use crate::node::{ApGroup, MixerNode, RackParams};

/// 源 MIDI 通道总数：A01..P16（16 port × 16 通道）。
pub const CHANNEL_COUNT: usize = 256;

/// 单个通道条的持久化参数。
///
/// 注意与 MIDI CC7/11（音量/表情）区分：那是乐曲内容、作用于合成器内部；
/// 这里的 gain/pan 是工程混音设置、作用于音频域，两层串联互不相干。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StripParams {
    /// 线性增益（1.0 = 0 dB）。
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// 声像，-1.0（左）~ 1.0（右），0.0 居中。
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
}

const fn default_gain() -> f32 {
    1.0
}

const fn default_instrument_uid() -> u64 {
    1
}

impl StripParams {
    /// 三态听感：Soloed 优先于 Muted（旧工程两者都为 true 时 solo 生效）。
    pub fn audibility(&self) -> Audibility {
        if self.solo {
            Audibility::Soloed
        } else if self.mute {
            Audibility::Muted
        } else {
            Audibility::Normal
        }
    }

    pub fn set_audibility(&mut self, audibility: Audibility) {
        self.mute = audibility == Audibility::Muted;
        self.solo = audibility == Audibility::Soloed;
    }
}

impl Default for StripParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}

/// 主输出的持久化参数。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterParams {
    /// 线性增益（1.0 = 0 dB）。
    #[serde(default = "default_gain")]
    pub gain: f32,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

/// 插件格式（持久化进工程；旧工程缺省按 CLAP 处理）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    #[default]
    Clap,
    Vst3,
    /// 内置效果器（yinhe-dsp）。`InsertRef.plugin_id` 为内置效果器标识
    /// （见 `yinhe_dsp::BuiltinEffectKind::id`），`plugin_path` 为空。
    Builtin,
}

/// insert 槽位的插件引用（持久化进工程文件）。
///
/// 插件本体（实例/处理器）由上层（yinhe-egui）管理，这里只存
/// 「哪个插件 + 是否旁通 + 状态字节」。加载时按 id 为主、路径为辅找回插件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertRef {
    /// 工程内稳定唯一 id（仅乐器用：自动化按此寻址到具体实例，增删/排序不受影响）。
    /// 0 = 未分配（旧工程），加载时由 `ensure_instrument_uids` 补号。
    #[serde(default)]
    pub uid: u64,
    /// 插件包路径（如 .clap 文件 / .vst3 bundle 目录）。
    pub plugin_path: PathBuf,
    /// 包内插件 id（CLAP id / VST3 32 位十六进制 class id）。
    pub plugin_id: String,
    /// 显示名（持久化：恢复时扫描结果可能不含该插件，仍能显示原名）。
    #[serde(default)]
    pub name: String,
    /// 插件格式（旧工程缺省 CLAP）。
    #[serde(default)]
    pub format: PluginFormat,
    /// 旁通：链上保留槽位但不参与处理。
    #[serde(default)]
    pub bypassed: bool,
    /// 插件状态字节（CLAP state 扩展 / VST3 component+controller 两段）；None = 未保存过。
    #[serde(default)]
    pub state: Option<Vec<u8>>,
}

/// 任意音频通道之间的发送（方案 C）。
///
/// source/target 都是 `ChannelId`；渲染图按它解析成 dense 下标。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelSend {
    pub source: ChannelId,
    pub target: ChannelId,
    /// 0..1；0 = OFF（-inf dB），不要保留 0 发送条目。
    #[serde(default)]
    pub amount: f32,
    #[serde(default)]
    pub pre_fader: bool,
}

impl ChannelSend {
    pub fn new(source: ChannelId, target: ChannelId, amount: f32, pre_fader: bool) -> Self {
        Self {
            source,
            target,
            amount: amount.clamp(0.0, 1.0),
            pre_fader,
        }
    }
}

/// 整个混音台的持久化参数。
///
/// 存储已从 v9 的并行分表合并为节点表：
/// - `racks[i]`：Rack（共享乐器），当前按源 MIDI 通道 0..256 一一对应；
/// - `nodes[i]`：LC 节点（MIDI / 音频 / 每轨 / AP），ID 独立且稳定；
/// - `master`：Master 节点。
///
/// v9 及更早工程由 `yinhe-yin` 的混音段迁移器转换成这个结构，
/// 每个旧 MIDI/音频通道条变成一条 LC，听感保持不变。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerParams {
    /// 共享乐器 Rack（固定 256 个槽位）。
    #[serde(default)]
    pub racks: Vec<RackParams>,
    /// 方案 C 的 AP 编组；只有实际共享 MIDI 通道时才创建。
    ///
    /// 过渡期 `ApGroup::rack/channel` 指向 `racks` / `channels` 里的共享
    /// Rack 和共享 LC，音频行为不变。AP 目前完全由 Track 推导，先不落盘；
    /// 等 AP 拥有独立设置后再加字段版本迁移，避免破坏现有 v14 工程。
    #[serde(skip)]
    pub aps: Vec<ApGroup>,
    /// 方案 C 的任意通道发送；source/target 都是 `ChannelId`。
    #[serde(default)]
    pub channel_sends: Vec<ChannelSend>,
    /// 所有非 Master 音频通道节点，按 `ChannelId` 排序。
    ///
    /// LC ID 是独立 u16 身份；节点的角色由 `MixerNode::source` 显式记录，
    /// 不再从 ID 数值区间反推。
    #[serde(default)]
    pub nodes: Vec<MixerNode>,
    /// 主输出节点。
    #[serde(default = "MixerNode::master")]
    pub master: MixerNode,
    /// 下一个乐器实例 uid（工程内唯一、持久化；加载时对齐到已用最大值 +1）。
    #[serde(default = "default_instrument_uid")]
    pub next_instrument_uid: u64,
    /// 下一个可分配的 LC ID（独立 u16 身份）。
    #[serde(default = "default_next_lc_id")]
    pub next_lc_id: LogicalChannelId,
}

fn default_next_lc_id() -> LogicalChannelId {
    LogicalChannelId(TRACK_LC_BASE)
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            racks: (0..CHANNEL_COUNT)
                .map(|i| RackParams::new(RackId(i as u16)))
                .collect(),
            aps: Vec::new(),
            channel_sends: Vec::new(),
            nodes: Vec::new(),
            master: MixerNode::master(),
            next_instrument_uid: 1,
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
        }
    }
}

mod repair;

impl MixerParams {
    /// 反序列化后调用：按依赖顺序修复老工程缺失/不一致的混音表。
    ///
    /// 顺序不能调换：
    /// 1. `repair_racks` / `repair_channels` / `repair_aps`：补齐基础表；
    /// 2. `repair_channel_sends`：依赖 LC/AP 已存在；
    /// 3. `repair_track_channels` / `repair_audio_channels`：整理每轨节点；
    /// 4. `repair_mixed_lc_freezes`：依赖发送表；
    /// 5. `repair_master` / `ensure_instrument_uids`：单点补齐；
    /// 6. `repair_routing`：清理悬空目标；
    /// 7. `repair_automation`：最后按节点归属过滤 lane（依赖以上全部完成）。
    pub fn ensure_len(&mut self) {
        self.repair_racks();
        self.repair_channels();
        self.repair_aps();
        self.repair_channel_sends();
        self.repair_track_channels();
        self.repair_audio_channels();
        self.repair_mixed_lc_freezes();
        self.repair_master();
        self.ensure_instrument_uids();
        self.repair_routing();
        self.repair_automation();
    }

    /// 所有非 Master 混音节点（统一视图；存储仍按旧分表过渡）。
    pub fn nodes(&self) -> impl Iterator<Item = &MixerNode> {
        self.nodes.iter()
    }

    /// 所有非 Master 混音节点（可变统一视图）。
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = &mut MixerNode> {
        self.nodes.iter_mut()
    }

    /// 按来源角色查节点（LC ID 独立，不再编码源通道）。
    pub fn node_by_source(&self, source: ChannelSource) -> Option<&MixerNode> {
        self.nodes.iter().find(|node| node.source == source)
    }

    pub fn node_by_source_mut(&mut self, source: ChannelSource) -> Option<&mut MixerNode> {
        self.nodes.iter_mut().find(|node| node.source == source)
    }

    pub fn midi_node(&self, ch: u8) -> Option<&MixerNode> {
        self.node_by_source(ChannelSource::Midi(ch))
    }

    pub fn midi_node_mut(&mut self, ch: u8) -> Option<&mut MixerNode> {
        self.node_by_source_mut(ChannelSource::Midi(ch))
    }

    pub fn audio_node(&self, ch: u16) -> Option<&MixerNode> {
        self.node_by_source(ChannelSource::Audio(ch))
    }

    pub fn audio_node_mut(&mut self, ch: u16) -> Option<&mut MixerNode> {
        self.node_by_source_mut(ChannelSource::Audio(ch))
    }

    pub fn track_node(&self, id: LogicalChannelId) -> Option<&MixerNode> {
        self.find_node(ChannelId::Lc(id))
    }

    pub fn track_node_mut(&mut self, id: LogicalChannelId) -> Option<&mut MixerNode> {
        self.find_node_mut(ChannelId::Lc(id))
    }

    /// 节点的来源角色。
    pub fn source_of(&self, id: ChannelId) -> Option<ChannelSource> {
        if id == ChannelId::Master {
            return Some(ChannelSource::Master);
        }
        self.find_node(id).map(|node| node.source)
    }

    /// UI 查询键 → 实际 ChannelId（经来源角色解析）。
    pub fn channel_id_for(&self, channel: MixerChannelId) -> ChannelId {
        match channel {
            MixerChannelId::Midi(ch) => self
                .midi_node(ch)
                .map(|node| node.id)
                .unwrap_or(ChannelId::Master),
            MixerChannelId::Audio(ch) => self
                .audio_node(ch)
                .map(|node| node.id)
                .unwrap_or(ChannelId::Master),
            MixerChannelId::TrackLc(id) => ChannelId::Lc(id),
            MixerChannelId::Master => ChannelId::Master,
        }
    }

    /// UI 查询键 → 实际 `ChannelId`（AP 感知）。
    ///
    /// 与 [`Self::channel_id_for`] 的区别：`Midi(ch)` 有 AP 编组时返回
    /// `ChannelId::Ap`（UI 的通道身份），否则返回该通道的 LC。
    /// 引擎/持久化用 `channel_id_for`，UI 展示与发送面板用本方法。
    pub fn channel_id_for_ui(&self, channel: MixerChannelId) -> ChannelId {
        if let MixerChannelId::Midi(ch) = channel
            && let Some(ap) = self.ap_for_midi(ch)
        {
            return ChannelId::Ap(yinhe_types::ApChannelId::new(ap.id, 0));
        }
        self.channel_id_for(channel)
    }

    /// 节点的 C 统一身份 → UI 查询键（[`Self::channel_id_for`] 的逆映射）。
    ///
    /// 自动化 target 用 `MixerChannelId` 表示 owner，渲染层需要从节点身份
    /// 反查这个键；放在模型层避免音频引擎引入 UI 概念。
    pub fn mixer_channel_of(&self, id: ChannelId) -> Option<MixerChannelId> {
        match id {
            ChannelId::Lc(lc) => match self.source_of(id)? {
                ChannelSource::Midi(ch) => Some(MixerChannelId::Midi(ch)),
                ChannelSource::Audio(ch) => Some(MixerChannelId::Audio(ch)),
                ChannelSource::Track(_) => Some(MixerChannelId::TrackLc(lc)),
                _ => None,
            },
            ChannelId::Ap(_) => None,
            ChannelId::Master => Some(MixerChannelId::Master),
        }
    }

    pub fn node_id_for(&self, channel: MixerChannelId) -> MixerNodeId {
        match self.channel_id_for(channel) {
            ChannelId::Lc(id) => MixerNodeId::LogicalChannel(id),
            ChannelId::Ap(_) => MixerNodeId::Master,
            ChannelId::Master => MixerNodeId::Master,
        }
    }

    /// 按 C 统一身份查节点；Master 映射到 `master`。
    pub fn channel_node(&self, id: ChannelId) -> Option<&MixerNode> {
        if id == ChannelId::Master {
            return Some(&self.master);
        }
        self.find_node(id)
    }

    pub fn channel_node_mut(&mut self, id: ChannelId) -> Option<&mut MixerNode> {
        if id == ChannelId::Master {
            return Some(&mut self.master);
        }
        self.find_node_mut(id)
    }

    pub fn ap(&self, id: ApGroupId) -> Option<&ApGroup> {
        self.aps.iter().find(|ap| ap.id == id)
    }

    pub fn ap_mut(&mut self, id: ApGroupId) -> Option<&mut ApGroup> {
        self.aps.iter_mut().find(|ap| ap.id == id)
    }

    pub fn ap_for_midi(&self, global: u8) -> Option<&ApGroup> {
        self.aps.iter().find(|ap| ap.global_midi() == global)
    }

    /// 为某 MIDI 通道创建 AP（如果还不存在）。过渡期 Rack/LC 复用当前共享表。
    pub fn ensure_ap(&mut self, global: u8) -> bool {
        if self.ap_for_midi(global).is_some() {
            return false;
        }
        let Some(rack) = self.racks.get(global as usize) else {
            return false;
        };
        let Some(channel) = self.midi_node(global).and_then(|node| match node.id {
            ChannelId::Lc(id) => Some(id),
            _ => None,
        }) else {
            return false;
        };
        self.aps.push(ApGroup::new(
            ApGroupId(u16::from(global)),
            (global >> 4, global & 0x0F),
            rack.id,
            channel,
        ));
        self.aps.sort_by_key(|ap| ap.id);
        true
    }

    /// 删除某 MIDI 通道的 AP，返回是否有变化。
    pub fn remove_ap_for_midi(&mut self, global: u8) -> bool {
        let before = self.aps.len();
        self.aps.retain(|ap| ap.global_midi() != global);
        self.aps.len() != before
    }

    fn channel_target_exists(&self, target: ChannelId) -> bool {
        match target {
            ChannelId::Master => true,
            ChannelId::Lc(id) => self.channel_exists(id),
            ChannelId::Ap(ap) => self.ap(ap.group).is_some(),
        }
    }

    /// 某通道发出的 C 发送。
    pub fn channel_sends_from(&self, source: ChannelId) -> impl Iterator<Item = &ChannelSend> {
        self.channel_sends
            .iter()
            .filter(move |s| s.source == source)
    }

    /// CH 显示编号：音频 LC + 每轨 LC 按稳定 ID 排序后 1..n。
    pub fn lc_display_map(&self) -> HashMap<LogicalChannelId, u32> {
        let mut ids: Vec<LogicalChannelId> = self
            .nodes
            .iter()
            .filter_map(|node| match (node.id, node.source) {
                (ChannelId::Lc(id), ChannelSource::Audio(_) | ChannelSource::Track(_)) => Some(id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .enumerate()
            .map(|(i, id)| (id, i as u32 + 1))
            .collect()
    }

    /// 所有可显示通道的 LC 编号。
    pub fn channel_display_map(&self) -> HashMap<ChannelId, u32> {
        self.lc_display_map()
            .into_iter()
            .map(|(id, n)| (ChannelId::Lc(id), n))
            .collect()
    }

    pub fn channel_send(&self, source: ChannelId, target: ChannelId) -> Option<&ChannelSend> {
        self.channel_sends
            .iter()
            .find(|s| s.source == source && s.target == target)
    }

    pub fn channel_send_amount(&self, source: ChannelId, target: ChannelId) -> f32 {
        self.channel_send(source, target)
            .map(|s| s.amount)
            .unwrap_or(0.0)
    }

    /// 设置一条发送；amount <= 0 等价于删除（-inf = OFF）。
    pub fn set_channel_send(
        &mut self,
        source: ChannelId,
        target: ChannelId,
        amount: f32,
        pre_fader: bool,
    ) -> bool {
        if source == ChannelId::Master || source == target {
            return false;
        }
        if amount <= 0.0 {
            return self.remove_channel_send(source, target);
        }
        let amount = amount.clamp(0.0, 1.0);
        if let Some(send) = self
            .channel_sends
            .iter_mut()
            .find(|s| s.source == source && s.target == target)
        {
            if (send.amount - amount).abs() < f32::EPSILON && send.pre_fader == pre_fader {
                return false;
            }
            send.amount = amount;
            send.pre_fader = pre_fader;
            return true;
        }
        self.channel_sends
            .push(ChannelSend::new(source, target, amount, pre_fader));
        true
    }

    pub fn remove_channel_send(&mut self, source: ChannelId, target: ChannelId) -> bool {
        let before = self.channel_sends.len();
        self.channel_sends
            .retain(|s| !(s.source == source && s.target == target));
        self.channel_sends.len() != before
    }

    fn find_node(&self, id: ChannelId) -> Option<&MixerNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    fn find_node_mut(&mut self, id: ChannelId) -> Option<&mut MixerNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// 确保某来源角色的 LC 节点存在，返回其独立 ID。
    pub fn ensure_lc_node(&mut self, source: ChannelSource) -> LogicalChannelId {
        if let Some(ChannelId::Lc(id)) = self
            .nodes
            .iter()
            .find(|node| node.source == source)
            .map(|node| node.id)
        {
            return id;
        }
        let id = self.alloc_lc_id();
        self.nodes.push(MixerNode::logical_channel(id, source));
        self.sort_dedup_nodes();
        id
    }

    /// 分配一个每轨 LC ID，并创建对应节点。
    pub fn add_track_lc(&mut self) -> LogicalChannelId {
        self.repair_track_channels();
        let id = self.alloc_lc_id();
        self.nodes.push(MixerNode::logical_channel(
            id,
            ChannelSource::Track(u16::MAX),
        ));
        self.sort_dedup_nodes();
        id
    }

    /// 用指定 ID 创建每轨 LC（加载修复用；ID 已存在则不重复创建）。
    pub fn add_track_lc_with_id(&mut self, id: LogicalChannelId) -> bool {
        if self.track_node(id).is_some() {
            return false;
        }
        self.nodes.push(MixerNode::logical_channel(
            id,
            ChannelSource::Track(u16::MAX),
        ));
        if id.0 >= self.next_lc_id.0 {
            self.next_lc_id = LogicalChannelId(id.0.saturating_add(1));
        }
        self.sort_dedup_nodes();
        true
    }

    /// 标记每轨 LC 归属的 track（方案 C 显式角色）。
    pub fn set_track_lc_source(&mut self, id: LogicalChannelId, track: u16) -> bool {
        match self.find_node_mut(ChannelId::Lc(id)) {
            Some(node) => {
                node.source = ChannelSource::Track(track);
                true
            }
            None => false,
        }
    }

    /// 按 ID 查每轨 LC。
    pub fn channel_exists(&self, id: LogicalChannelId) -> bool {
        self.find_node(ChannelId::Lc(id)).is_some()
    }

    /// 删除每轨 LC，并清理所有指向它的路由（当前没有外部引用，保留接口）。
    pub fn remove_track_lc(&mut self, id: LogicalChannelId) {
        self.nodes.retain(|node| node.id != ChannelId::Lc(id));
    }

    /// 分配一个新的乐器实例 uid（恒 ≥1）。
    pub fn alloc_instrument_uid(&mut self) -> u64 {
        let uid = self.next_instrument_uid.max(1);
        self.next_instrument_uid = uid + 1;
        uid
    }

    /// 加载后调用：给未分配 uid 的乐器补号，并把 `next_instrument_uid`
    /// 对齐到已用最大值 +1。幂等。
    pub fn ensure_instrument_uids(&mut self) {
        for rack in &mut self.racks {
            for r in rack.instruments.iter_mut() {
                if r.uid == 0 {
                    let uid = self.next_instrument_uid.max(1);
                    self.next_instrument_uid = uid + 1;
                    r.uid = uid;
                } else if r.uid >= self.next_instrument_uid {
                    self.next_instrument_uid = r.uid + 1;
                }
            }
        }
    }

    /// 去掉 revision 已不匹配的冻结（只用于推给音频引擎的副本）。
    ///
    /// 持久化层保留失效冻结，UI 可以显示“已失效”并允许重新冻结；
    /// 引擎拿到的副本不含失效冻结，播放回退到实时输出。
    pub fn strip_stale_freezes(&mut self, revision: u64) {
        for node in self.nodes_mut() {
            let stale = node
                .freeze
                .as_ref()
                .is_some_and(|freeze| freeze.source_revision != revision);
            if stale {
                node.freeze = None;
            }
        }
        if let Some(freeze) = self.master.freeze.as_ref()
            && freeze.source_revision != revision
        {
            self.master.freeze = None;
        }
    }

    /// 某 Rack 的乐器链（越界返回空切片）。
    pub fn rack_instruments(&self, rack: RackId) -> &[InsertRef] {
        self.racks
            .get(rack.index())
            .map(|r| r.instruments.as_slice())
            .unwrap_or(&[])
    }

    /// 某 Rack 的乐器链（可变；越界返回 `None`）。
    pub fn rack_instruments_mut(&mut self, rack: RackId) -> Option<&mut Vec<InsertRef>> {
        self.racks.get_mut(rack.index()).map(|r| &mut r.instruments)
    }

    /// 按稳定 ID 查任意混音节点。
    pub fn node(&self, id: MixerNodeId) -> Option<&MixerNode> {
        self.channel_node(id.c_channel_id())
    }

    /// 按稳定 ID 查任意混音节点（可变）。
    pub fn node_mut(&mut self, id: MixerNodeId) -> Option<&mut MixerNode> {
        self.channel_node_mut(id.c_channel_id())
    }

    /// 某源 MIDI 通道的 strip 参数（越界给默认值，防御性）。
    pub fn strip(&self, channel: u8) -> StripParams {
        self.midi_node(channel)
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 音频通道 `ch` 的 strip 参数（越界给默认值，防御性）。
    pub fn audio_strip(&self, ch: u16) -> StripParams {
        self.audio_node(ch)
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 统一节点读取：MIDI / 音频 / 每轨 LC / Master 走同一入口。
    pub fn node_strip(&self, node: MixerChannelId) -> StripParams {
        self.channel_node(self.channel_id_for(node))
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 统一节点写入。返回是否找到了目标；音频通道会按需扩容。
    pub fn set_node_strip(&mut self, node: MixerChannelId, params: StripParams) -> bool {
        if node == MixerChannelId::Master {
            self.master.strip = params;
            return true;
        }
        match node {
            MixerChannelId::Midi(ch) => {
                self.ensure_lc_node(ChannelSource::Midi(ch));
            }
            MixerChannelId::Audio(ch) => {
                self.ensure_audio_len(ch as usize + 1);
            }
            _ => {}
        }
        match self.find_node_mut(self.channel_id_for(node)) {
            Some(slot) => {
                slot.strip = params;
                true
            }
            None => false,
        }
    }

    /// 确保某音频通道号对应的 LC 节点存在（迁移/外部构造 audio LC 用）。
    pub fn ensure_audio_lc_node(&mut self, ch: u16) {
        self.ensure_audio_len(ch as usize + 1);
    }

    /// 统一节点 insert 链（可变）。
    pub fn node_inserts_mut(&mut self, node: MixerChannelId) -> Option<&mut Vec<InsertRef>> {
        if node == MixerChannelId::Master {
            return Some(&mut self.master.inserts);
        }
        self.node_mut_for_ui(node).map(|n| &mut n.inserts)
    }

    /// 统一节点主输出（MIDI / 音频 / 每轨 LC；Master 没有）。
    pub fn node_output(&self, node: MixerChannelId) -> Option<ChannelId> {
        if node == MixerChannelId::Master {
            return None;
        }
        self.channel_node(self.channel_id_for(node))
            .and_then(|n| n.output)
    }

    /// 统一节点主输出写入。返回是否支持该节点。
    pub fn set_node_output(&mut self, node: MixerChannelId, output: Option<ChannelId>) -> bool {
        if node == MixerChannelId::Master {
            return false;
        }
        let source = self.channel_id_for(node);
        if let Some(target) = output
            && (target == source || target == ChannelId::Master)
        {
            return false;
        }
        if let Some(target) = output
            && !self.channel_target_exists(target)
        {
            return false;
        }
        match self.node_mut_for_ui(node) {
            Some(slot) => {
                slot.output = output;
                true
            }
            None => false,
        }
    }

    /// 把音频通道相关表补齐到 `count` 条。
    pub fn ensure_channel_tables(&mut self, audio_count: usize) {
        self.ensure_audio_len(audio_count);
    }
}

#[cfg(test)]
#[path = "params/tests.rs"]
mod tests;
