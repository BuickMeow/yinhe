//! 混音台持久化参数（serde，进工程文件）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use yinhe_types::{BusId, MixerChannelId};

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
/// 索引语义：`channels[i]` / `channel_inserts[i]` 对应**源 MIDI 通道 i**
/// （A01 = 0，P16 = 255），不是工程轨道、也不是压缩后的 dense 索引——
/// 源通道号在音轨增删后保持稳定，dense 索引随布局重建变化。
/// 未被工程使用的通道的条目闲置无害。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerParams {
    /// 按源 MIDI 通道索引的 strip 参数，固定 CHANNEL_COUNT 长度。
    #[serde(default)]
    pub channels: Vec<StripParams>,
    #[serde(default)]
    pub master: MasterParams,
    /// 每源通道的 insert 链（固定 CHANNEL_COUNT 长度，元素为有序槽位列表）。
    #[serde(default)]
    pub channel_inserts: Vec<Vec<InsertRef>>,
    /// 主输出 insert 链。
    #[serde(default)]
    pub master_inserts: Vec<InsertRef>,
    /// 乐器插件链（索引 = 源 MIDI 通道，固定 CHANNEL_COUNT 长度）：
    /// 空 = 该通道使用默认的内置 XSynth；非空 = 按序**叠加**的插件乐器，
    /// 音符事件广播给链内每个乐器，输出相加。
    /// 混音参数（推子/声像/insert/send）统一走该 MIDI 通道的 strip 表。
    /// 旧格式（每通道 `Option<InsertRef>`）由混音段版本号处理：版本不符整段忽略。
    #[serde(default)]
    pub instruments: Vec<Vec<InsertRef>>,
    /// 下一个乐器实例 uid（工程内唯一、持久化；加载时对齐到已用最大值 +1）。
    #[serde(default = "default_instrument_uid")]
    pub next_instrument_uid: u64,
    /// 音频通道 strip（索引 = 音频通道号，与 `TrackData::audio_channel` 对齐）。
    #[serde(default)]
    pub audio_channels: Vec<StripParams>,
    /// 每音频通道的 insert 链。
    #[serde(default)]
    pub audio_inserts: Vec<Vec<InsertRef>>,
    /// 每音频通道的 send 列表。
    #[serde(default)]
    pub audio_sends: Vec<Vec<SendParams>>,
    /// 总线（bus / return）通道参数：每元素一条总线（与源通道同语义）。
    #[serde(default)]
    pub buses: Vec<StripParams>,
    /// 每条总线的稳定 ID（与 `buses` 等长，元素不随删除/重排变化）。
    /// 旧工程缺省为空，加载时按原索引 0..n 生成，行为与旧版一致。
    #[serde(default)]
    pub bus_ids: Vec<BusId>,
    /// 下一个可分配的 BusId（持久化，避免删除后复用导致自动化串线）。
    #[serde(default)]
    pub next_bus_id: BusId,
    /// 每条总线的 insert 链（与 `buses` 等长）。
    #[serde(default)]
    pub bus_inserts: Vec<Vec<InsertRef>>,
    /// 每条总线的发送列表（与 `buses` 等长；总线可发送到其他总线或自己）。
    /// 自发送（bus → 自己）会引入一个缓冲块延迟，防止反馈环路。
    #[serde(default)]
    pub bus_sends: Vec<Vec<SendParams>>,
    /// 每个源通道的发送列表（按源通道索引；一个通道可发往多条总线）。
    #[serde(default)]
    pub sends: Vec<Vec<SendParams>>,
    /// MIDI 源通道的主输出目标（索引 = 源通道，`Some(bus)` = 输出到该总线，
    /// `None` = 直接进 Master）。由通道输出选择 UI 显式写入。
    #[serde(default)]
    pub midi_outputs: Vec<Option<BusId>>,
    /// 音频通道的主输出目标（索引 = 音频通道号）。
    #[serde(default)]
    pub audio_outputs: Vec<Option<BusId>>,
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            channels: vec![StripParams::default(); CHANNEL_COUNT],
            master: MasterParams::default(),
            channel_inserts: vec![Vec::new(); CHANNEL_COUNT],
            master_inserts: Vec::new(),
            instruments: vec![Vec::new(); CHANNEL_COUNT],
            next_instrument_uid: 1,
            audio_channels: Vec::new(),
            audio_inserts: Vec::new(),
            audio_sends: Vec::new(),
            buses: Vec::new(),
            bus_ids: Vec::new(),
            next_bus_id: BusId::default(),
            bus_inserts: Vec::new(),
            bus_sends: Vec::new(),
            sends: Vec::new(),
            midi_outputs: Vec::new(),
            audio_outputs: Vec::new(),
        }
    }
}

