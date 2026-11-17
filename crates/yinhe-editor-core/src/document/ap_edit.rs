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
    fn per_track_lc_and_audio_tracks_do_not_need_ap() {
        let tracks = vec![midi(0, 0, true), midi(0, 1, false), audio()];
        let needed = needed_aps(&tracks);
        assert_eq!(needed.into_iter().collect::<Vec<_>>(), vec![1]);
    }
}
