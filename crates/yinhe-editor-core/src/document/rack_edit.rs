//! Rack 输出拓扑与每轨 LC 的文档级操作。
//!
//! Rack 默认是单输出：整个 Rack 一条 LC，所有使用它的 Track 共享。
//! 内置音源可以切到 `PerTrack`：每条 MIDI Track 分配一个稳定 LC，
//! 引擎按轨拆分音频后分别进入各自的 LC。
//!
//! 外部插件同样可以按轨拆分：每个 OwnLc Track 一个插件实例，
//! 输出写各自 home LC（方案 C 决策 #12）。

use std::sync::Arc;

use yinhe_core::{TrackAudioTarget, TrackData, TrackKind};
use yinhe_types::{ApGroupId, RackId, RackOutputMode};

use crate::document::Document;

impl Document {
    /// 某源 MIDI 通道的 Rack 输出模式。
    pub fn rack_output_mode(&self, channel: u8) -> RackOutputMode {
        self.mixer
            .racks
            .get(channel as usize)
            .map(|rack| rack.output_mode)
            .unwrap_or_default()
    }

    /// 切换 Rack 输出模式，并按模式分配/清理每轨 LC。
    ///
    /// 返回是否有变化。外部插件与内置音源都支持 `PerTrack`：
    /// 前者每轨一个实例、后者按轨拆音频。
    pub fn set_rack_output_mode(&mut self, channel: u8, mode: RackOutputMode) -> bool {
        let Some(rack) = self.mixer.racks.get(channel as usize) else {
            return false;
        };
        if rack.output_mode == mode {
            return false;
        }
        self.mixer.racks[channel as usize].output_mode = mode;
        let model = Arc::make_mut(&mut self.data.model);
        for (track_idx, track) in model.tracks.iter_mut().enumerate() {
            if track.kind != TrackKind::Midi || track.global_channel() != channel {
                continue;
            }
            match mode {
                RackOutputMode::PerTrack => {
                    if track.lc.is_none() {
                        // 复制共享 LC 的混音参数，避免切换模式后听感突变。
                        let source = self
                            .mixer
                            .ap_node(channel)
                            .cloned()
                            .unwrap_or_else(yinhe_mixer::MixerNode::master);
                        let id = self.mixer.add_track_lc(track_idx as u16);
                        if let Some(node) = self.mixer.track_node_mut(id) {
                            node.strip = source.strip;
                            node.inserts = source.inserts.clone();
                            node.output = source.output;
                        }
                        Arc::make_mut(track).lc = Some(id);
                    }
                    if let Some(lc) = track.lc {
                        Arc::make_mut(track).set_audio_target(TrackAudioTarget::OwnLc {
                            lc,
                            rack: Some(RackId::new(u16::from(channel))),
                        });
                    }
                }
                RackOutputMode::Shared => {
                    let track = Arc::make_mut(track);
                    if let Some(id) = track.lc.take() {
                        self.mixer.remove_track_lc(id);
                    }
                    track.set_audio_target(TrackAudioTarget::Ap {
                        ap: ApGroupId::new(u16::from(channel)),
                    });
                }
            }
        }
        self.ensure_aps();
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// 把 MIDI 轨链接到 AP：清除自己的 LC，音频走 AP 通道。
    /// 同一 MIDI 通道的其他 OwnLc 轨不受影响。
    pub fn link_track_to_ap(&mut self, track_idx: usize, ap: ApGroupId) -> bool {
        let Some(track) = self.data.model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Midi {
            return false;
        }
        if track.audio_target == Some(TrackAudioTarget::Ap { ap }) {
            return false;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        track.set_audio_target(TrackAudioTarget::Ap { ap });
        self.ensure_rack_track_lcs();
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// Audio Track：回到自己的 home LC。
    pub fn link_audio_track_to_own_lc(&mut self, track_idx: usize) -> bool {
        let Some(track) = self.data.model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Audio {
            return false;
        }
        let ch = track.audio_channel.unwrap_or(0);
        let lc = self
            .mixer
            .ensure_lc_node(yinhe_types::ChannelSource::Audio(ch));
        if track.audio_target == Some(TrackAudioTarget::OwnLc { lc, rack: None }) {
            return false;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        track.set_audio_target(TrackAudioTarget::OwnLc { lc, rack: None });
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// 某 MIDI 通道的乐器宿主列表（方案 C）：
    ///
    /// - 每条 OwnLc 轨一个实例（`Some(lc)`），各自写 home LC；
    /// - 存在 AP 轨时再加一条共享链（`None`）；
    /// - 没有 MIDI 轨时返回一条共享链兜底。
    ///
    /// UI（`InstrumentRack`）与音频引擎都以此为准，避免两边各自解析
    /// `TrackAudioTarget` 导致宿主对不上。
    /// 复杂度 O(轨数 × 已收集宿主数)：宿主数 = 同通道的 LC 数 + AP，
    /// 实际远小于轨数，无需额外哈希表。
    pub fn instrument_hosts(&self, channel: u8) -> Vec<Option<yinhe_types::LogicalChannelId>> {
        let mut hosts: Vec<Option<yinhe_types::LogicalChannelId>> = Vec::new();
        for t in self.data.model.tracks.iter() {
            if t.kind != TrackKind::Midi || t.global_channel() != channel {
                continue;
            }
            match t.effective_audio_target() {
                Some(TrackAudioTarget::OwnLc { lc, .. }) => {
                    if !hosts.contains(&Some(lc)) {
                        hosts.push(Some(lc));
                    }
                }
                Some(TrackAudioTarget::Ap { .. }) if !hosts.contains(&None) => hosts.push(None),
                Some(TrackAudioTarget::Ap { .. }) | None => {}
            }
        }
        if hosts.is_empty() {
            hosts.push(None);
        }
        hosts
    }

    /// 某轨的音频通道身份（C 统一身份）。
    ///
    /// - MIDI OwnLc → `ChannelId::Lc(lc)`
    /// - MIDI AP → `ChannelId::Ap`
    /// - 音频轨 → 对应音频 LC 节点 ID
    /// - Master → `ChannelId::Master`
    pub fn track_channel_id(&self, track_idx: usize) -> Option<yinhe_types::ChannelId> {
        use yinhe_types::{ApChannelId, ChannelId};
        let track = self.data.model.tracks.get(track_idx)?;
        match track.kind {
            TrackKind::Midi => match track.effective_audio_target() {
                Some(TrackAudioTarget::OwnLc { lc, .. }) => Some(ChannelId::Lc(lc)),
                Some(TrackAudioTarget::Ap { ap }) => Some(ChannelId::Ap(ApChannelId::new(ap, 0))),
                None => None,
            },
            TrackKind::Audio => track
                .audio_channel
                .and_then(|ch| self.mixer.audio_node(ch).map(|node| node.id)),
            TrackKind::Master => Some(ChannelId::Master),
            TrackKind::Folder => None,
        }
    }

    /// 把 MIDI 轨解链回自己的 LC。外部插件由每轨实例承接，不复制乐器自动化。
    pub fn link_track_to_own_lc(&mut self, track_idx: usize) -> bool {
        let Some(track) = self.data.model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Midi || track.lc.is_some() {
            return false;
        }
        let channel = track.global_channel();
        if self.mixer.racks.get(channel as usize).is_none() {
            return false;
        }
        let source = self
            .mixer
            .ap_node(channel)
            .cloned()
            .unwrap_or_else(yinhe_mixer::MixerNode::master);
        let id = self.mixer.add_track_lc(track_idx as u16);
        if let Some(node) = self.mixer.track_node_mut(id) {
            node.strip = source.strip;
            node.inserts = source.inserts.clone();
            node.output = source.output;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);
            let track = Arc::make_mut(&mut model.tracks[track_idx]);
            track.set_audio_target(TrackAudioTarget::OwnLc {
                lc: id,
                rack: Some(RackId::new(u16::from(channel))),
            });
        }
        if let Some(rack) = self.mixer.racks.get_mut(channel as usize) {
            rack.output_mode = RackOutputMode::PerTrack;
        }
        self.ensure_rack_track_lcs();
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// 新建 MIDI 轨的 C 默认：如果它落在一个全新的 MIDI 通道上，且 Rack
    /// 没有外部插件乐器，就把该 Rack 切到 `PerTrack`，让这条轨拥有自己的
    /// LC（内置音源按轨拆分音频）。
    ///
    /// 已有轨的通道不自动拆分：保持原 AP/共享关系，新轨加入该通道。
    /// 外部插件 Rack 同样支持自动 OwnLc（每轨一个实例）。
    pub fn default_own_lc_for_new_midi_tracks(&mut self, first_new: usize) {
        let model = &self.data.model;
        let mut own_channels = std::collections::BTreeSet::new();
        for track in model.tracks.iter().skip(first_new) {
            if track.kind != TrackKind::Midi || track.lc.is_some() {
                continue;
            }
            let channel = track.global_channel();
            let older: Vec<&TrackData> = model.tracks[..first_new]
                .iter()
                .filter(|t| t.kind == TrackKind::Midi && t.global_channel() == channel)
                .map(|t| t.as_ref())
                .collect();
            let older_own = older.iter().any(|t| t.lc.is_some());
            let older_any = !older.is_empty();
            // 空通道：默认 OwnLc；已有 OwnLc 的通道：新轨继续 OwnLc；
            // 已有 AP 轨的通道：加入共享。
            if !older_any || older_own {
                own_channels.insert(channel);
            }
        }

        // 为新轨分配 LC，并同步显式 C 目标。
        let mut assigned = Vec::new();
        {
            let model = &self.data.model;
            for (idx, track) in model.tracks.iter().enumerate().skip(first_new) {
                if track.kind == TrackKind::Midi
                    && track.lc.is_none()
                    && own_channels.contains(&track.global_channel())
                {
                    assigned.push((idx, track.global_channel()));
                }
            }
        }
        for (idx, channel) in assigned {
            let source = self
                .mixer
                .ap_node(channel)
                .cloned()
                .unwrap_or_else(yinhe_mixer::MixerNode::master);
            let id = self.mixer.add_track_lc(idx as u16);
            if let Some(node) = self.mixer.track_node_mut(id) {
                node.strip = source.strip;
                node.inserts = source.inserts.clone();
                node.output = source.output;
            }
            let model = Arc::make_mut(&mut self.data.model);
            if let Some(track) = model.tracks.get_mut(idx) {
                let track = Arc::make_mut(track);
                track.set_audio_target(TrackAudioTarget::OwnLc {
                    lc: id,
                    rack: Some(RackId::new(u16::from(channel))),
                });
            }
        }
        for channel in own_channels {
            if let Some(rack) = self.mixer.racks.get_mut(channel as usize) {
                rack.output_mode = RackOutputMode::PerTrack;
            }
        }
        self.ensure_rack_track_lcs();
    }

    /// 空白工程默认把每条 MIDI 轨放进自己的 LC（每个通道只有一条轨时）。
    /// 已有工程加载不走这里，保留原来的 AP/共享关系。
    pub fn default_empty_project_own_lc(&mut self) {
        self.default_own_lc_for_new_midi_tracks(0);
    }

    /// 加载后修复 per-track Rack 的 LC：
    /// - `PerTrack` 但 Track 没有 `lc` → 分配；
    /// - Track 有 `lc` 但节点不存在 → 创建；
    /// - `Shared` 但 Track 还留着 `lc` → 清掉并删除节点。
    ///
    /// 返回是否有修复。
    pub fn ensure_rack_track_lcs(&mut self) -> bool {
        self.ensure_audio_targets();
        let model = Arc::make_mut(&mut self.data.model);
        let mut changed = false;
        for (track_idx, track) in model.tracks.iter_mut().enumerate() {
            if track.kind != TrackKind::Midi {
                continue;
            }
            let channel = track.global_channel();

            // 普通 C 目标：Ap 清掉 LC，OwnLc 保留/修复自己的 LC。
            match track.effective_audio_target() {
                Some(TrackAudioTarget::Ap { .. }) => {
                    let track = Arc::make_mut(track);
                    if let Some(id) = track.lc.take() {
                        self.mixer.remove_track_lc(id);
                        changed = true;
                    }
                }
                Some(TrackAudioTarget::OwnLc { lc: id, rack }) => {
                    if self.mixer.track_node(id).is_none() {
                        self.mixer.add_track_lc_with_source(
                            id,
                            yinhe_types::ChannelSource::Track(track_idx as u16),
                        );
                        changed = true;
                    }
                    let track = Arc::make_mut(track);
                    if track.lc != Some(id) {
                        track.lc = Some(id);
                        changed = true;
                    }
                    if track.audio_target.is_none() {
                        track.set_audio_target(TrackAudioTarget::OwnLc { lc: id, rack });
                        changed = true;
                    }
                }
                None => {
                    // 防御：MIDI 轨必须有目标。按当前 lc 推导一次并写入。
                    let target = if let Some(lc) = track.lc {
                        TrackAudioTarget::OwnLc {
                            lc,
                            rack: Some(RackId::new(u16::from(channel))),
                        }
                    } else {
                        TrackAudioTarget::Ap {
                            ap: ApGroupId::new(u16::from(channel)),
                        }
                    };
                    Arc::make_mut(track).set_audio_target(target);
                    changed = true;
                }
            }
        }
        self.ensure_aps();
        if changed {
            self.data.bump_revision();
            self.mark_mixer_dirty();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_track_mode_assigns_and_releases_lcs() {
        let mut doc = Document::empty();
        // Track 1 是 A01（索引 0 是 Master）。C 空白工程默认已是 PerTrack，
        // 先切回 Shared 验证再切回 PerTrack 的分配/释放。
        assert_eq!(doc.data.model.tracks[1].global_channel(), 0);
        assert!(doc.set_rack_output_mode(0, RackOutputMode::Shared));
        assert!(doc.data.model.tracks[1].lc.is_none());
        assert!(doc.set_rack_output_mode(0, RackOutputMode::PerTrack));

        let lc = doc.data.model.tracks[1].lc.expect("per-track lc");
        assert!(doc.mixer.track_node(lc).is_some());
        assert_eq!(doc.rack_output_mode(0), RackOutputMode::PerTrack);

        assert!(doc.set_rack_output_mode(0, RackOutputMode::Shared));
        assert!(doc.data.model.tracks[1].lc.is_none());
        assert!(doc.mixer.track_node(lc).is_none());
    }

    /// 方案 C：外部插件同样支持 PerTrack（每轨一个实例，各写 home LC）。
    #[test]
    fn plugin_rack_supports_per_track_mode() {
        let mut doc = Document::empty();
        assert!(doc.set_rack_output_mode(0, RackOutputMode::Shared));
        doc.mixer.racks[0]
            .instruments
            .push(yinhe_mixer::InsertRef::default());
        assert!(doc.set_rack_output_mode(0, RackOutputMode::PerTrack));
        assert_eq!(doc.rack_output_mode(0), RackOutputMode::PerTrack);
        assert!(doc.data.model.tracks[1].lc.is_some());
    }

    #[test]
    fn ap_and_own_lc_can_coexist_on_same_midi_channel() {
        use crate::document::track_ops::NewTrackSpec;
        let mut doc = Document::empty();
        let specs = vec![NewTrackSpec {
            kind: TrackKind::Midi,
            port: 0,
            channel: 0,
            audio_channel: None,
        }];
        assert!(doc.add_tracks_batch(&specs).is_some());
        let idx = doc.data.model.tracks.len() - 1;
        let own_lc = doc.data.model.tracks[idx].lc.expect("own lc");

        assert!(doc.link_track_to_ap(1, ApGroupId::new(0)));
        assert!(doc.data.model.tracks[1].lc.is_none());
        assert!(doc.mixer.ap_for_midi(0).is_some());
        assert_eq!(
            doc.data.model.tracks[idx].lc,
            Some(own_lc),
            "同通道的另一条 OwnLc 轨不能被 AP 链接影响"
        );

        assert!(doc.link_track_to_own_lc(1));
        assert!(doc.data.model.tracks[1].lc.is_some());
        assert!(doc.mixer.ap_for_midi(0).is_none());
    }

    #[test]
    fn ensure_repairs_missing_nodes_and_clears_shared_lcs() {
        let mut doc = Document::empty();
        let lc = doc.data.model.tracks[1].lc.expect("empty project own lc");
        assert!(doc.mixer.track_node(lc).is_some());

        // 手工删节点，加载修复应重建。
        doc.mixer.remove_track_lc(lc);
        assert!(doc.ensure_rack_track_lcs());
        assert!(doc.mixer.track_node(lc).is_some());
    }
}
