//! 混音台持久化参数（serde，进工程文件）。

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use yinhe_types::{
    ApGroupId, BusId, LogicalChannelId, MixerChannelId, MixerNodeId, RackId, TRACK_LC_BASE,
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

/// 发送参数：源通道 → 总线（bus / return）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendParams {
    /// 目标总线的稳定 ID（不是 `MixerParams::buses` 下标）。
    pub bus: BusId,
    /// 发送量（线性增益；0.0 = 关闭）。
    #[serde(default)]
    pub amount: f32,
    /// 发送点：true = 推子前（insert 后、fader 前，不受增益/声像影响），
    /// false = 推子后。
    #[serde(default)]
    pub pre_fader: bool,
}

impl Default for SendParams {
    fn default() -> Self {
        Self {
            bus: BusId::default(),
            amount: 0.0,
            pre_fader: false,
        }
    }
}

/// 整个混音台的持久化参数。
///
/// 存储已从 v9 的并行分表合并为节点表：
/// - `racks[i]`：Rack（共享乐器），当前按源 MIDI 通道 0..256 一一对应；
/// - `channels[i]`：MIDI 源通道 i 的 LC；
/// - `audio_channels[i]`：音频通道 i 的 LC；
/// - `buses`：Bus 节点，ID 稳定；
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
    /// MIDI 源通道 i 的 LC（固定 CHANNEL_COUNT 长度，索引 = A01..P16）。
    #[serde(default)]
    pub channels: Vec<MixerNode>,
    /// 每轨独立 LC（`TrackData.lc` 指向这里；稀疏表，按 ID 查找）。
    #[serde(default)]
    pub track_channels: Vec<MixerNode>,
    /// 音频通道 i 的 LC（索引 = `TrackData::audio_channel`）。
    #[serde(default)]
    pub audio_channels: Vec<MixerNode>,
    /// 总线 / Return 节点。
    #[serde(default)]
    pub buses: Vec<MixerNode>,
    /// 主输出节点。
    #[serde(default = "MixerNode::master")]
    pub master: MixerNode,
    /// 下一个乐器实例 uid（工程内唯一、持久化；加载时对齐到已用最大值 +1）。
    #[serde(default = "default_instrument_uid")]
    pub next_instrument_uid: u64,
    /// 下一个可分配的 BusId（持久化，避免删除后复用导致自动化串线）。
    #[serde(default)]
    pub next_bus_id: BusId,
    /// 下一个可分配的每轨 LC ID。
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
            channels: (0..CHANNEL_COUNT)
                .map(|i| MixerNode::logical_channel(LogicalChannelId(i as u32)))
                .collect(),
            track_channels: Vec::new(),
            audio_channels: Vec::new(),
            buses: Vec::new(),
            master: MixerNode::master(),
            next_instrument_uid: 1,
            next_bus_id: BusId::default(),
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
        }
    }
}

impl MixerParams {
    /// 反序列化后调用：老版本工程可能缺字段/长度不足，补齐到固定通道数。
    pub fn ensure_len(&mut self) {
        self.ensure_racks();
        self.ensure_rack_outputs();
        self.ensure_channels();
        self.ensure_aps();
        self.ensure_track_channels();
        self.ensure_audio_channels();
        self.ensure_buses();
        self.ensure_master();
        self.ensure_instrument_uids();
        self.sanitize_routing();
        self.sanitize_automation();
    }

    fn sanitize_automation(&mut self) {
        for node in self
            .channels
            .iter_mut()
            .chain(self.audio_channels.iter_mut())
            .chain(self.track_channels.iter_mut())
            .chain(self.buses.iter_mut())
            .chain(std::iter::once(&mut self.master))
        {
            let Some(channel) = node.id.channel_id() else {
                continue;
            };
            node.automation.retain(|lane| match &lane.target {
                yinhe_types::AutomationTarget::MixerGain { channel: c }
                | yinhe_types::AutomationTarget::MixerPan { channel: c }
                | yinhe_types::AutomationTarget::MixerSend { node: c, .. }
                | yinhe_types::AutomationTarget::InsertParam { target: c, .. } => *c == channel,
                yinhe_types::AutomationTarget::BusGain { .. } => {
                    node.id
                        == yinhe_types::MixerNodeId::Bus(match channel {
                            MixerChannelId::Bus(bus) => bus,
                            _ => return false,
                        })
                }
                _ => true,
            });
        }
    }

    fn ensure_racks(&mut self) {
        self.racks.truncate(CHANNEL_COUNT);
        for i in self.racks.len()..CHANNEL_COUNT {
            self.racks.push(RackParams::new(RackId(i as u16)));
        }
        // 当前 Rack 与源 MIDI 通道一一对应；目标 B 允许多条 Track 指向同一
        // Rack，但 RackId 仍保持 0..256 的稳定槽位。
        for (i, rack) in self.racks.iter_mut().enumerate() {
            rack.id = RackId(i as u16);
        }
    }

