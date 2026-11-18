use super::*;

impl MixerParams {
    pub(super) fn repair_automation(&mut self) {
        // 先解析好 UI 查询键 → 实际 ChannelId，避免与节点可变借用冲突。
        let mut resolved: HashMap<MixerChannelId, ChannelId> = HashMap::new();
        for ch in 0..CHANNEL_COUNT as u8 {
            resolved.insert(
                MixerChannelId::Midi(ch),
                self.channel_id_for(MixerChannelId::Midi(ch)),
            );
        }
        for ch in 0..CHANNEL_COUNT as u16 {
            resolved.insert(
                MixerChannelId::Audio(ch),
                self.channel_id_for(MixerChannelId::Audio(ch)),
            );
        }
        // 每轨 LC 的自动化 owner 是 TrackLc，也必须参与解析，否则加载后
        // 其 MixerGain/MixerPan lane 会被误判为「不属于该节点」而删除。
        let track_lcs: Vec<LogicalChannelId> = self
            .nodes
            .iter()
            .filter(|node| matches!(node.source, ChannelSource::Track(_)))
            .filter_map(|node| match node.id {
                ChannelId::Lc(id) => Some(id),
                _ => None,
            })
            .collect();
        for lc in track_lcs {
            resolved.insert(MixerChannelId::TrackLc(lc), ChannelId::Lc(lc));
        }
        resolved.insert(MixerChannelId::Master, ChannelId::Master);
        let valid = |channel: ChannelId, lane: &yinhe_types::AutomationLane| match &lane.target {
            yinhe_types::AutomationTarget::MixerGain { channel: c }
            | yinhe_types::AutomationTarget::MixerPan { channel: c }
            | yinhe_types::AutomationTarget::MixerSend { source: c, .. }
            | yinhe_types::AutomationTarget::InsertParam { target: c, .. } => {
                resolved.get(c).copied().unwrap_or(ChannelId::Master) == channel
            }
            _ => true,
        };
        for node in self.nodes_mut() {
            let channel = node.id;
            node.automation.retain(|lane| valid(channel, lane));
        }
        let master_channel = self.master.id;
        self.master
            .automation
            .retain(|lane| valid(master_channel, lane));
    }

    pub(super) fn repair_racks(&mut self) {
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
    pub(super) fn repair_aps(&mut self) {
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

    pub(super) fn repair_channel_sends(&mut self) {
        let sends = std::mem::take(&mut self.channel_sends);
        self.channel_sends = sends
            .into_iter()
            .filter(|send| {
                send.source != ChannelId::Master
                    && send.amount > 0.0
                    && self.channel_target_exists(send.target)
            })
            .collect();
        for send in &mut self.channel_sends {
            send.amount = send.amount.clamp(0.0, 1.0);
        }
        let mut seen = HashSet::new();
        self.channel_sends
            .retain(|send| seen.insert((send.source, send.target)));
    }

    pub(super) fn sort_dedup_nodes(&mut self) {
        self.nodes.sort_by_key(|node| node.id);
    }

    pub(super) fn alloc_lc_id(&mut self) -> LogicalChannelId {
        let id = self.next_lc_id;
        self.next_lc_id = LogicalChannelId(id.0.saturating_add(1));
        id
    }

    pub(super) fn repair_channels(&mut self) {
        for i in 0..CHANNEL_COUNT {
            self.ensure_lc_node(ChannelSource::Midi(i as u8));
        }
    }

    /// 修复重复/越界的 LC ID（独立 u16 空间；持有 Track 来源的节点保持原 ID）。
    pub(super) fn repair_track_channels(&mut self) {
        let mut seen: HashSet<LogicalChannelId> = HashSet::new();
        for node in &mut self.nodes {
            if !matches!(node.source, ChannelSource::Track(_))
                || !matches!(node.id, ChannelId::Lc(_))
            {
                continue;
            }
            let mut id = match node.id {
                ChannelId::Lc(id) => id,
                _ => unreachable!(),
            };
            if !seen.insert(id) {
                id = self.next_lc_id;
                seen.insert(id);
                self.next_lc_id = LogicalChannelId(self.next_lc_id.0.saturating_add(1));
                node.id = ChannelId::Lc(id);
            }
            if id.0 >= self.next_lc_id.0 {
                self.next_lc_id = LogicalChannelId(id.0.saturating_add(1));
            }
        }
        self.sort_dedup_nodes();
    }

    pub(super) fn repair_audio_channels(&mut self) {
        // 音频 LC 的 ID 由 ensure_audio_len 保证连续；这里只去重/排序。
        self.sort_dedup_nodes();
    }

    /// 混合 LC 不冻结：有任意 C 发送汇入的 LC，清除 freeze。
    pub(super) fn repair_mixed_lc_freezes(&mut self) {
        let targets: HashSet<LogicalChannelId> = self
            .channel_sends
            .iter()
            .filter_map(|send| match send.target {
                ChannelId::Lc(id) => Some(id),
                ChannelId::Ap(ap) => self.ap(ap.group).map(|group| group.channel),
                ChannelId::Master => None,
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        for node in self.nodes_mut() {
            if let ChannelId::Lc(id) = node.id
                && targets.contains(&id)
            {
                node.freeze = None;
            }
        }
    }

    pub(super) fn repair_master(&mut self) {
        self.master.id = ChannelId::Master;
    }

    pub(super) fn repair_routing(&mut self) {
        let known_targets: HashSet<ChannelId> = self
            .nodes
            .iter()
            .map(|node| node.id)
            .chain(std::iter::once(ChannelId::Master))
            .collect();
        for node in self.nodes.iter_mut() {
            if node.output.is_some_and(|id| {
                id == ChannelId::Master || id == node.id || !known_targets.contains(&id)
            }) {
                node.output = None;
            }
        }
    }

    pub(super) fn ensure_audio_len(&mut self, len: usize) {
        for i in 0..len {
            self.ensure_lc_node(ChannelSource::Audio(i as u16));
        }
    }

    pub(super) fn node_mut_for_ui(&mut self, node: MixerChannelId) -> Option<&mut MixerNode> {
        if let MixerChannelId::Audio(ch) = node {
            self.ensure_audio_len(ch as usize + 1);
        }
        self.find_node_mut(self.channel_id_for(node))
    }
}
