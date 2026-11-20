//! 混音台持久化参数（serde，进工程文件）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use yinhe_types::{
    ApChannelId, ApGroupId, Audibility, ChannelId, ChannelSource, LogicalChannelId, RackId,
    TRACK_LC_BASE,
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
    /// 声像，-1.0（左）~ 1.0（右），0.0 居中。
    #[serde(default)]
    pub pan: f32,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
        }
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
    /// AP 现在是独立音频节点（`ChannelId::Ap`），编组与节点一起持久化。
    #[serde(default)]
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
    /// 7. `ensure_master_sends`：为直连 Master 的节点补默认 0 dB 发送（依赖 routing）；
    /// 8. `repair_automation`：最后按节点归属过滤 lane（依赖以上全部完成）。
    pub fn ensure_len(&mut self) {
        self.repair_racks();
        self.repair_aps();
        self.repair_channel_sends();
        self.repair_track_channels();
        self.repair_audio_channels();
        self.repair_mixed_lc_freezes();
        self.repair_master();
        self.ensure_instrument_uids();
        self.repair_routing();
        self.ensure_master_sends();
        self.repair_automation();
    }

    /// 为所有「主输出 = Master」且没有显式 Master 发送的节点补 0 dB 发送。
    ///
    /// 发送图现在是进 Master 的唯一路径：默认 0 dB 保证新通道能出声。
    /// SEND 关闭会保留 `amount = 0` 的条目（见 `set_channel_send`），
    /// 所以本函数不会把用户关掉的 SEND 又补成 0 dB。
    pub fn ensure_master_sends(&mut self) {
        let sources: Vec<ChannelId> = self
            .nodes
            .iter()
            .filter(|node| node.output.is_none())
            .map(|node| node.id)
            .collect();
        for source in sources {
            self.ensure_master_send(source);
        }
    }

    fn ensure_master_send(&mut self, source: ChannelId) {
        let exists = self
            .channel_sends
            .iter()
            .any(|send| send.source == source && send.target == ChannelId::Master);
        if !exists {
            self.channel_sends
                .push(ChannelSend::new(source, ChannelId::Master, 1.0, false));
        }
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

    /// 某 MIDI 通道对应的 AP 通道身份（没有 AP 编组返回 None）。
    pub fn ap_channel_id(&self, global: u8) -> Option<ChannelId> {
        self.ap_for_midi(global)
            .map(|ap| ChannelId::Ap(ApChannelId::new(ap.id, 0)))
    }

    /// 某 MIDI 通道的 AP 节点；没有 AP 返回 None。
    pub fn ap_node(&self, global: u8) -> Option<&MixerNode> {
        self.ap_channel_id(global).and_then(|id| self.find_node(id))
    }

    pub fn ap_node_mut(&mut self, global: u8) -> Option<&mut MixerNode> {
        let id = self.ap_channel_id(global)?;
        self.find_node_mut(id)
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

    /// 为某 MIDI 通道创建 AP 编组与独立 AP 节点（如果还不存在）。
    pub fn ensure_ap(&mut self, global: u8) -> bool {
        if self.ap_for_midi(global).is_some() {
            return false;
        }
        let Some(rack) = self.racks.get(global as usize) else {
            return false;
        };
        let id = ApGroupId(u16::from(global));
        self.aps
            .push(ApGroup::new(id, (global >> 4, global & 0x0F), rack.id));
        self.aps.sort_by_key(|ap| ap.id);
        self.ensure_ap_node(id);
        true
    }

    /// 确保 AP 编组的独立节点存在（加载/修复用）。
    pub(super) fn ensure_ap_node(&mut self, id: ApGroupId) {
        let channel = ChannelId::Ap(ApChannelId::new(id, 0));
        if self.find_node(channel).is_none() {
            self.nodes
                .push(MixerNode::new(channel, ChannelSource::Ap(id)));
            self.sort_dedup_nodes();
            self.ensure_master_send(channel);
        }
    }

    /// 删除某 MIDI 通道的 AP 编组与节点，返回是否有变化。
    pub fn remove_ap_for_midi(&mut self, global: u8) -> bool {
        let id = ApGroupId(u16::from(global));
        let ap = ChannelId::Ap(ApChannelId::new(id, 0));
        let before = self.aps.len();
        self.aps.retain(|ap| ap.global_midi() != global);
        let before_nodes = self.nodes.len();
        self.nodes.retain(|node| node.id != ap);
        self.channel_sends
            .retain(|send| send.target != ap && send.source != ap);
        self.aps.len() != before || self.nodes.len() != before_nodes
    }

    fn channel_target_exists(&self, target: ChannelId) -> bool {
        target == ChannelId::Master || self.find_node(target).is_some()
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
        let amount = amount.clamp(0.0, 1.0);
        // Master 的 0 条目是「显式关闭」：必须保留，否则默认发送会被补齐。
        if amount <= 0.0 && target != ChannelId::Master {
            return self.remove_channel_send(source, target);
        }
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
        let channel = ChannelId::Lc(id);
        self.nodes.push(MixerNode::logical_channel(id, source));
        self.sort_dedup_nodes();
        self.ensure_master_send(channel);
        id
    }

    /// 分配一个每轨 LC ID，并创建绑定到 `track` 的节点。
    ///
    /// 创建即归属（一轨一通道），不再有 Track(u16::MAX) 临时哨兵：
    /// 哨兵曾会让"未绑定的 LC"以 Track(65535) 落盘，引擎侧还要专门防御越界。
    pub fn add_track_lc(&mut self, track: u16) -> LogicalChannelId {
        self.repair_track_channels();
        let id = self.alloc_lc_id();
        let channel = ChannelId::Lc(id);
        self.nodes
            .push(MixerNode::logical_channel(id, ChannelSource::Track(track)));
        self.sort_dedup_nodes();
        self.ensure_master_send(channel);
        id
    }

    /// 加载修复：用指定 ID 与来源创建每轨 LC（ID 已存在则不重复创建）。
    pub fn add_track_lc_with_source(
        &mut self,
        id: LogicalChannelId,
        source: ChannelSource,
    ) -> bool {
        if self.find_node(ChannelId::Lc(id)).is_some() {
            return false;
        }
        self.nodes.push(MixerNode::logical_channel(id, source));
        if id.0 >= self.next_lc_id.0 {
            self.next_lc_id = LogicalChannelId(id.0.saturating_add(1));
        }
        self.sort_dedup_nodes();
        self.ensure_master_send(ChannelId::Lc(id));
        true
    }

    /// 按 ID 查每轨 LC。
    pub fn channel_exists(&self, id: LogicalChannelId) -> bool {
        self.find_node(ChannelId::Lc(id)).is_some()
    }

    /// 删除每轨 LC，并清理所有指向它的路由（当前没有外部引用，保留接口）。
    pub fn remove_track_lc(&mut self, id: LogicalChannelId) {
        let ch = ChannelId::Lc(id);
        self.nodes.retain(|node| node.id != ch);
        self.channel_sends
            .retain(|send| send.source != ch && send.target != ch);
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
    pub fn node(&self, id: ChannelId) -> Option<&MixerNode> {
        self.channel_node(id)
    }

    /// 按稳定 ID 查任意混音节点（可变）。
    pub fn node_mut(&mut self, id: ChannelId) -> Option<&mut MixerNode> {
        self.channel_node_mut(id)
    }

    /// 音频通道 `ch` 的 strip 参数（越界给默认值，防御性）。
    pub fn audio_strip(&self, ch: u16) -> StripParams {
        self.audio_node(ch)
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 统一节点读取：AP / 音频 / 每轨 LC / Master 走同一入口。
    pub fn node_strip(&self, node: ChannelId) -> StripParams {
        self.channel_node(node)
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 统一节点写入。返回是否找到了目标。
    pub fn set_node_strip(&mut self, node: ChannelId, params: StripParams) -> bool {
        if node == ChannelId::Master {
            self.master.strip = params;
            return true;
        }
        match self.channel_node_mut(node) {
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
    pub fn node_inserts_mut(&mut self, node: ChannelId) -> Option<&mut Vec<InsertRef>> {
        if node == ChannelId::Master {
            return Some(&mut self.master.inserts);
        }
        self.node_mut(node).map(|n| &mut n.inserts)
    }

    /// 统一节点主输出（AP / 音频 / 每轨 LC；Master 没有）。
    pub fn node_output(&self, node: ChannelId) -> Option<ChannelId> {
        if node == ChannelId::Master {
            return None;
        }
        self.channel_node(node).and_then(|n| n.output)
    }

    /// 统一节点主输出写入。返回是否支持该节点。
    pub fn set_node_output(&mut self, node: ChannelId, output: Option<ChannelId>) -> bool {
        if node == ChannelId::Master {
            return false;
        }
        if let Some(target) = output
            && (target == node || target == ChannelId::Master)
        {
            return false;
        }
        if let Some(target) = output
            && !self.channel_target_exists(target)
        {
            return false;
        }
        match self.node_mut(node) {
            Some(slot) => {
                if slot.output == output {
                    return false;
                }
                slot.output = output;
            }
            None => return false,
        }
        // 主输出 = Master 时唯一路径是 Master send；改为另一条通道时
        // 必须删掉 Master send，否则会同时走两条路。
        if output.is_some() {
            self.remove_channel_send(node, ChannelId::Master);
        } else {
            self.ensure_master_send(node);
        }
        true
    }

    /// 把音频通道相关表补齐到 `count` 条。
    pub fn ensure_channel_tables(&mut self, audio_count: usize) {
        self.ensure_audio_len(audio_count);
    }
}

#[cfg(test)]
#[path = "params/tests.rs"]
mod tests;
