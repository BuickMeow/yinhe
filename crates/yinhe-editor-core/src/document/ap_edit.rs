//! AP 编组与 Track 的同步。
//!
//! 方案 C 里 AP 是显式的共享 MIDI 编组。迁移期先按 Track 的 `lc` 状态推导：
//! - MIDI 轨 `lc == None`：与同 MIDI 通道的其他共享轨一起构成 AP；
//! - MIDI 轨 `lc == Some(..)`：OwnLc，不参与 AP；
//! - 音频轨 / Folder / Master：不参与 AP。
//!
//! AP 表由 `MixerParams.aps` 持有；过渡期它的 `rack/channel` 指向当前共享
//! `racks` / `channels`，所以音频行为不变。

use std::collections::BTreeSet;
use std::sync::Arc;

use yinhe_core::{TrackAudioTarget, TrackData};

use crate::document::Document;

/// 需要 AP 的 MIDI 通道（global = port * 16 + channel）。
///
/// 同一通道只要有一条共享轨（`lc == None`）就需要 AP；全部 OwnLc 时不需要。
pub(super) fn needed_aps(tracks: &[Arc<TrackData>]) -> BTreeSet<u8> {
    tracks
        .iter()
        .filter_map(|track| match track.effective_audio_target() {
            Some(TrackAudioTarget::Ap { ap }) => Some(ap.0 as u8),
            _ => None,
        })
        .collect()
}

impl Document {
    /// 把旧字段（`lc` / `audio_channel`）固定成显式 C 目标。
    ///
    /// - 旧 MIDI 轨有 `lc` -> OwnLc，否则 -> Ap；
    /// - 音频轨 -> OwnLc（audio LC），Folder/Master -> None。
    ///
    /// 只填充 `None` 的轨；已有目标不覆盖。
    pub fn ensure_audio_targets(&mut self) {
        let model = std::sync::Arc::make_mut(&mut self.data.model);
        for track in &mut model.tracks {
            if track.audio_target.is_some() {
                continue;
            }
            if let Some(target) = track.effective_audio_target() {
                std::sync::Arc::make_mut(track).set_audio_target(target);
            }
        }
    }
}

impl Document {
    /// 旧工程迁移：共用 home LC 的音频轨拆成各自唯一的 LC，并用 SEND 汇总回去。
    ///
    /// 旧模型允许多条音频轨共用同一 `audio_channel`（共享一条音频通道条）。方案 C
    /// 要求每条 Track 的 home LC 唯一。加载时按通道分组：
    /// - 第一条轨保留原 `audio_channel`（原听感的汇总 LC）；
    /// - 其余轨分配新的空闲音频通道，并加一条 0 dB 的 post-fader SEND 到汇总 LC。
    ///
    /// 这样汇总 LC 的推子/insert 仍然作用于所有音轨之和，听感不变。只处理
    /// `audio_target` 尚未显式设定的旧轨；C 新工程没有重复 home LC，是 no-op。
    pub fn migrate_shared_audio_home_lcs(&mut self) -> bool {
        use std::collections::{HashMap, HashSet};

        use yinhe_types::{ChannelId, LogicalChannelId, MIDI_CHANNEL_COUNT};

        let mut groups: HashMap<u16, Vec<usize>> = HashMap::new();
        for (i, track) in self.data.model.tracks.iter().enumerate() {
            if track.kind == yinhe_core::TrackKind::Audio
                && track.audio_target.is_none()
                && let Some(ch) = track.audio_channel
            {
                groups.entry(ch).or_default().push(i);
            }
        }
        if groups.values().all(|tracks| tracks.len() <= 1) {
            return false;
        }

        let mut used: HashSet<u16> = groups.keys().copied().collect();
        let mut next_ch = used
            .iter()
            .copied()
            .max()
            .map_or(1, |c| c.saturating_add(1));
        let mut sends: Vec<(LogicalChannelId, LogicalChannelId)> = Vec::new();

        for (ch, tracks) in &groups {
            if tracks.len() <= 1 {
                continue;
            }
            let first_lc = LogicalChannelId(MIDI_CHANNEL_COUNT as u32 + u32::from(*ch));
            self.mixer.ensure_audio_lc_node(*ch);
            for &idx in &tracks[1..] {
                while used.contains(&next_ch) {
                    next_ch = next_ch.saturating_add(1);
                }
                let new_ch = next_ch;
                used.insert(new_ch);
                next_ch = next_ch.saturating_add(1);
                let new_lc = LogicalChannelId(MIDI_CHANNEL_COUNT as u32 + u32::from(new_ch));
                self.mixer.ensure_audio_lc_node(new_ch);

                let model = std::sync::Arc::make_mut(&mut self.data.model);
                let track = std::sync::Arc::make_mut(&mut model.tracks[idx]);
                track.audio_channel = Some(new_ch);
                track.set_audio_target(TrackAudioTarget::OwnLc {
                    lc: new_lc,
                    rack: None,
                });
                sends.push((new_lc, first_lc));
            }
        }
        for (source, target) in sends {
            self.mixer
                .set_channel_send(ChannelId::Lc(source), ChannelId::Lc(target), 1.0, false);
        }
        self.mixer.ensure_len();
        self.data.bump_revision();
        true
    }

