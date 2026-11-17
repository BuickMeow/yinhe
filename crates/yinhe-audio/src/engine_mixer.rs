//! 引擎的混音台接线：MixerParams（源通道索引）↔ MixerGraph（dense 索引）映射，
//! insert 命令处理与处理器回收。

use yinhe_mixer::{
    ChannelOutput, ChannelSendTarget, InsertProcessor, InstrumentProcessor, MasterParams,
    MixerParams, ResolvedChannelSend, ResolvedSend, SendParams, StripParams,
};
use yinhe_types::{BusId, MixerChannelId};

use crate::channel_layout::ChannelNamespace;
use crate::engine::AudioEngine;
use crate::spawn::InsertTarget;

impl AudioEngine {
    /// 全量同步混音台参数（引擎 spawn/工程加载后由 UI 推一次）。
    /// 只推 strip/master 参数；insert 处理器走 Insert* 命令单独进。
    pub(crate) fn set_mixer_params(&mut self, mut params: MixerParams) {
        // 防御：通道表补齐到当前布局需要的长度（只增不减，保留下线通道设置）。
        params.ensure_channel_tables(self.channel_layout.audio_channels().len());
        params.ensure_len();
        self.mixer_params = params;
        let strips = self.dense_strip_params();
        for (dense, p) in strips.into_iter().enumerate() {
            self.mixer.set_strip(dense, p);
        }
        self.mixer.set_master(MasterParams {
            gain: self.mixer_params.master.gain,
        });
        // 总线与发送：全量同步（数量变化走 resize_buses）。
        let bus_strips: Vec<StripParams> = self.mixer_params.bus_strips();
        self.mixer
            .resize_buses(bus_strips.len(), self.mixer.frames(), &bus_strips);
        for (i, p) in bus_strips.into_iter().enumerate() {
            self.mixer.set_bus_strip(i, p);
        }
        self.sync_output_routing();
        self.sync_sends_to_graph();
    }

    /// 把持久化的通道主输出路由推到混音图。
    /// 源通道按 dense 映射到 Master 或某条总线。
    fn sync_output_routing(&mut self) {
        let count = self.channel_layout.compacted_channels() as usize;
        for dense in 0..count {
            let target = match self.dense_to_namespace(dense) {
                Some(ChannelNamespace::Midi(src)) => self
                    .mixer_params
                    .node_output(MixerChannelId::Midi(src as u8)),
                Some(ChannelNamespace::Audio(ach)) => self
                    .mixer_params
                    .node_output(MixerChannelId::Audio(ach as u16)),
                Some(ChannelNamespace::TrackLc { lc, .. }) => self
                    .mixer_params
                    .track_node(lc)
                    .and_then(|node| node.output),
                Some(ChannelNamespace::DirectMaster) | None => None,
            };
            let dest = target.and_then(|id| match id {
                yinhe_types::ChannelId::Bus(bus) => {
                    self.mixer_params.bus_index(bus).map(ChannelOutput::Bus)
                }
                yinhe_types::ChannelId::Lc(_) | yinhe_types::ChannelId::Ap(_) => self
                    .dense_for_channel_id(id)
                    .map(|target| ChannelOutput::Channel(target as usize)),
                yinhe_types::ChannelId::Master => None,
            });
            self.mixer.set_channel_output(dense, dest);
        }
        // Bus 主输出（嵌套 Bus）。参数层已做环检测；这里只映射稳定 ID → 下标。
        for i in 0..self.mixer_params.bus_count() {
            let Some(id) = self.mixer_params.bus_id_at(i) else {
                continue;
            };
            let dest = self
                .mixer_params
                .node_output(MixerChannelId::Bus(id))
                .and_then(|target| match target {
                    yinhe_types::ChannelId::Bus(bus) => self.mixer_params.bus_index(bus),
                    _ => None,
                });
            self.mixer.set_bus_output(i, dest);
        }
    }

