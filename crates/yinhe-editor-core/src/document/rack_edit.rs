//! Rack 输出拓扑与每轨 LC 的文档级操作。
//!
//! Rack 默认是单输出：整个 Rack 一条 LC，所有使用它的 Track 共享。
//! 内置音源可以切到 `PerTrack`：每条 MIDI Track 分配一个稳定 LC，
//! 引擎按轨拆分音频后分别进入各自的 LC。
//!
//! 外部单输出插件（Rack 里有 `instruments`）不支持按轨拆分；

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
    /// 返回是否有变化。`PerTrack` 只允许内置音源（Rack 无插件乐器）；
    /// 外部单输出插件无法按轨拆分，直接拒绝。
    pub fn set_rack_output_mode(&mut self, channel: u8, mode: RackOutputMode) -> bool {
        let Some(rack) = self.mixer.racks.get(channel as usize) else {
            return false;
        };
        if rack.output_mode == mode {
            return false;
        }
        if mode == RackOutputMode::PerTrack && !rack.instruments.is_empty() {
            return false;
        }
        self.mixer.racks[channel as usize].output_mode = mode;
        let model = Arc::make_mut(&mut self.data.model);
        for track in model.tracks.iter_mut() {
            if track.kind != TrackKind::Midi || track.global_channel() != channel {
                continue;
            }
            match mode {
                RackOutputMode::PerTrack => {
                    if track.lc.is_none() {
                        // 复制共享 LC 的混音参数，避免切换模式后听感突变。
                        let source = self.mixer.midi_node(channel).cloned().unwrap_or_else(|| {
                            yinhe_mixer::MixerNode::logical_channel(yinhe_types::LogicalChannelId(
                                u32::from(channel),
                            ))
                        });
                        let id = self.mixer.add_track_lc();
                        if let Some(node) = self.mixer.track_node_mut(id) {
                            node.strip = source.strip;
                            node.inserts = source.inserts.clone();
                            node.sends = source.sends.clone();
                            node.output = source.output;
                        }
                        Arc::make_mut(track).lc = Some(id);
                    }
                    let lc = track.lc.expect("per-track lc");
                    Arc::make_mut(track).set_audio_target(TrackAudioTarget::OwnLc {
                        lc,
                        rack: Some(RackId::new(u16::from(channel))),
                    });
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

    /// 把 MIDI 轨设为 DirectMaster（`---`）：不创建 LC/AP 通道，音频直接进 Master。
    pub fn link_track_to_direct_master(&mut self, track_idx: usize) -> bool {
        let Some(track) = self.data.model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Midi {
            return false;
        }
        let channel = track.global_channel();
        if track.audio_target
            == Some(TrackAudioTarget::DirectMaster {
                rack: Some(RackId::new(u16::from(channel))),
            })
        {
            return false;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        track.set_audio_target(TrackAudioTarget::DirectMaster {
            rack: Some(RackId::new(u16::from(channel))),
        });
        self.ensure_rack_track_lcs();
        self.data.bump_revision();
        self.mark_mixer_dirty();
        true
    }

    /// Audio Track：直通 Master（`---`）。
    pub fn link_audio_track_to_direct_master(&mut self, track_idx: usize) -> bool {
        let Some(track) = self.data.model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Audio
            || matches!(
                track.effective_audio_target(),
                Some(TrackAudioTarget::DirectMaster { .. })
            )
        {
            return false;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        track.set_audio_target(TrackAudioTarget::DirectMaster { rack: None });
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
        let lc =
            yinhe_types::LogicalChannelId(yinhe_types::MIDI_CHANNEL_COUNT as u32 + u32::from(ch));
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

    /// 把 MIDI 轨解链回自己的 LC。外部单输出插件 Rack 不能按轨拆分，返回 false。
    pub fn link_track_to_own_lc(&mut self, track_idx: usize) -> bool {
        let Some(track) = self.data.model.tracks.get(track_idx) else {
            return false;
        };
        if track.kind != TrackKind::Midi || track.lc.is_some() {
            return false;
        }
        let channel = track.global_channel();
        let Some(rack) = self.mixer.racks.get(channel as usize) else {
            return false;
        };
        if !rack.instruments.is_empty() {
            return false;
        }
        let source = self.mixer.midi_node(channel).cloned().unwrap_or_else(|| {
            yinhe_mixer::MixerNode::logical_channel(yinhe_types::LogicalChannelId(u32::from(
                channel,
            )))
        });
        let id = self.mixer.add_track_lc();
        if let Some(node) = self.mixer.track_node_mut(id) {
            node.strip = source.strip;
            node.inserts = source.inserts.clone();
            node.sends = source.sends.clone();
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
    /// 外部插件 Rack 暂不支持自动 OwnLc（单输出插件无法按轨拆音频）。
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
            let rack_has_plugins = self
                .mixer
                .racks
                .get(channel as usize)
                .is_some_and(|rack| !rack.instruments.is_empty());
            // 空通道：默认 OwnLc；已有 OwnLc 的通道：新轨继续 OwnLc；
            // 已有 AP / 外部插件 Rack：加入共享。
            if !rack_has_plugins && (!older_any || older_own) {
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
            let source = self.mixer.midi_node(channel).cloned().unwrap_or_else(|| {
                yinhe_mixer::MixerNode::logical_channel(yinhe_types::LogicalChannelId(u32::from(
                    channel,
                )))
            });
            let id = self.mixer.add_track_lc();
            if let Some(node) = self.mixer.track_node_mut(id) {
                node.strip = source.strip;
                node.inserts = source.inserts.clone();
                node.sends = source.sends.clone();
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
        for track in model.tracks.iter_mut() {
            if track.kind != TrackKind::Midi {
                continue;
            }
            let channel = track.global_channel();

            // 普通 C 目标：Ap/DirectMaster 清掉 LC，OwnLc 保留/修复自己的 LC。
            match track.effective_audio_target() {
                Some(TrackAudioTarget::Ap { .. }) | Some(TrackAudioTarget::DirectMaster { .. }) => {
                    let track = Arc::make_mut(track);
                    if let Some(id) = track.lc.take() {
                        self.mixer.remove_track_lc(id);
                        changed = true;
                    }
                }
                Some(TrackAudioTarget::OwnLc { lc: id, rack }) => {
                    if self.mixer.track_node(id).is_none() {
                        self.mixer.add_track_lc_with_id(id);
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

    #[test]
    fn plugin_rack_rejects_per_track_mode() {
        let mut doc = Document::empty();
        assert!(doc.set_rack_output_mode(0, RackOutputMode::Shared));
        doc.mixer.racks[0]
            .instruments
            .push(yinhe_mixer::InsertRef::default());
        assert!(!doc.set_rack_output_mode(0, RackOutputMode::PerTrack));
        assert_eq!(doc.rack_output_mode(0), RackOutputMode::Shared);
        assert!(doc.data.model.tracks[1].lc.is_none());
    }

    #[test]
    fn audio_track_can_link_to_direct_master_and_back() {
        let mut doc = Document::empty();
        let mut track = TrackData::new(0, 0);
        track.kind = TrackKind::Audio;
        track.audio_channel = Some(2);
        let idx = doc.data.model.tracks.len();
        Arc::make_mut(&mut doc.data.model)
            .tracks
            .push(Arc::new(track));
        assert!(doc.link_audio_track_to_direct_master(idx));
        assert!(matches!(
            doc.data.model.tracks[idx].effective_audio_target(),
            Some(TrackAudioTarget::DirectMaster { .. })
        ));
        assert!(doc.link_audio_track_to_own_lc(idx));
        assert!(matches!(
            doc.data.model.tracks[idx].effective_audio_target(),
            Some(TrackAudioTarget::OwnLc { .. })
        ));
    }

    #[test]
    fn midi_track_can_link_to_direct_master() {
        let mut doc = Document::empty();
        assert!(doc.link_track_to_direct_master(1));
        let track = &doc.data.model.tracks[1];
        assert!(track.lc.is_none());
        assert!(matches!(
            track.effective_audio_target(),
            Some(TrackAudioTarget::DirectMaster { .. })
        ));
        assert!(doc.mixer.ap_for_midi(0).is_none());
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