    /// 按当前 Track 状态同步 AP 表。返回是否有变化。
    ///
    /// 只做结构同步，不 bump revision / mixer dirty；调用方在用户操作路径上
    /// 自行标记。加载时由 `ensure_rack_track_lcs` 触发。
    pub fn ensure_aps(&mut self) -> bool {
        let needed = needed_aps(&self.data.model.tracks);
        let before = self.mixer.aps.len();
        self.mixer
            .aps
            .retain(|ap| needed.contains(&ap.global_midi()));
        let mut changed = self.mixer.aps.len() != before;

        for global in needed {
            if self.mixer.ensure_ap(global) {
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::TrackKind;

    fn midi(port: u8, channel: u8, own_lc: bool) -> Arc<TrackData> {
        let mut track = TrackData::new(port, channel);
        if own_lc {
            track.lc = Some(yinhe_types::LogicalChannelId(1_000_000));
        }
        Arc::new(track)
    }

    fn audio() -> Arc<TrackData> {
        let mut track = TrackData::new(0, 0);
        track.kind = yinhe_core::TrackKind::Audio;
        track.audio_channel = Some(0);
        Arc::new(track)
    }

    #[test]
    fn shared_tracks_need_one_ap_per_midi_channel() {
        let tracks = vec![midi(0, 0, false), midi(0, 0, false), midi(0, 1, false)];
        let needed = needed_aps(&tracks);
        assert_eq!(needed.into_iter().collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn empty_document_creates_own_lcs_not_aps() {
        use crate::document::Document;
        let doc = Document::empty();
        assert!(doc.mixer.aps.is_empty());
        assert!(
            doc.data
                .model
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Midi)
                .all(|t| t.lc.is_some()),
            "C 空白工程的 MIDI 轨应默认拥有自己的 LC"
        );
    }

    #[test]
    fn shared_audio_home_lcs_split_into_unique_lcs_with_send() {
        use crate::document::Document;
        use yinhe_types::{ChannelId, LogicalChannelId, MIDI_CHANNEL_COUNT};

        let mut doc = Document::empty();
        // 两条音频轨共用 audio_channel 0（旧工程共享 home LC）。
        {
            let model = std::sync::Arc::make_mut(&mut doc.data.model);
            for _ in 0..2 {
                let mut t = TrackData::new(0, 0);
                t.kind = TrackKind::Audio;
                t.audio_channel = Some(0);
                model.tracks.push(std::sync::Arc::new(t));
            }
        }
        assert!(doc.migrate_shared_audio_home_lcs());
        let model = &doc.data.model;
        let audio_channels: Vec<u16> = model
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .filter_map(|t| t.audio_channel)
            .collect();
        assert_eq!(audio_channels.len(), 2);
        assert_ne!(audio_channels[0], audio_channels[1], "home LC 必须唯一");

        let first_lc = LogicalChannelId(MIDI_CHANNEL_COUNT as u32);
        let second_lc =
            LogicalChannelId(MIDI_CHANNEL_COUNT as u32 + u16::from(audio_channels[1]) as u32);
        let send = doc
            .mixer
            .channel_send(ChannelId::Lc(second_lc), ChannelId::Lc(first_lc));
        assert!(send.is_some(), "拆出的轨应 SEND 回汇总 LC");
        assert!((send.expect("send").amount - 1.0).abs() < 1e-6);

        // 再次迁移应为 no-op（已是唯一 home LC）。
        assert!(!doc.migrate_shared_audio_home_lcs());
    }

    #[test]
    fn per_track_lc_and_audio_tracks_do_not_need_ap() {
        let tracks = vec![midi(0, 0, true), midi(0, 1, false), audio()];
        let needed = needed_aps(&tracks);
        assert_eq!(needed.into_iter().collect::<Vec<_>>(), vec![1]);
    }
}