    /// 清理 AP 表：删除指向不存在 Rack/LC 的 AP，保证 ID 唯一。
    ///
    /// AP 的创建/删除由 `Document` 按 Track 的 `lc` 状态驱动；这里只做
    /// 反序列化后的合法性修复。
    fn ensure_aps(&mut self) {
        let keep: Vec<ApGroup> = self
            .aps
            .iter()
            .filter(|ap| {
                ap.id.index() < CHANNEL_COUNT
                    && ap.rack.index() < self.racks.len()
                    && self.channel_exists(ap.channel)
                    && ap.midi.0 < 16
                    && ap.midi.1 < 16
            })
            .cloned()
            .collect();
        self.aps = keep;
        self.aps.sort_by_key(|ap| ap.id);
        self.aps.dedup_by_key(|ap| ap.id);
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
        let channel = LogicalChannelId(u32::from(global));
        if !self.channel_exists(channel) {
            return false;
        }
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

    fn ensure_rack_outputs(&mut self) {
        for i in 0..self.racks.len() {
            let count = self.racks[i].outputs.outputs.len();
            if count <= 1 {
                self.racks[i].output_lcs.clear();
                if self.racks[i].output_mode == yinhe_types::RackOutputMode::PerOutput {
                    self.racks[i].output_mode = yinhe_types::RackOutputMode::Shared;
                }
                continue;
            }
            let mut lcs = Vec::with_capacity(count);
            for id in std::mem::take(&mut self.racks[i].output_lcs) {
                if lcs.len() >= count {
                    break;
                }
                if id.0 >= TRACK_LC_BASE && self.track_node(id).is_some() {
                    lcs.push(id);
                } else {
                    lcs.push(self.add_track_lc());
                }
            }
            while lcs.len() < count {
                lcs.push(self.add_track_lc());
            }
            self.racks[i].output_lcs = lcs;
            self.racks[i].output_mode = yinhe_types::RackOutputMode::PerOutput;
        }
    }

    fn ensure_channels(&mut self) {
        self.channels.truncate(CHANNEL_COUNT);
        for i in self.channels.len()..CHANNEL_COUNT {
            self.channels
                .push(MixerNode::logical_channel(LogicalChannelId(i as u32)));
        }
        for (i, node) in self.channels.iter_mut().enumerate() {
            node.id = MixerNodeId::LogicalChannel(LogicalChannelId(i as u32));
        }
    }

    fn ensure_track_channels(&mut self) {
        self.track_channels.retain(
            |node| matches!(node.id, MixerNodeId::LogicalChannel(id) if id.0 >= TRACK_LC_BASE),
        );
        let mut next = self.next_lc_id.0.max(TRACK_LC_BASE);
        let mut seen: HashSet<LogicalChannelId> = HashSet::with_capacity(self.track_channels.len());
        for node in &mut self.track_channels {
            let mut id = match node.id {
                MixerNodeId::LogicalChannel(id) => id,
                _ => LogicalChannelId(next),
            };
            if id.0 < TRACK_LC_BASE || !seen.insert(id) {
                id = LogicalChannelId(next);
                seen.insert(id);
            }
            next = next.max(id.0.saturating_add(1));
            node.id = MixerNodeId::LogicalChannel(id);
        }
        self.next_lc_id = LogicalChannelId(next);
    }

    /// 分配一个每轨 LC ID，并创建对应节点。
    pub fn add_track_lc(&mut self) -> LogicalChannelId {
        self.ensure_track_channels();
        let id = self.next_lc_id;
        self.next_lc_id = LogicalChannelId(self.next_lc_id.0.saturating_add(1));
        self.track_channels.push(MixerNode::logical_channel(id));
        id
    }

    /// 用指定 ID 创建每轨 LC（加载修复用；ID 已存在则不重复创建）。
    pub fn add_track_lc_with_id(&mut self, id: LogicalChannelId) -> bool {
        if id.0 < TRACK_LC_BASE || self.track_node(id).is_some() {
            return false;
        }
        self.track_channels.push(MixerNode::logical_channel(id));
        self.next_lc_id = LogicalChannelId(self.next_lc_id.0.max(id.0.saturating_add(1)));
        true
    }

    /// 按 ID 查每轨 LC。
    /// LC 是否存在于任一通道表（MIDI / 音频 / 每轨）。
    pub fn channel_exists(&self, id: LogicalChannelId) -> bool {
        let target = MixerNodeId::LogicalChannel(id);
        self.channels.iter().any(|node| node.id == target)
            || self.audio_channels.iter().any(|node| node.id == target)
            || self.track_channels.iter().any(|node| node.id == target)
    }

    pub fn track_node(&self, id: LogicalChannelId) -> Option<&MixerNode> {
        self.track_channels
            .iter()
            .find(|node| node.id == MixerNodeId::LogicalChannel(id))
    }

    /// 按 ID 查每轨 LC（可变）。
    pub fn track_node_mut(&mut self, id: LogicalChannelId) -> Option<&mut MixerNode> {
        self.track_channels
            .iter_mut()
            .find(|node| node.id == MixerNodeId::LogicalChannel(id))
    }

    /// 删除每轨 LC，并清理所有指向它的路由（当前没有外部引用，保留接口）。
    pub fn remove_track_lc(&mut self, id: LogicalChannelId) {
        self.track_channels
            .retain(|node| node.id != MixerNodeId::LogicalChannel(id));
    }

    fn ensure_audio_channels(&mut self) {
        for (i, node) in self.audio_channels.iter_mut().enumerate() {
            node.id =
                MixerNodeId::LogicalChannel(LogicalChannelId(CHANNEL_COUNT as u32 + i as u32));
        }
    }

    fn ensure_buses(&mut self) {
        let mut next = self.next_bus_id;
        let mut seen: HashSet<BusId> = HashSet::with_capacity(self.buses.len());
        for node in &mut self.buses {
            let mut id = match node.id {
                MixerNodeId::Bus(id) => id,
                _ => alloc_bus_id(&mut next),
            };
            if !seen.insert(id) {
                id = alloc_bus_id(&mut next);
                seen.insert(id);
            }
            if id.0 >= next.0 {
                next = BusId(id.0.saturating_add(1));
            }
            node.id = MixerNodeId::Bus(id);
        }
        self.next_bus_id = next;
    }

    fn ensure_master(&mut self) {
        self.master.id = MixerNodeId::Master;
    }

    fn sanitize_routing(&mut self) {
        let known: HashSet<BusId> = self
            .buses
            .iter()
            .filter_map(|node| match node.id {
                MixerNodeId::Bus(id) => Some(id),
                _ => None,
            })
            .collect();
        for node in self
            .channels
            .iter_mut()
            .chain(self.audio_channels.iter_mut())
            .chain(self.buses.iter_mut())
        {
            node.sends.retain(|s| known.contains(&s.bus));
            if node.output.is_some_and(|id| !known.contains(&id)) {
                node.output = None;
            }
        }
        // Bus 主输出不能成环；环上的输出清成 Master。
        let bus_ids: Vec<BusId> = self.bus_ids();
        for bus in &bus_ids {
            let mut cur = self.bus_index(*bus).and_then(|i| self.buses[i].output);
            let mut steps = 0;
            let mut cycle = false;
            while let Some(id) = cur {
                if id == *bus {
                    cycle = true;
                    break;
                }
                if steps > self.buses.len() {
                    cycle = true;
                    break;
                }
                steps += 1;
                cur = self.bus_index(id).and_then(|i| self.buses[i].output);
            }
            if cycle && let Some(idx) = self.bus_index(*bus) {
                self.buses[idx].output = None;
            }
        }
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

    /// 总线数量。
    pub fn bus_count(&self) -> usize {
        self.buses.len()
    }

    /// 第 `index` 条总线的稳定 ID（越界返回 `None`）。
    pub fn bus_id_at(&self, index: usize) -> Option<BusId> {
        self.buses.get(index).and_then(|node| match node.id {
            MixerNodeId::Bus(id) => Some(id),
            _ => None,
        })
    }

    /// 稳定 BusId → `buses` 下标（不存在返回 `None`）。
    pub fn bus_index(&self, id: BusId) -> Option<usize> {
        self.buses
            .iter()
            .position(|node| node.id == MixerNodeId::Bus(id))
    }

    /// 某条总线的 strip 参数（按稳定 ID，越界给默认值）。
    pub fn bus_strip(&self, id: BusId) -> StripParams {
        self.bus_index(id)
            .and_then(|i| self.buses.get(i))
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 总线 ID 列表（按 `buses` 顺序）。
    pub fn bus_ids(&self) -> Vec<BusId> {
        self.buses
            .iter()
            .filter_map(|node| match node.id {
                MixerNodeId::Bus(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// 总线 strip 参数快照（按 `buses` 顺序，给混音图/UI 批量同步用）。
    pub fn bus_strips(&self) -> Vec<StripParams> {
        self.buses.iter().map(|node| node.strip).collect()
    }

    /// MIDI 源通道发送表快照（按源通道索引）。
    pub fn midi_sends_snapshot(&self) -> Vec<Vec<SendParams>> {
        self.channels
            .iter()
            .map(|node| node.sends.clone())
            .collect()
    }

    /// 音频通道发送表快照（按音频通道索引）。
    pub fn audio_sends_snapshot(&self) -> Vec<Vec<SendParams>> {
        self.audio_channels
            .iter()
            .map(|node| node.sends.clone())
            .collect()
    }

    /// 总线发送表快照（按 `buses` 顺序）。
    pub fn bus_sends_snapshot(&self) -> Vec<Vec<SendParams>> {
        self.buses.iter().map(|node| node.sends.clone()).collect()
    }

    fn alloc_bus_id(&mut self) -> BusId {
        alloc_bus_id(&mut self.next_bus_id)
    }

    /// 新增一条总线，返回其稳定 ID；到达 `u16` 上限时返回 `None`。
    pub fn add_bus(&mut self) -> Option<BusId> {
        // ID 空间 0..=u16::MAX，共 65_536 条；不静默回绕/复用。
        if self.buses.len() > usize::from(u16::MAX) {
            return None;
        }
        self.ensure_buses();
        let id = self.alloc_bus_id();
        self.buses.push(MixerNode::bus(id));
        Some(id)
    }

    /// 删除总线 `id`：移除节点，清理指向它的 send 和主输出。
    ///
    /// BusId 稳定，删除后不需要重排其他总线。
    pub fn remove_bus(&mut self, id: BusId) {
        let Some(idx) = self.bus_index(id) else {
            return;
        };
        self.buses.remove(idx);
        for node in self
            .channels
            .iter_mut()
            .chain(self.audio_channels.iter_mut())
            .chain(self.buses.iter_mut())
        {
            node.sends.retain(|s| s.bus != id);
        }
        for node in self
            .channels
            .iter_mut()
            .chain(self.audio_channels.iter_mut())
            .chain(self.buses.iter_mut())
        {
            if node.output == Some(id) {
                node.output = None;
            }
        }
    }

    /// 去掉 revision 已不匹配的冻结（只用于推给音频引擎的副本）。
    ///
    /// 持久化层保留失效冻结，UI 可以显示“已失效”并允许重新冻结；
    /// 引擎拿到的副本不含失效冻结，播放回退到实时输出。
    pub fn strip_stale_freezes(&mut self, revision: u64) {
        for node in self
            .channels
            .iter_mut()
            .chain(self.audio_channels.iter_mut())
            .chain(self.track_channels.iter_mut())
            .chain(self.buses.iter_mut())
            .chain(std::iter::once(&mut self.master))
        {
            let stale = node
                .freeze
                .as_ref()
                .is_some_and(|freeze| freeze.source_revision != revision);
            if stale {
                node.freeze = None;
            }
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
        match id {
            MixerNodeId::LogicalChannel(lc) => lc_match(self.channels.iter(), lc)
                .or_else(|| lc_match(self.audio_channels.iter(), lc))
                .or_else(|| self.track_node(lc)),
            MixerNodeId::Bus(bus) => self.bus_index(bus).and_then(|idx| self.buses.get(idx)),
            MixerNodeId::Master => Some(&self.master),
        }
    }

    /// 按稳定 ID 查任意混音节点（可变）。
    pub fn node_mut(&mut self, id: MixerNodeId) -> Option<&mut MixerNode> {
        match id {
            MixerNodeId::LogicalChannel(lc) => {
                if let Some(idx) = self
                    .channels
                    .iter()
                    .position(|node| node.id == MixerNodeId::LogicalChannel(lc))
                {
                    return self.channels.get_mut(idx);
                }
                if let Some(idx) = self
                    .audio_channels
                    .iter()
                    .position(|node| node.id == MixerNodeId::LogicalChannel(lc))
                {
                    return self.audio_channels.get_mut(idx);
                }
                self.track_node_mut(lc)
            }
            MixerNodeId::Bus(bus) => self.bus_index(bus).and_then(|idx| self.buses.get_mut(idx)),
            MixerNodeId::Master => Some(&mut self.master),
        }
    }

    /// 某源 MIDI 通道的 strip 参数（越界给默认值，防御性）。
    pub fn strip(&self, channel: u8) -> StripParams {
        self.channels
            .get(channel as usize)
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 音频通道 `ch` 的 strip 参数（越界给默认值，防御性）。
    pub fn audio_strip(&self, ch: u16) -> StripParams {
        self.audio_channels
            .get(ch as usize)
            .map(|node| node.strip)
            .unwrap_or_default()
    }

    /// 统一节点读取：MIDI / 音频 / Bus / Master 走同一入口。
    pub fn node_strip(&self, node: MixerChannelId) -> StripParams {
        match node {
            MixerChannelId::Midi(ch) => self.strip(ch),
            MixerChannelId::Audio(ch) => self.audio_strip(ch),
            MixerChannelId::TrackLc(id) => self
                .track_node(id)
                .map(|node| node.strip)
                .unwrap_or_default(),
            MixerChannelId::Bus(bus) => self.bus_strip(bus),
            MixerChannelId::Master => self.master.strip,
        }
    }

    /// 统一节点写入。返回是否找到了目标；音频通道会按需扩容。
    pub fn set_node_strip(&mut self, node: MixerChannelId, params: StripParams) -> bool {
        match node {
            MixerChannelId::Midi(ch) => match self.channels.get_mut(ch as usize) {
                Some(slot) => {
                    slot.strip = params;
                    true
                }
                None => false,
            },
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                self.ensure_audio_len(idx + 1);
                self.audio_channels[idx].strip = params;
                true
            }
            MixerChannelId::TrackLc(id) => match self.track_node_mut(id) {
                Some(node) => {
                    node.strip = params;
                    true
                }
                None => false,
            },
            MixerChannelId::Bus(bus) => match self.bus_index(bus) {
                Some(index) => {
                    self.buses[index].strip = params;
                    true
                }
                None => false,
            },
            MixerChannelId::Master => {
                self.master.strip = params;
                true
            }
        }
    }

    fn ensure_audio_len(&mut self, len: usize) {
        if self.audio_channels.len() >= len {
            return;
        }
        let start = self.audio_channels.len();
        for i in start..len {
            self.audio_channels
                .push(MixerNode::logical_channel(LogicalChannelId(
                    CHANNEL_COUNT as u32 + i as u32,
                )));
        }
    }

    /// 统一节点 insert 链（可变）。目标 B 的 LC 就是当前通道条。
    pub fn node_inserts_mut(&mut self, node: MixerChannelId) -> Option<&mut Vec<InsertRef>> {
        match node {
            MixerChannelId::Midi(ch) => self.channels.get_mut(ch as usize).map(|n| &mut n.inserts),
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                self.ensure_audio_len(idx + 1);
                self.audio_channels.get_mut(idx).map(|n| &mut n.inserts)
            }
            MixerChannelId::TrackLc(id) => self.track_node_mut(id).map(|n| &mut n.inserts),
            MixerChannelId::Bus(bus) => {
                let index = self.bus_index(bus)?;
                self.buses.get_mut(index).map(|n| &mut n.inserts)
            }
            MixerChannelId::Master => Some(&mut self.master.inserts),
        }
    }

    /// 统一节点发送列表。
    pub fn node_sends(&self, node: MixerChannelId) -> Option<&[SendParams]> {
        match node {
            MixerChannelId::Midi(ch) => self.channels.get(ch as usize).map(|n| n.sends.as_slice()),
            MixerChannelId::Audio(ch) => self
                .audio_channels
                .get(ch as usize)
                .map(|n| n.sends.as_slice()),
            MixerChannelId::TrackLc(id) => self.track_node(id).map(|n| n.sends.as_slice()),
            MixerChannelId::Bus(bus) => {
                let index = self.bus_index(bus)?;
                self.buses.get(index).map(|n| n.sends.as_slice())
            }
            MixerChannelId::Master => None,
        }
    }

    /// 统一节点发送列表（可变；用于就地改发送量）。
    pub fn node_sends_mut(&mut self, node: MixerChannelId) -> Option<&mut Vec<SendParams>> {
        match node {
            MixerChannelId::Midi(ch) => self.channels.get_mut(ch as usize).map(|n| &mut n.sends),
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                self.ensure_audio_len(idx + 1);
                self.audio_channels.get_mut(idx).map(|n| &mut n.sends)
            }
            MixerChannelId::TrackLc(id) => self.track_node_mut(id).map(|n| &mut n.sends),
            MixerChannelId::Bus(bus) => {
                let index = self.bus_index(bus)?;
                self.buses.get_mut(index).map(|n| &mut n.sends)
            }
            MixerChannelId::Master => None,
        }
    }

    /// 统一节点发送列表写入。
    pub fn set_node_sends(&mut self, node: MixerChannelId, sends: Vec<SendParams>) -> bool {
        match node {
            MixerChannelId::Midi(ch) => match self.channels.get_mut(ch as usize) {
                Some(slot) => {
                    slot.sends = sends;
                    true
                }
                None => false,
            },
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                self.ensure_audio_len(idx + 1);
                self.audio_channels[idx].sends = sends;
                true
            }
            MixerChannelId::TrackLc(id) => match self.track_node_mut(id) {
                Some(node) => {
                    node.sends = sends;
                    true
                }
                None => false,
            },
            MixerChannelId::Bus(bus) => match self.bus_index(bus) {
                Some(index) => {
                    self.buses[index].sends = sends;
                    true
                }
                None => false,
            },
            MixerChannelId::Master => false,
        }
    }

    /// 统一节点主输出（MIDI / 音频 / 每轨 LC / Bus；Master 没有）。
    pub fn node_output(&self, node: MixerChannelId) -> Option<BusId> {
        match node {
            MixerChannelId::Midi(ch) => self.channels.get(ch as usize).and_then(|n| n.output),
            MixerChannelId::Audio(ch) => {
                self.audio_channels.get(ch as usize).and_then(|n| n.output)
            }
            MixerChannelId::TrackLc(id) => self.track_node(id).and_then(|n| n.output),
            MixerChannelId::Bus(bus) => self
                .bus_index(bus)
                .and_then(|idx| self.buses.get(idx))
                .and_then(|n| n.output),
            MixerChannelId::Master => None,
        }
    }

    /// 统一节点主输出写入。返回是否支持该节点。
    pub fn set_node_output(&mut self, node: MixerChannelId, output: Option<BusId>) -> bool {
        match node {
            MixerChannelId::Midi(ch) => match self.channels.get_mut(ch as usize) {
                Some(slot) => {
                    slot.output = output;
                    true
                }
                None => false,
            },
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                self.ensure_audio_len(idx + 1);
                self.audio_channels[idx].output = output;
                true
            }
            MixerChannelId::TrackLc(id) => match self.track_node_mut(id) {
                Some(node) => {
                    node.output = output;
                    true
                }
                None => false,
            },
            MixerChannelId::Bus(bus) => {
                let Some(target) = output else {
                    if let Some(idx) = self.bus_index(bus) {
                        self.buses[idx].output = None;
                        return true;
                    }
                    return false;
                };
                if !self.can_route_bus_to(bus, target) {
                    return false;
                }
                match self.bus_index(bus) {
                    Some(idx) => {
                        self.buses[idx].output = Some(target);
                        true
                    }
                    None => false,
                }
            }
            MixerChannelId::Master => false,
        }
    }

    /// 该 Bus 是否可以路由到目标 Bus（存在、非自身、不成环）。
    pub fn can_route_bus_to(&self, bus: BusId, target: BusId) -> bool {
        self.bus_index(target).is_some()
            && target != bus
            && !self.bus_output_would_cycle(bus, target)
    }

    /// Bus 主输出改成 `target` 后是否形成环。
    fn bus_output_would_cycle(&self, bus: BusId, target: BusId) -> bool {
        let mut cur = Some(target);
        let mut steps = 0;
        while let Some(id) = cur {
            if id == bus {
                return true;
            }
            if steps > self.buses.len() {
                return true;
            }
            steps += 1;
            cur = self.bus_index(id).and_then(|idx| self.buses[idx].output);
        }
        false
    }

    /// 把音频通道相关表补齐到 `count` 条（通道数变化时调用，只增不减：
    /// 通道号是稳定索引，删轨道后重新加同号通道要恢复原设置）。
    pub fn ensure_channel_tables(&mut self, audio_count: usize) {
        self.ensure_audio_len(audio_count);
    }
}

fn lc_match<'a>(
    nodes: impl Iterator<Item = &'a MixerNode>,
    lc: LogicalChannelId,
) -> Option<&'a MixerNode> {
    nodes
        .into_iter()
        .find(|node| node.id == MixerNodeId::LogicalChannel(lc))
}

fn alloc_bus_id(next: &mut BusId) -> BusId {
    let id = *next;
    *next = BusId(next.0.saturating_add(1));
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_buses(p: &mut MixerParams, n: usize) -> Vec<BusId> {
        (0..n).filter_map(|_| p.add_bus()).collect()
    }

    fn send(bus: BusId, amount: f32, pre_fader: bool) -> SendParams {
        SendParams {
            bus,
            amount,
            pre_fader,
        }
    }

    #[test]
    fn default_has_256_racks_and_channels() {
        let p = MixerParams::default();
        assert_eq!(p.racks.len(), CHANNEL_COUNT);
        assert_eq!(p.channels.len(), CHANNEL_COUNT);
        assert_eq!(
            p.channels[9].id,
            MixerNodeId::LogicalChannel(LogicalChannelId(9))
        );
        assert_eq!(p.racks[9].id, RackId(9));
        assert_eq!(p.master.id, MixerNodeId::Master);
    }

    #[test]
    fn ensure_len_backfills_tables_and_fixes_ids() {
        let mut p = MixerParams::default();
        p.racks.clear();
        p.channels.clear();
        p.audio_channels = vec![MixerNode::logical_channel(LogicalChannelId(999))];
        p.buses = vec![MixerNode::bus(BusId(5)), MixerNode::bus(BusId(5))];
        p.master = MixerNode::bus(BusId(0));
        p.ensure_len();

        assert_eq!(p.racks.len(), CHANNEL_COUNT);
        assert_eq!(p.channels.len(), CHANNEL_COUNT);
        assert_eq!(
            p.audio_channels[0].id,
            MixerNodeId::LogicalChannel(LogicalChannelId(CHANNEL_COUNT as u32))
        );
        assert_ne!(p.buses[0].id, p.buses[1].id, "重复 BusId 必须修复");
        assert_eq!(p.master.id, MixerNodeId::Master);
    }

    /// 删除一条总线不改变其他总线的 ID；指向它的 send/输出被清理。
    #[test]
    fn remove_bus_keeps_other_ids() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 3);
        p.set_node_sends(
            MixerChannelId::Midi(0),
            vec![send(ids[1], 0.5, false), send(ids[2], 0.5, true)],
        );
        p.set_node_output(MixerChannelId::Midi(0), Some(ids[2]));

        p.remove_bus(ids[1]);

        assert_eq!(p.bus_ids(), vec![ids[0], ids[2]]);
        let sends = p.node_sends(MixerChannelId::Midi(0)).expect("sends");
        assert_eq!(sends.len(), 1, "指向被删总线的 send 清理");
        assert_eq!(sends[0].bus, ids[2], "其他总线 ID 不前移");
        assert_eq!(
            p.node_output(MixerChannelId::Midi(0)),
            Some(ids[2]),
            "主输出保持原 ID"
        );
    }