impl MixerParams {
    /// 反序列化后调用：老版本工程可能缺字段/长度不足，补齐到固定通道数。
    pub fn ensure_len(&mut self) {
        self.channels.resize(CHANNEL_COUNT, StripParams::default());
        self.channel_inserts.resize(CHANNEL_COUNT, Vec::new());
        self.channels.truncate(CHANNEL_COUNT);
        self.channel_inserts.truncate(CHANNEL_COUNT);
        self.sends.resize(CHANNEL_COUNT, Vec::new());
        self.sends.truncate(CHANNEL_COUNT);
        self.instruments.resize(CHANNEL_COUNT, Vec::new());
        self.instruments.truncate(CHANNEL_COUNT);
        self.ensure_instrument_uids();
        self.bus_inserts.resize(self.buses.len(), Vec::new());
        self.bus_inserts.truncate(self.buses.len());
        self.bus_sends.resize(self.buses.len(), Vec::new());
        self.bus_sends.truncate(self.buses.len());
        self.ensure_bus_ids();
        self.midi_outputs.resize(CHANNEL_COUNT, None);
        self.midi_outputs.truncate(CHANNEL_COUNT);
        // 防御：清理指向不存在总线的 send / 主输出（工程被手工编辑/版本迁移残留）。
        // MIDI / 音频 / 总线三张发送表一视同仁。
        let known = self.bus_ids.clone();
        for list in &mut self.sends {
            list.retain(|s| known.contains(&s.bus));
        }
        for list in &mut self.audio_sends {
            list.retain(|s| known.contains(&s.bus));
        }
        for list in &mut self.bus_sends {
            list.retain(|s| known.contains(&s.bus));
        }
        for target in self
            .midi_outputs
            .iter_mut()
            .chain(self.audio_outputs.iter_mut())
        {
            if target.is_some_and(|id| !known.contains(&id)) {
                *target = None;
            }
        }
    }

    /// 补齐并修复总线 ID 表。
    ///
    /// - 旧工程没有 `bus_ids`：按原索引 0..n 生成，听感/路由与旧版一致；
    /// - 表比总线短：给新增总线分配新 ID；
    /// - 表比总线长：截断；
    /// - 重复 ID：后面的重复项重新分配，避免自动化/发送指向同一条总线。
    fn ensure_bus_ids(&mut self) {
        self.bus_ids.truncate(self.buses.len());
        for i in 0..self.bus_ids.len() {
            if self.bus_ids[..i].contains(&self.bus_ids[i]) {
                let id = self.alloc_bus_id();
                self.bus_ids[i] = id;
            }
        }
        while self.bus_ids.len() < self.buses.len() {
            let id = self.alloc_bus_id();
            self.bus_ids.push(id);
        }
        if let Some(max) = self.bus_ids.iter().map(|id| id.0).max() {
            self.next_bus_id = BusId(self.next_bus_id.0.max(max.saturating_add(1)));
        }
    }

    fn alloc_bus_id(&mut self) -> BusId {
        let id = self.next_bus_id;
        self.next_bus_id = BusId(self.next_bus_id.0.saturating_add(1));
        id
    }

    /// 总线数量。
    pub fn bus_count(&self) -> usize {
        self.buses.len()
    }

    /// 第 `index` 条总线的稳定 ID（越界返回 `None`）。
    pub fn bus_id_at(&self, index: usize) -> Option<BusId> {
        self.bus_ids.get(index).copied()
    }

    /// 稳定 BusId → `buses` 下标（不存在返回 `None`）。
    pub fn bus_index(&self, id: BusId) -> Option<usize> {
        self.bus_ids.iter().position(|&x| x == id)
    }