    /// 把按通道命名空间索引的发送列表映射到 dense 通道并推给混音图；
    /// 总线发送（bus → bus）按总线索引直接推。
    /// 未激活通道的 send 暂存于持久化层，引擎重建/激活后由全量同步补上。
    fn sync_sends_to_graph(&mut self) {
        let sends: Vec<Vec<SendParams>> = self.mixer_params.midi_sends_snapshot();
        let audio_sends: Vec<Vec<SendParams>> = self.mixer_params.audio_sends_snapshot();
        let count = self.channel_layout.compacted_channels() as usize;
        for dense in 0..count {
            let list = self
                .dense_to_namespace(dense)
                .and_then(|ns| match ns {
                    ChannelNamespace::Midi(src) => sends.get(src),
                    ChannelNamespace::Audio(ach) => audio_sends.get(ach),
                    ChannelNamespace::TrackLc { lc, .. } => {
                        self.mixer_params.track_node(lc).map(|node| &node.sends)
                    }
                    ChannelNamespace::DirectMaster => None,
                })
                .map(|list| self.resolve_sends(list))
                .unwrap_or_default();
            self.mixer.set_sends(dense, list);
        }
        let bus_sends: Vec<Vec<SendParams>> = self.mixer_params.bus_sends_snapshot();
        for (i, list) in bus_sends.into_iter().enumerate() {
            let resolved = self.resolve_sends(&list);
            self.mixer.set_bus_sends(i, resolved);
        }

        // C 的任意通道发送：按 source 聚到 dense 通道。
        let mut channel_sends: Vec<Vec<ResolvedChannelSend>> = vec![Vec::new(); count];
        for send in &self.mixer_params.channel_sends {
            let Some(src) = self.dense_for_channel_id(send.source) else {
                continue;
            };
            let target = match send.target {
                yinhe_types::ChannelId::Master => ChannelSendTarget::Master,
                yinhe_types::ChannelId::Bus(bus) => {
                    let Some(index) = self.mixer_params.bus_index(bus) else {
                        continue;
                    };
                    ChannelSendTarget::Bus(index)
                }
                other => {
                    let Some(dst) = self.dense_for_channel_id(other) else {
                        continue;
                    };
                    ChannelSendTarget::Channel(dst as usize)
                }
            };
            if let Some(list) = channel_sends.get_mut(src as usize) {
                list.push(ResolvedChannelSend {
                    target,
                    amount: send.amount,
                    pre_fader: send.pre_fader,
                });
            }
        }
        for (dense, list) in channel_sends.into_iter().enumerate() {
            self.mixer.set_channel_sends(dense, list);
        }
    }

    /// LC ID → dense 通道下标（按显式 source 角色，不从 ID 数值反推）。
    pub(crate) fn lc_dense(&self, id: yinhe_types::LogicalChannelId) -> u32 {
        use yinhe_types::{ChannelId, ChannelSource};
        match self.mixer_params.source_of(ChannelId::Lc(id)) {
            Some(ChannelSource::Midi(ch)) => self.channel_layout.dense_for(ch as usize),
            Some(ChannelSource::Audio(ch)) => self.channel_layout.audio_dense_for(ch),
            Some(ChannelSource::Track(track)) if track != u16::MAX => {
                self.channel_layout.track_lc_dense(track as usize)
            }
            _ => self
                .channel_layout
                .track_lc_dense_for(id)
                .map(|(_, dense)| dense)
                .unwrap_or(u32::MAX),
        }
    }

    /// 稳定 ChannelId → dense 通道下标。
    fn dense_for_channel_id(&self, id: yinhe_types::ChannelId) -> Option<u32> {
        let lc = match id {
            yinhe_types::ChannelId::Lc(lc) => lc,
            yinhe_types::ChannelId::Ap(ap) => {
                let group = self.mixer_params.ap(ap.group)?;
                group.channel
            }
            yinhe_types::ChannelId::Bus(_) | yinhe_types::ChannelId::Master => return None,
        };
        let dense = self.lc_dense(lc);
        (dense != u32::MAX).then_some(dense)
    }

    /// 持久化层用稳定 BusId；渲染图只认 `buses` 下标。
    /// 指向已删除总线的 send 在 `ensure_len` 已清理，这里再防御一次。
    fn resolve_sends(&self, sends: &[SendParams]) -> Vec<ResolvedSend> {
        sends
            .iter()
            .filter_map(|s| {
                let yinhe_types::ChannelId::Bus(target) = s.target else {
                    return None;
                };
                self.mixer_params.bus_index(target).map(|bus| ResolvedSend {
                    bus,
                    amount: s.amount,
                    pre_fader: s.pre_fader,
                })
            })
            .collect()
    }