    #[test]
    fn remove_bus_drops_sends_to_it() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 2);
        p.set_node_sends(MixerChannelId::Midi(3), vec![send(ids[0], 1.0, false)]);
        p.remove_bus(ids[0]);
        assert!(
            p.node_sends(MixerChannelId::Midi(3))
                .expect("sends")
                .is_empty(),
            "指向被删 bus 的 send 应清理"
        );
    }

    /// 删除后新增总线不复用旧 ID，避免自动化/发送串到新总线。
    #[test]
    fn new_bus_does_not_reuse_deleted_id() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 3);
        p.remove_bus(ids[1]);
        let new_id = p.add_bus().expect("add bus");
        assert_ne!(new_id, ids[1]);
        assert_eq!(p.bus_ids(), vec![ids[0], ids[2], new_id]);
    }

    /// 总线发送表随增删总线维护；音频通道发送表同步清理。
    #[test]
    fn bus_sends_follow_bus_add_remove() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 3);
        p.set_node_sends(MixerChannelId::Bus(ids[0]), vec![send(ids[2], 1.0, false)]);
        p.set_node_sends(MixerChannelId::Bus(ids[2]), vec![send(ids[0], 1.0, false)]);
        p.set_node_sends(MixerChannelId::Audio(0), vec![send(ids[2], 1.0, false)]);

        p.remove_bus(ids[1]);
        assert_eq!(p.bus_count(), 2);
        assert_eq!(
            p.node_sends(MixerChannelId::Bus(ids[0]))
                .expect("bus sends"),
            &[send(ids[2], 1.0, false)]
        );
        assert_eq!(
            p.node_sends(MixerChannelId::Audio(0)).expect("audio sends"),
            &[send(ids[2], 1.0, false)]
        );
    }

    /// 自发送随其他总线删除保持指向自己。
    #[test]
    fn bus_self_send_survives_other_bus_removal() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 2);
        p.set_node_sends(MixerChannelId::Bus(ids[1]), vec![send(ids[1], 1.0, false)]);
        p.remove_bus(ids[0]);
        assert_eq!(
            p.node_sends(MixerChannelId::Bus(ids[1]))
                .expect("bus sends"),
            &[send(ids[1], 1.0, false)]
        );
    }

    /// ensure_len 清理三张发送表里指向不存在总线的条目。
    #[test]
    fn ensure_len_prunes_dangling_routing() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let _ = p.add_bus();
        p.set_node_sends(MixerChannelId::Midi(0), vec![send(BusId(5), 1.0, false)]);
        p.set_node_sends(MixerChannelId::Audio(0), vec![send(BusId(5), 1.0, false)]);
        p.set_node_output(MixerChannelId::Midi(0), Some(BusId(5)));
        p.ensure_len();
        assert!(p.node_sends(MixerChannelId::Midi(0)).unwrap().is_empty());
        assert!(p.node_sends(MixerChannelId::Audio(0)).unwrap().is_empty());
        assert_eq!(p.node_output(MixerChannelId::Midi(0)), None);
    }

    #[test]
    fn ap_groups_follow_midi_channel_and_are_unique() {
        let mut p = MixerParams::default();
        p.ensure_len();
        assert!(p.ensure_ap(3));
        let ap = p.ap_for_midi(3).expect("ap");
        assert_eq!(ap.id, ApGroupId(3));
        assert_eq!(ap.midi, (0, 3));
        assert_eq!(ap.rack, RackId(3));
        assert_eq!(ap.channel, LogicalChannelId(3));
        assert!(!p.ensure_ap(3));
        assert_eq!(p.aps.len(), 1);
        assert!(p.remove_ap_for_midi(3));
        assert!(p.ap_for_midi(3).is_none());
    }

    /// 统一节点入口：MIDI / 音频 / Bus / Master 各走各的存储，但 API 一致。
    #[test]
    fn node_strip_routes_each_namespace() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let bus = p.add_bus().expect("bus");

        for (node, gain) in [
            (MixerChannelId::Midi(3), 0.3),
            (MixerChannelId::Audio(7), 0.7),
            (MixerChannelId::Bus(bus), 0.5),
            (MixerChannelId::Master, 0.9),
        ] {
            let params = StripParams {
                gain,
                ..StripParams::default()
            };
            assert!(p.set_node_strip(node, params));
            assert_eq!(p.node_strip(node).gain, gain);
        }

        assert!(!p.set_node_strip(MixerChannelId::Bus(BusId(99)), StripParams::default()));
        assert_eq!(p.strip(3).gain, 0.3);
        assert_eq!(p.audio_strip(7).gain, 0.7);
        assert_eq!(p.bus_strip(bus).gain, 0.5);
        assert_eq!(p.master.gain, 0.9);
    }

    #[test]
    fn node_inserts_sends_and_output_route_each_namespace() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let bus = p.add_bus().expect("bus");

        assert!(p.node_inserts_mut(MixerChannelId::Midi(1)).is_some());
        assert!(p.node_inserts_mut(MixerChannelId::Audio(2)).is_some());
        assert!(p.node_inserts_mut(MixerChannelId::Bus(bus)).is_some());
        assert!(p.node_inserts_mut(MixerChannelId::Master).is_some());

        let send = send(bus, 0.5, false);
        assert!(p.set_node_sends(MixerChannelId::Midi(1), vec![send]));
        assert_eq!(p.node_sends(MixerChannelId::Midi(1)), Some(&[send][..]));
        assert!(!p.set_node_sends(MixerChannelId::Master, vec![send]));

        assert!(p.set_node_output(MixerChannelId::Midi(1), Some(bus)));
        assert_eq!(p.node_output(MixerChannelId::Midi(1)), Some(bus));
        assert!(!p.set_node_output(MixerChannelId::Bus(bus), Some(bus)));
    }

    #[test]
    fn rack_instruments_are_shared_by_id() {
        let mut p = MixerParams::default();
        let rack = RackId(7);
        let instruments = p.rack_instruments_mut(rack).expect("rack");
        instruments.push(InsertRef {
            plugin_id: "test".into(),
            ..InsertRef::default()
        });
        assert_eq!(p.rack_instruments(rack).len(), 1);
        assert_eq!(p.rack_instruments(rack)[0].plugin_id, "test");
    }

    /// 重复 BusId 修复：后面的重复项重新分配，避免两条总线共享自动化。
    #[test]
    fn bus_outputs_allow_nesting_and_reject_cycles() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 2);
        assert!(p.set_node_output(MixerChannelId::Bus(ids[0]), Some(ids[1])));
        assert_eq!(p.node_output(MixerChannelId::Bus(ids[0])), Some(ids[1]));
        // 反向成环：拒绝。
        assert!(!p.set_node_output(MixerChannelId::Bus(ids[1]), Some(ids[0])));
        assert_eq!(p.node_output(MixerChannelId::Bus(ids[1])), None);
        // 自输出：拒绝。
        assert!(!p.set_node_output(MixerChannelId::Bus(ids[0]), Some(ids[0])));
    }

    #[test]
    fn duplicate_bus_ids_are_reassigned() {
        let mut p = MixerParams::default();
        p.ensure_len();
        p.buses = vec![MixerNode::bus(BusId(7)), MixerNode::bus(BusId(7))];
        p.next_bus_id = BusId(8);
        p.ensure_len();
        assert_eq!(p.bus_ids(), vec![BusId(7), BusId(8)]);
        assert_eq!(p.next_bus_id, BusId(9));
    }

    #[test]
    fn track_lcs_allocate_from_high_namespace_and_are_independent() {
        let mut p = MixerParams::default();
        let a = p.add_track_lc();
        let b = p.add_track_lc();
        assert_ne!(a, b);
        assert!(a.0 >= TRACK_LC_BASE);
        assert!(b.0 >= TRACK_LC_BASE);
        assert!(p.track_node(a).is_some());

        p.track_node_mut(a).expect("track lc").strip.gain = 0.25;
        assert_eq!(p.track_node(a).expect("track lc").gain, 0.25);
        assert_eq!(p.track_node(b).expect("track lc").gain, 1.0);

        p.remove_track_lc(a);
        assert!(p.track_node(a).is_none());
        assert!(p.track_node(b).is_some());
    }

    #[test]
    fn strip_out_of_range_gives_default() {
        let p = MixerParams {
            channels: Vec::new(),
            ..MixerParams::default()
        };
        assert_eq!(p.strip(200), StripParams::default());
    }
}