    /// 某条总线的 strip 参数（按稳定 ID，越界给默认值）。
    pub fn bus_strip(&self, id: BusId) -> StripParams {
        self.bus_index(id)
            .and_then(|i| self.buses.get(i).copied())
            .unwrap_or_default()
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
        for refs in &mut self.instruments {
            for r in refs.iter_mut() {
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

    /// 新增一条总线，返回其稳定 ID；到达 `u16` ID 上限时返回 `None`。
    pub fn add_bus(&mut self) -> Option<BusId> {
        // ID 空间 0..=u16::MAX，共 65_536 条；不静默回绕/复用。
        if self.buses.len() > usize::from(u16::MAX) {
            return None;
        }
        self.ensure_bus_ids();
        let id = self.alloc_bus_id();
        self.buses.push(StripParams::default());
        self.bus_ids.push(id);
        self.bus_inserts.push(Vec::new());
        self.bus_sends.push(Vec::new());
        Some(id)
    }

    /// 删除总线 `id`：移除其参数、insert 链与 ID，清理指向它的 send 和主输出。
    ///
    /// 因为发送/输出存的是稳定 BusId，删除后**不需要**把更高索引前移；
    /// 其他总线的路由保持原样，这是与旧索引实现最大的行为差异（也是修复点）。
    pub fn remove_bus(&mut self, id: BusId) {
        let Some(idx) = self.bus_index(id) else {
            return;
        };
        self.buses.remove(idx);
        self.bus_ids.remove(idx);
        if idx < self.bus_inserts.len() {
            self.bus_inserts.remove(idx);
        }
        if idx < self.bus_sends.len() {
            self.bus_sends.remove(idx);
        }
        for list in self
            .sends
            .iter_mut()
            .chain(self.audio_sends.iter_mut())
            .chain(self.bus_sends.iter_mut())
        {
            list.retain(|s| s.bus != id);
        }
        for target in self
            .midi_outputs
            .iter_mut()
            .chain(self.audio_outputs.iter_mut())
        {
            if *target == Some(id) {
                *target = None;
            }
        }
    }

    /// 某源通道的 strip 参数（越界给默认值，防御性）。
    pub fn strip(&self, channel: u8) -> StripParams {
        self.channels
            .get(channel as usize)
            .copied()
            .unwrap_or_default()
    }

    /// 音频通道 `ch` 的 strip 参数（越界给默认值，防御性）。
    pub fn audio_strip(&self, ch: u16) -> StripParams {
        self.audio_channels
            .get(ch as usize)
            .copied()
            .unwrap_or_default()
    }

    /// 统一节点读取：MIDI / 音频 / Bus / Master 走同一入口。
    ///
    /// Master 目前只有 gain，pan/mute/solo 返回默认值；这是过渡期的
    /// 能力差异，目标 B 会把 Master 也变成完整 `MixerNode`。
    pub fn node_strip(&self, node: MixerChannelId) -> StripParams {
        match node {
            MixerChannelId::Midi(ch) => self.strip(ch),
            MixerChannelId::Audio(ch) => self.audio_strip(ch),
            MixerChannelId::Bus(bus) => self.bus_strip(bus),
            MixerChannelId::Master => StripParams {
                gain: self.master.gain,
                ..StripParams::default()
            },
        }
    }

    /// 统一节点写入。返回是否找到了目标；音频通道会按需扩容。
    ///
    /// Master 只写 gain（pan/mute/solo 尚未建模），其余字段忽略。
    pub fn set_node_strip(&mut self, node: MixerChannelId, params: StripParams) -> bool {
        match node {
            MixerChannelId::Midi(ch) => {
                if let Some(slot) = self.channels.get_mut(ch as usize) {
                    *slot = params;
                    true
                } else {
                    false
                }
            }
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                if self.audio_channels.len() <= idx {
                    self.audio_channels.resize(idx + 1, StripParams::default());
                }
                self.audio_channels[idx] = params;
                true
            }
            MixerChannelId::Bus(bus) => match self.bus_index(bus) {
                Some(index) => {
                    self.buses[index] = params;
                    true
                }
                None => false,
            },
            MixerChannelId::Master => {
                self.master.gain = params.gain;
                true
            }
        }
    }

    /// 统一节点 insert 链（可变）。目标 B 的 LC 就是当前通道条。
    pub fn node_inserts_mut(&mut self, node: MixerChannelId) -> Option<&mut Vec<InsertRef>> {
        match node {
            MixerChannelId::Midi(ch) => self.channel_inserts.get_mut(ch as usize),
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                if self.audio_inserts.len() <= idx {
                    self.audio_inserts.resize(idx + 1, Vec::new());
                }
                self.audio_inserts.get_mut(idx)
            }
            MixerChannelId::Bus(bus) => {
                let index = self.bus_index(bus)?;
                self.bus_inserts.get_mut(index)
            }
            MixerChannelId::Master => Some(&mut self.master_inserts),
        }
    }

    /// 统一节点发送列表。
    pub fn node_sends(&self, node: MixerChannelId) -> Option<&[SendParams]> {
        match node {
            MixerChannelId::Midi(ch) => self.sends.get(ch as usize).map(Vec::as_slice),
            MixerChannelId::Audio(ch) => self.audio_sends.get(ch as usize).map(Vec::as_slice),
            MixerChannelId::Bus(bus) => {
                let index = self.bus_index(bus)?;
                self.bus_sends.get(index).map(Vec::as_slice)
            }
            MixerChannelId::Master => None,
        }
    }

    /// 统一节点发送列表写入。
    pub fn set_node_sends(&mut self, node: MixerChannelId, sends: Vec<SendParams>) -> bool {
        match node {
            MixerChannelId::Midi(ch) => match self.sends.get_mut(ch as usize) {
                Some(slot) => {
                    *slot = sends;
                    true
                }
                None => false,
            },
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                if self.audio_sends.len() <= idx {
                    self.audio_sends.resize(idx + 1, Vec::new());
                }
                self.audio_sends[idx] = sends;
                true
            }
            MixerChannelId::Bus(bus) => match self.bus_index(bus) {
                Some(index) => {
                    self.bus_sends[index] = sends;
                    true
                }
                None => false,
            },
            MixerChannelId::Master => false,
        }
    }