    /// dense 索引 → 所属通道命名空间（仅结构性操作时调用，O(1)）。
    fn dense_to_namespace(&self, dense: usize) -> Option<ChannelNamespace> {
        let d = dense as u32;
        let audio_base = self.channel_layout.midi_compacted();
        let track_base = audio_base + self.channel_layout.audio_channels().len() as u32;
        if Some(d) == self.channel_layout.direct_master_dense() {
            return Some(ChannelNamespace::DirectMaster);
        }
        if d >= track_base {
            let idx = (d - track_base) as usize;
            self.channel_layout
                .track_lc_channels()
                .get(idx)
                .map(|&(track, lc)| ChannelNamespace::TrackLc { track, lc })
        } else if d >= audio_base {
            let idx = (d - audio_base) as usize;
            self.channel_layout
                .audio_channels()
                .get(idx)
                .map(|&ach| ChannelNamespace::Audio(ach as usize))
        } else {
            self.channel_layout
                .channel_map()
                .iter()
                .position(|&x| x as usize == dense)
                .map(ChannelNamespace::Midi)
        }
    }

    /// 更新任意混音节点的 strip（推子/声像/M/S 高频路径，幂等）。
    pub(crate) fn set_node_strip(&mut self, node: MixerChannelId, params: StripParams) {
        match node {
            MixerChannelId::Midi(ch) => self.set_channel_strip(ch, params),
            MixerChannelId::Audio(ch) => self.set_audio_strip(ch, params),
            MixerChannelId::TrackLc(id) => {
                if let Some(node) = self.mixer_params.track_node_mut(id) {
                    node.strip = params;
                }
                if let Some((_, dense)) = self.channel_layout.track_lc_dense_for(id)
                    && dense != u32::MAX
                {
                    self.mixer.set_strip(dense as usize, params);
                }
            }
            MixerChannelId::Bus(bus) => self.set_bus_strip(bus, params),
            MixerChannelId::Master => {
                self.mixer_params.master.strip.gain = params.gain;
                self.mixer.set_master(MasterParams { gain: params.gain });
            }
        }
    }

    /// 更新某总线的 strip 参数（高频路径）。
    pub(crate) fn set_bus_strip(&mut self, bus: BusId, params: StripParams) {
        let Some(idx) = self.mixer_params.bus_index(bus) else {
            return;
        };
        if let Some(node) = self.mixer_params.bus_node_mut(bus) {
            node.strip = params;
        }
        self.mixer.set_bus_strip(idx, params);
    }

    /// 全量同步总线参数与发送（增删总线 / 改 send 后推一次）。
    pub(crate) fn sync_bus_config(
        &mut self,
        buses: Vec<StripParams>,
        sends: Vec<Vec<SendParams>>,
        bus_sends: Vec<Vec<SendParams>>,
        audio_sends: Vec<Vec<SendParams>>,
        track_sends: Vec<Vec<SendParams>>,
    ) {
        // 旧命令仍按「总线条 / 源通道发送 / 音频发送」三张表传递；
        // 这里落到 v10 的节点表，保持命令层不用一次性改完。
        for (i, strip) in buses.into_iter().enumerate() {
            let Some(bus) = self.mixer_params.bus_id_at(i) else {
                continue;
            };
            if let Some(node) = self.mixer_params.bus_node_mut(bus) {
                node.strip = strip;
            }
        }
        self.mixer_params.ensure_channel_tables(audio_sends.len());
        for (i, list) in sends.into_iter().enumerate() {
            if let Some(node) = self.mixer_params.midi_node_mut(i as u8) {
                node.sends = list;
            }
        }
        self.mixer_params.ensure_channel_tables(audio_sends.len());
        for (i, list) in audio_sends.into_iter().enumerate() {
            if let Some(node) = self.mixer_params.audio_node_mut(i as u16) {
                node.sends = list;
            }
        }
        let track_ids: Vec<_> = self
            .mixer_params
            .nodes
            .iter()
            .filter_map(|node| match (node.id, node.source) {
                (yinhe_types::ChannelId::Lc(id), yinhe_types::ChannelSource::Track(_)) => Some(id),
                _ => None,
            })
            .collect();
        for (i, list) in track_sends.into_iter().enumerate() {
            let Some(id) = track_ids.get(i) else {
                break;
            };
            if let Some(node) = self.mixer_params.track_node_mut(*id) {
                node.sends = list;
            }
        }
        for (i, list) in bus_sends.into_iter().enumerate() {
            let Some(bus) = self.mixer_params.bus_id_at(i) else {
                continue;
            };
            if let Some(node) = self.mixer_params.bus_node_mut(bus) {
                node.sends = list;
            }
        }
        let bus_strips: Vec<StripParams> = self.mixer_params.bus_strips();
        self.mixer
            .resize_buses(bus_strips.len(), self.mixer.frames(), &bus_strips);
        for (i, p) in bus_strips.into_iter().enumerate() {
            self.mixer.set_bus_strip(i, p);
        }
        self.sync_output_routing();
        self.sync_sends_to_graph();
    }

    /// 按 tick 求值 LC / Bus / Master 的 MixerGain / MixerPan 自动化。
    pub(crate) fn apply_mixer_automation(&mut self, tick: u32) {
        use yinhe_types::AutomationTarget;

        let mut updates: Vec<(MixerChannelId, Option<f32>, Option<f32>)> = Vec::new();
        let mut send_updates: Vec<(MixerChannelId, BusId, f32)> = Vec::new();
        let mut insert_updates: Vec<(MixerChannelId, usize, u32, f64)> = Vec::new();
        let mut collect = |node: &yinhe_mixer::MixerNode| {
            let Some(channel) = mixer_channel_of(node.id, &self.mixer_params) else {
                return;
            };
            let gain = node
                .automation
                .iter()
                .find(|lane| lane.target == AutomationTarget::MixerGain { channel });
            let pan = node
                .automation
                .iter()
                .find(|lane| lane.target == AutomationTarget::MixerPan { channel });
            let gain = gain.map(|lane| {
                lane.value_at(tick)
                    .map(|(v, _)| v)
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0)
            });
            let pan = pan.map(|lane| {
                let v = lane
                    .value_at(tick)
                    .map(|(v, _)| v)
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0);
                v * 2.0 - 1.0
            });
            if gain.is_some() || pan.is_some() {
                updates.push((channel, gain, pan));
            }
            for lane in &node.automation {
                let AutomationTarget::MixerSend { node: target, bus } = lane.target else {
                    continue;
                };
                if target != channel {
                    continue;
                }
                let value = lane
                    .value_at(tick)
                    .map(|(v, _)| v)
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                send_updates.push((channel, bus, value * 2.0));
            }
            for lane in &node.automation {
                let AutomationTarget::InsertParam {
                    target, slot, id, ..
                } = lane.target
                else {
                    continue;
                };
                if target != channel {
                    continue;
                }
                let value = lane
                    .value_at(tick)
                    .map(|(v, _)| f64::from(v))
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                insert_updates.push((channel, slot, id, value));
            }
        };
        for node in self.mixer_params.nodes() {
            collect(node);
        }
        collect(&self.mixer_params.master);

        for (channel, gain, pan) in updates {
            let mut params = self.mixer_params.node_strip(channel);
            if let Some(gain) = gain {
                params.gain = gain;
            }
            if let Some(pan) = pan {
                params.pan = pan;
            }
            self.set_node_strip(channel, params);
        }
        for (channel, slot, id, value) in insert_updates {
            match channel {
                MixerChannelId::Midi(ch) => {
                    if let Some(dense) = self.dense_of(ch) {
                        self.mixer.set_channel_insert_param(dense, slot, id, value);
                    }
                }
                MixerChannelId::Audio(ch) => {
                    let dense = self.channel_layout.audio_dense_for(ch);
                    if dense != u32::MAX {
                        self.mixer
                            .set_channel_insert_param(dense as usize, slot, id, value);
                    }
                }
                MixerChannelId::TrackLc(id_lc) => {
                    if let Some((_, dense)) = self.channel_layout.track_lc_dense_for(id_lc)
                        && dense != u32::MAX
                    {
                        self.mixer
                            .set_channel_insert_param(dense as usize, slot, id, value);
                    }
                }
                MixerChannelId::Bus(bus) => {
                    if let Some(idx) = self.mixer_params.bus_index(bus) {
                        self.mixer.set_bus_insert_param(idx, slot, id, value);
                    }
                }
                MixerChannelId::Master => {
                    self.mixer.set_master_insert_param(slot, id, value);
                }
            }
        }
        for (channel, bus, amount) in send_updates {
            let Some(node) = self
                .mixer_params
                .node_mut(self.mixer_params.node_id_for(channel))
            else {
                continue;
            };
            if let Some(send) = node
                .sends
                .iter_mut()
                .find(|s| s.target == yinhe_types::ChannelId::Bus(bus))
            {
                send.amount = amount;
            }
            let sends: Vec<SendParams> = node
                .sends
                .iter()
                .filter(|s| s.amount > 0.0)
                .copied()
                .collect();
            let resolved = self.resolve_sends(&sends);
            match channel {
                MixerChannelId::Midi(ch) => {
                    if let Some(dense) = self.dense_of(ch) {
                        self.mixer.set_sends(dense, resolved);
                    }
                }
                MixerChannelId::Audio(ch) => {
                    let dense = self.channel_layout.audio_dense_for(ch);
                    if dense != u32::MAX {
                        self.mixer.set_sends(dense as usize, resolved);
                    }
                }
                MixerChannelId::TrackLc(id) => {
                    if let Some((_, dense)) = self.channel_layout.track_lc_dense_for(id)
                        && dense != u32::MAX
                    {
                        self.mixer.set_sends(dense as usize, resolved);
                    }
                }
                MixerChannelId::Bus(bus_id) => {
                    if let Some(idx) = self.mixer_params.bus_index(bus_id) {
                        self.mixer.set_bus_sends(idx, resolved);
                    }
                }
                MixerChannelId::Master => {}
            }
        }
    }

    /// 按 tick 求值 BusGain 自动化并写入总线 strip。
    /// 没有 lane 的总线保持用户推子值；lane 存在但事件尚未开始时用 1.0。
    pub(crate) fn apply_bus_automation(&mut self, tick: u32) {
        let updates: Vec<(BusId, f32)> = self
            .mixer_params
            .nodes()
            .filter_map(|node| {
                let yinhe_types::ChannelId::Bus(bus_id) = node.id else {
                    return None;
                };
                let lane = node.automation.iter().find(|lane| {
                    lane.target == yinhe_types::AutomationTarget::BusGain { bus: bus_id }
                })?;
                let value = lane
                    .value_at(tick)
                    .map(|(v, _)| v)
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0);
                Some((bus_id, value))
            })
            .collect();
        for (bus_id, value) in updates {
            let Some(idx) = self.mixer_params.bus_index(bus_id) else {
                continue;
            };
            if let Some(node) = self.mixer_params.bus_node_mut(bus_id)
                && (node.strip.gain - value).abs() > f32::EPSILON
            {
                node.strip.gain = value;
                self.mixer.set_bus_gain(idx, value);
            }
        }
    }

    /// 更新某源通道的 strip（推子/声像/M/S 拖动的高频路径，幂等）。
    pub(crate) fn set_channel_strip(&mut self, channel: u8, params: StripParams) {
        if let Some(slot) = self.mixer_params.midi_node_mut(channel) {
            slot.strip = params;
        }
        let dense = self.channel_layout.dense_for(channel as usize);
        if dense != u32::MAX {
            self.mixer.set_strip(dense as usize, params);
        }
    }

    pub(crate) fn set_master_params(&mut self, params: MasterParams) {
        self.mixer_params.master.strip.gain = params.gain;
        self.mixer.set_master(params);
    }

    /// 更新某音频通道的 strip（推子/声像/M/S 拖动的高频路径，幂等）。
    pub(crate) fn set_audio_strip(&mut self, channel: u16, params: StripParams) {
        self.mixer_params
            .ensure_channel_tables(channel as usize + 1);
        if let Some(slot) = self.mixer_params.audio_node_mut(channel) {
            slot.strip = params;
        }
        let dense = self.channel_layout.audio_dense_for(channel);
        if dense != u32::MAX {
            self.mixer.set_strip(dense as usize, params);
        }
    }

    /// 各 dense 通道当前的 strip 参数（resize 重建 strip 状态用）。
    /// MIDI 源通道按 channel_map 反查，音频通道按 dense 段位置索引。
    pub(crate) fn dense_strip_params(&self) -> Vec<StripParams> {
        (0..self.channel_layout.compacted_channels() as usize)
            .map(|dense| match self.dense_to_namespace(dense) {
                Some(ChannelNamespace::Midi(src)) => self.mixer_params.strip(src as u8),
                Some(ChannelNamespace::Audio(ach)) => self.mixer_params.audio_strip(ach as u16),
                Some(ChannelNamespace::TrackLc { lc, .. }) => self
                    .mixer_params
                    .track_node(lc)
                    .map(|node| node.strip)
                    .unwrap_or_default(),
                Some(ChannelNamespace::DirectMaster) | None => StripParams::default(),
            })
            .collect()
    }

    /// 源通道 → dense 索引（未激活返回 None）。
    fn dense_of(&self, channel: u8) -> Option<usize> {
        let dense = self.channel_layout.dense_for(channel as usize);
        (dense != u32::MAX).then_some(dense as usize)
    }

    pub(crate) fn insert_add(
        &mut self,
        target: InsertTarget,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    ) {
        match target {
            InsertTarget::Midi(ch) => match self.dense_of(ch) {
                Some(dense) => self.mixer.insert_insert(dense, slot, processor),
                // 通道未激活（模型无音轨用此通道）：处理器无处安放，直接退回。
                None => self.insert_returns.push(processor),
            },
            InsertTarget::Audio(ch) => {
                let dense = self.channel_layout.audio_dense_for(ch);
                if dense != u32::MAX {
                    self.mixer.insert_insert(dense as usize, slot, processor);
                } else {
                    self.insert_returns.push(processor);
                }
            }
            InsertTarget::TrackLc(id) => {
                let dense = self
                    .channel_layout
                    .track_lc_dense_for(id)
                    .map(|(_, dense)| dense)
                    .unwrap_or(u32::MAX);
                if dense != u32::MAX {
                    self.mixer.insert_insert(dense as usize, slot, processor);
                } else {
                    self.insert_returns.push(processor);
                }
            }
            InsertTarget::Bus(bus) => match self.mixer_params.bus_index(bus) {
                Some(idx) => self.mixer.insert_bus_insert(idx, slot, processor),
                None => self.insert_returns.push(processor),
            },
            InsertTarget::Master => self.mixer.insert_master_insert(slot, processor),
        }
    }

    pub(crate) fn insert_remove(&mut self, target: InsertTarget, slot: usize) {
        let removed = match target {
            InsertTarget::Midi(ch) => self
                .dense_of(ch)
                .and_then(|dense| self.mixer.remove_insert(dense, slot)),
            InsertTarget::Audio(ch) => {
                let dense = self.channel_layout.audio_dense_for(ch);
                (dense != u32::MAX)
                    .then(|| self.mixer.remove_insert(dense as usize, slot))
                    .flatten()
            }
            InsertTarget::TrackLc(id) => {
                self.channel_layout
                    .track_lc_dense_for(id)
                    .and_then(|(_, dense)| {
                        (dense != u32::MAX)
                            .then(|| self.mixer.remove_insert(dense as usize, slot))
                            .flatten()
                    })
            }
            InsertTarget::Bus(bus) => self
                .mixer_params
                .bus_index(bus)
                .and_then(|idx| self.mixer.remove_bus_insert(idx, slot)),
            InsertTarget::Master => self.mixer.remove_master_insert(slot),
        };
        if let Some(p) = removed {
            self.insert_returns.push(p);
        }
    }

    pub(crate) fn insert_replace(
        &mut self,
        target: InsertTarget,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    ) {
        let old = match target {
            InsertTarget::Midi(ch) => self
                .dense_of(ch)
                .and_then(|dense| self.mixer.replace_insert(dense, slot, processor)),
            InsertTarget::Audio(ch) => {
                let dense = self.channel_layout.audio_dense_for(ch);
                (dense != u32::MAX)
                    .then(|| self.mixer.replace_insert(dense as usize, slot, processor))
                    .flatten()
            }
            InsertTarget::TrackLc(id) => {
                self.channel_layout
                    .track_lc_dense_for(id)
                    .and_then(|(_, dense)| {
                        (dense != u32::MAX)
                            .then(|| self.mixer.replace_insert(dense as usize, slot, processor))
                            .flatten()
                    })
            }
            InsertTarget::Bus(bus) => self
                .mixer_params
                .bus_index(bus)
                .and_then(|idx| self.mixer.replace_bus_insert(idx, slot, processor)),
            InsertTarget::Master => self.mixer.replace_master_insert(slot, processor),
        };
        if let Some(p) = old {
            self.insert_returns.push(p);
        }
    }

    /// 取出待回收的 insert 处理器（renderer 每轮命令处理后调用，送回 UI 线程）。
    pub(crate) fn drain_insert_returns(&mut self) -> Vec<Box<dyn InsertProcessor>> {
        std::mem::take(&mut self.insert_returns)
    }

    /// 安装/替换/移除某 MIDI 通道乐器链里的**一个槽位**（按 `slot_id`）。
    /// - `processor = Some`：链内不存在则追加、存在则替换（旧处理器退回）；
    /// - `processor = None`：移除该槽位（旧处理器退回）；链空后该通道回到默认 XSynth。
    ///
    /// 由 `AudioCommand::SetInstrumentSlot` 触发，渲染线程调用。被替换/移除的旧
    /// 处理器（以及通道未激活却收到安装命令的多余处理器）攒进 `instrument_returns`
    /// 送回 UI 线程 deactivate——渲染线程不能 deactivate 插件。
    pub(crate) fn set_instrument_slot(
        &mut self,
        channel: u8,
        slot_id: u64,
        processor: Option<Box<dyn InstrumentProcessor>>,
    ) {
        let dense = self.channel_layout.dense_for(channel as usize);
        let Some(dense) = (dense != u32::MAX).then_some(dense as usize) else {
            if let Some(p) = processor {
                self.instrument_returns.push((slot_id, p));
            }
            return;
        };
        if dense >= self.instruments.len() {
            // 命令与模型不同步（dense 越界）：直接退回，不越界写。
            if let Some(p) = processor {
                self.instrument_returns.push((slot_id, p));
            }
            return;
        }
        let old: Option<Box<dyn InstrumentProcessor>> = match processor {
            Some(p) => {
                let src = self.instruments[dense]
                    .get_or_insert_with(crate::instrument::InstrumentSource::new);
                match src.chain.iter_mut().find(|c| c.slot_id == slot_id) {
                    Some(slot) => Some(std::mem::replace(&mut slot.processor, p)),
                    None => {
                        src.chain.push(crate::instrument::InstrumentProc {
                            slot_id,
                            params: Vec::new(),
                            processor: p,
                        });
                        None
                    }
                }
            }
            None => {
                let mut removed = None;
                if let Some(src) = self.instruments[dense].as_mut()
                    && let Some(pos) = src.chain.iter().position(|c| c.slot_id == slot_id)
                {
                    removed = Some(src.chain.remove(pos).processor);
                    if src.chain.is_empty() {
                        self.instruments[dense] = None;
                    }
                }
                removed
            }
        };
        if let Some(old) = old {
            self.instrument_returns.push((slot_id, old));
        }
        self.refresh_channel_latency(dense);
    }

    /// 重算某 dense 通道的乐器延迟（链内取最大值）并写入混音图。
    /// OwnLc 单轨的插件输出在 home LC，延迟也写到那条通道以正确 PDC。
    fn refresh_channel_latency(&mut self, dense: usize) {
        let latency = self.instruments[dense]
            .as_ref()
            .map(|src| {
                src.chain
                    .iter()
                    .map(|p| p.processor.latency_samples())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let out_dense = self.channel_layout.instrument_output_dense(dense) as usize;
        self.mixer.set_channel_latency(dense, latency);
        if out_dense != dense {
            self.mixer.set_channel_latency(out_dense, latency);
        }
    }

    /// 取出待回收的乐器处理器（renderer 每轮命令处理后调用，送回 UI 线程）。
    pub(crate) fn drain_instrument_returns(&mut self) -> Vec<(u64, Box<dyn InstrumentProcessor>)> {
        std::mem::take(&mut self.instrument_returns)
    }

    /// 插件延迟变化：重新查询乐器延迟并重算 PDC 对齐（insert 链延迟由
    /// `refresh_pdc` 内部向各处理器查询最新值）。
    pub(crate) fn refresh_latency(&mut self) {
        for dense in 0..self.instruments.len() {
            if self.instruments[dense].is_some() {
                self.refresh_channel_latency(dense);
            }
        }
        self.mixer.refresh_pdc();
    }
}

fn mixer_channel_of(
    id: yinhe_types::ChannelId,
    mixer: &yinhe_mixer::MixerParams,
) -> Option<MixerChannelId> {
    use yinhe_types::{ChannelId, ChannelSource};
    match id {
        ChannelId::Lc(lc) => match mixer.source_of(id)? {
            ChannelSource::Midi(ch) => Some(MixerChannelId::Midi(ch)),
            ChannelSource::Audio(ch) => Some(MixerChannelId::Audio(ch)),
            ChannelSource::Track(_) => Some(MixerChannelId::TrackLc(lc)),
            _ => None,
        },
        ChannelId::Ap(_) => None,
        ChannelId::Bus(bus) => Some(MixerChannelId::Bus(bus)),
        ChannelId::Master => Some(MixerChannelId::Master),
    }
}