    /// 统一节点主输出（仅 MIDI / 音频源通道有主输出）。
    pub fn node_output(&self, node: MixerChannelId) -> Option<BusId> {
        match node {
            MixerChannelId::Midi(ch) => self.midi_outputs.get(ch as usize).copied().flatten(),
            MixerChannelId::Audio(ch) => self.audio_outputs.get(ch as usize).copied().flatten(),
            MixerChannelId::Bus(_) | MixerChannelId::Master => None,
        }
    }

    /// 统一节点主输出写入。返回是否支持该节点。
    pub fn set_node_output(&mut self, node: MixerChannelId, output: Option<BusId>) -> bool {
        match node {
            MixerChannelId::Midi(ch) => {
                let idx = ch as usize;
                if self.midi_outputs.len() <= idx {
                    self.midi_outputs.resize(idx + 1, None);
                }
                self.midi_outputs[idx] = output;
                true
            }
            MixerChannelId::Audio(ch) => {
                let idx = ch as usize;
                if self.audio_outputs.len() <= idx {
                    self.audio_outputs.resize(idx + 1, None);
                }
                self.audio_outputs[idx] = output;
                true
            }
            MixerChannelId::Bus(_) | MixerChannelId::Master => false,
        }
    }

    /// 把音频通道相关表补齐到 `count` 条（通道数变化时调用，只增不减：
    /// 通道号是稳定索引，删轨道后重新加同号通道要恢复原设置）。
    /// 参数表长度与通道号索引严格对应，缺位补默认值。
    pub fn ensure_channel_tables(&mut self, audio_count: usize) {
        if self.audio_channels.len() < audio_count {
            self.audio_channels
                .resize(audio_count, StripParams::default());
        }
        if self.audio_inserts.len() < audio_count {
            self.audio_inserts.resize(audio_count, Vec::new());
        }
        if self.audio_sends.len() < audio_count {
            self.audio_sends.resize(audio_count, Vec::new());
        }
        if self.audio_outputs.len() < audio_count {
            self.audio_outputs.resize(audio_count, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_len_pads_and_truncates() {
        let mut p = MixerParams {
            channels: vec![StripParams {
                gain: 0.5,
                ..StripParams::default()
            }],
            ..MixerParams::default()
        };
        p.channel_inserts.clear();
        p.ensure_len();
        assert_eq!(p.channels.len(), CHANNEL_COUNT);
        assert_eq!(p.channel_inserts.len(), CHANNEL_COUNT);
        assert_eq!(p.channels[0].gain, 0.5);
        assert!(p.channels[1..].iter().all(|s| *s == StripParams::default()));
    }

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

    /// 删除一条总线不改变其他总线的 ID；指向它的 send/输出被清理。
    #[test]
    fn remove_bus_keeps_other_ids() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 3);
        p.sends[0] = vec![send(ids[1], 0.5, false), send(ids[2], 0.5, true)];
        p.midi_outputs[0] = Some(ids[2]);

        p.remove_bus(ids[1]);

        assert_eq!(p.buses.len(), 2);
        assert_eq!(p.bus_ids, vec![ids[0], ids[2]]);
        assert_eq!(p.sends[0].len(), 1, "指向被删总线的 send 清理");
        assert_eq!(p.sends[0][0].bus, ids[2], "其他总线 ID 不前移");
        assert_eq!(p.midi_outputs[0], Some(ids[2]), "主输出保持原 ID");
    }

    #[test]
    fn remove_bus_drops_sends_to_it() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 2);
        p.sends[3] = vec![send(ids[0], 1.0, false)];
        p.remove_bus(ids[0]);
        assert!(p.sends[3].is_empty(), "指向被删 bus 的 send 应清理");
    }

    /// 旧工程没有 bus_ids：按原索引 0..n 生成，发送/输出保持不变。
    #[test]
    fn ensure_len_migrates_legacy_bus_indices_to_ids() {
        let mut p = MixerParams {
            buses: vec![StripParams::default(); 3],
            ..MixerParams::default()
        };
        p.ensure_len();
        // 模拟旧工程：总线在，但没有 bus_ids/next_bus_id。
        p.bus_ids.clear();
        p.next_bus_id = BusId::default();
        p.sends[0] = vec![send(BusId(1), 1.0, false)];
        p.midi_outputs[0] = Some(BusId(2));
        p.ensure_len();

        assert_eq!(p.bus_ids, vec![BusId(0), BusId(1), BusId(2)]);
        assert_eq!(p.next_bus_id, BusId(3));
        assert_eq!(p.sends[0][0].bus, BusId(1));
        assert_eq!(p.midi_outputs[0], Some(BusId(2)));
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
        assert_eq!(p.bus_ids, vec![ids[0], ids[2], new_id]);
    }

    /// 总线发送表随增删总线维护；音频通道发送表同步清理。
    #[test]
    fn bus_sends_follow_bus_add_remove() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 3);
        // bus0 → bus2；bus2 → bus0；音频通道 0 → bus2
        p.bus_sends[0].push(send(ids[2], 1.0, false));
        p.bus_sends[2].push(send(ids[0], 1.0, false));
        p.audio_sends = vec![vec![send(ids[2], 1.0, false)]];

        p.remove_bus(ids[1]);
        assert_eq!(p.bus_sends.len(), 2);
        assert_eq!(p.bus_sends[0].len(), 1, "其他总线的发送保留");
        assert_eq!(p.bus_sends[0][0].bus, ids[2], "目标 ID 不变");
        assert_eq!(p.audio_sends[0][0].bus, ids[2]);
    }

    /// 总线自发送随其他总线删除保持指向自己。
    #[test]
    fn bus_self_send_survives_other_bus_removal() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let ids = add_buses(&mut p, 2);
        p.bus_sends[1].push(send(ids[1], 1.0, false));
        p.remove_bus(ids[0]);
        assert_eq!(p.bus_sends.len(), 1);
        assert_eq!(p.bus_sends[0].len(), 1, "自发送应保留");
        assert_eq!(p.bus_sends[0][0].bus, ids[1], "仍指向自己");
    }

    /// ensure_len 清理三张发送表里指向不存在总线的条目。
    #[test]
    fn ensure_len_prunes_all_send_tables() {
        let mut p = MixerParams::default();
        p.ensure_len();
        let _ = p.add_bus();
        p.sends[0].push(send(BusId(5), 1.0, false));
        p.audio_sends = vec![vec![send(BusId(5), 1.0, false)]];
        p.bus_sends[0].push(send(BusId(5), 1.0, false));
        p.ensure_len();
        assert!(p.sends[0].is_empty());
        assert!(p.audio_sends[0].is_empty());
        assert!(p.bus_sends[0].is_empty());
    }

    /// 重复 BusId 修复：后面的重复项重新分配，避免两条总线共享自动化。
    #[test]
    fn duplicate_bus_ids_are_reassigned() {
        let mut p = MixerParams {
            buses: vec![StripParams::default(); 2],
            bus_ids: vec![BusId(7), BusId(7)],
            next_bus_id: BusId(8),
            ..MixerParams::default()
        };
        p.ensure_len();
        assert_eq!(p.bus_ids, vec![BusId(7), BusId(8)]);
        assert_eq!(p.next_bus_id, BusId(9));
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

        let send = SendParams {
            bus,
            amount: 0.5,
            pre_fader: false,
        };
        assert!(p.set_node_sends(MixerChannelId::Midi(1), vec![send]));
        assert_eq!(p.node_sends(MixerChannelId::Midi(1)), Some(&[send][..]));
        assert!(!p.set_node_sends(MixerChannelId::Master, vec![send]));

        assert!(p.set_node_output(MixerChannelId::Midi(1), Some(bus)));
        assert_eq!(p.node_output(MixerChannelId::Midi(1)), Some(bus));
        assert!(!p.set_node_output(MixerChannelId::Bus(bus), Some(bus)));
    }

    #[test]
    fn strip_out_of_range_gives_default() {
        let p = MixerParams {
            channels: Vec::new(),
            channel_inserts: Vec::new(),
            ..MixerParams::default()
        };
        assert_eq!(p.strip(200), StripParams::default());
    }
}
