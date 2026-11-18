//! MIX 图层面板的通道分组。
//!
//! 行单位 = **音频目标**（与混音台条一致）：
//! - AP 轨按 MIDI 通道归组（A01…），组内是该通道所有 AP 轨；
//! - OwnLc 轨一轨一通道，独立成行，显示轨道号（001…）；
//! - 音频轨按音频通道归组（Audio-XX…），追加在 MIDI 行之后；
//! - Conductor（Master）轨不归入任何通道。
//!
//! 原通道 AR 视图已删除；本分组只服务右侧图层面板。

use std::collections::BTreeMap;

use yinhe_core::{TrackAudioTarget, TrackKind, YinModel};

/// MIX 图层行的命名空间。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum CaChannel {
    /// AP 的 MIDI 源通道（global channel 0..=255，A01 = 0）。
    Midi(u8),
    /// 音频通道（与混音台 Audio-XX 一致）。
    Audio(u16),
    /// OwnLc 单轨行（值 = YinModel.tracks 下标；显示为轨道号 001…）。
    Track(u16),
}

/// 一条分组行的组成。
#[derive(Clone, Debug)]
pub(crate) struct CaChannelGroup {
    pub channel: CaChannel,
    /// 该行包含的轨道索引（YinModel.tracks 位置，升序）。
    pub tracks: Vec<u16>,
}

impl CaChannelGroup {
    /// 色带 = 首条轨道的显示色（与混音台色带同源）。
    pub fn color(&self, track_colors: &[[f32; 4]]) -> [f32; 4] {
        self.tracks
            .first()
            .and_then(|&t| track_colors.get(t as usize).copied())
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR)
    }
}

/// 全部分组行：MIDI/AP 与 OwnLc 行按轨道出现顺序排列，音频行按通道号升序追加。
#[derive(Default)]
pub(crate) struct CaGroups {
    pub rows: Vec<CaChannelGroup>,
}

impl CaGroups {
    /// 从模型构建。`conductor_idx` 指定的 Master 轨不归入任何通道。
    pub fn from_model(model: &YinModel, conductor_idx: Option<u16>) -> Self {
        let mut rows: Vec<CaChannelGroup> = Vec::new();
        let mut audio_map: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
        for (i, track) in model.tracks.iter().enumerate() {
            let i = i as u16;
            if Some(i) == conductor_idx {
                continue;
            }
            match track.kind {
                TrackKind::Midi => {
                    let ch = track
                        .midi_target_channel()
                        .unwrap_or_else(|| track.global_channel());
                    match track.effective_audio_target() {
                        // AP（含迁移期默认）→ 按通道归组：该通道首条 AP 轨处成组，
                        // 后到的 AP 轨追加进组。
                        Some(TrackAudioTarget::Ap { .. }) | None => {
                            if let Some(g) =
                                rows.iter_mut().find(|g| g.channel == CaChannel::Midi(ch))
                            {
                                g.tracks.push(i);
                            } else {
                                rows.push(CaChannelGroup {
                                    channel: CaChannel::Midi(ch),
                                    tracks: vec![i],
                                });
                            }
                        }
                        // OwnLc：一轨一通道 → 独立行（显示轨道号）。
                        Some(TrackAudioTarget::OwnLc { .. }) => rows.push(CaChannelGroup {
                            channel: CaChannel::Track(i),
                            tracks: vec![i],
                        }),
                    }
                }
                // 音频轨统一由 `audio_target_channel` 兜底到 0；直通 Master 不列行。
                TrackKind::Audio => {
                    audio_map
                        .entry(track.audio_target_channel().unwrap_or(0))
                        .or_default()
                        .push(i);
                }
                TrackKind::Folder | TrackKind::Master => {}
            }
        }
        rows.extend(audio_map.into_iter().map(|(ch, tracks)| CaChannelGroup {
            channel: CaChannel::Audio(ch),
            tracks,
        }));
        Self { rows }
    }

    /// 全部分组行（MIDI/AP 与 OwnLc 按轨道序，音频行在后）。
    pub fn iter_all(&self) -> impl Iterator<Item = &CaChannelGroup> {
        self.rows.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::TrackData;

    fn midi_track(port: u8, channel: u8) -> TrackData {
        TrackData::new(port, channel)
    }

    fn own_lc_track(port: u8, channel: u8) -> TrackData {
        let mut t = TrackData::new(port, channel);
        let lc = yinhe_types::LogicalChannelId(7);
        t.lc = Some(lc);
        t.set_audio_target(TrackAudioTarget::OwnLc {
            lc,
            rack: Some(yinhe_types::RackId::new(u16::from(t.global_channel()))),
        });
        t
    }

    fn audio_track(ach: u16) -> TrackData {
        let mut t = TrackData::new(0, 0);
        t.kind = TrackKind::Audio;
        t.audio_channel = Some(ach);
        t
    }

    fn model_with(tracks: Vec<TrackData>) -> YinModel {
        let mut model = YinModel::new_empty_with_16_tracks();
        // 0 号轨为 Conductor；替换其余轨道。
        let mut all = vec![model.tracks[0].as_ref().clone()];
        all.extend(tracks);
        model.tracks = all.into_iter().map(std::sync::Arc::new).collect();
        model
    }

    /// AP 轨按通道归组；OwnLc 轨独立成行；音频行追加在后；Conductor 不入组。
    #[test]
    fn groups_by_audio_target_with_own_lc_rows() {
        let model = model_with(vec![
            midi_track(0, 0),   // 1: A01（AP，迁移期默认）
            own_lc_track(0, 1), // 2: OwnLc → 独立行
            audio_track(1),     // 3: Audio-02
            midi_track(0, 0),   // 4: A01 另一条 AP 轨 → 并入 A01 组
            own_lc_track(0, 2), // 5: OwnLc → 独立行
            audio_track(0),     // 6: Audio-01
        ]);
        let groups = CaGroups::from_model(&model, Some(0));
        let rows = &groups.rows;
        assert_eq!(rows.len(), 5, "A01 组 + 两条 OwnLc 行 + 两个音频组");
        assert_eq!(rows[0].channel, CaChannel::Midi(0));
        assert_eq!(rows[0].tracks, vec![1, 4], "A01 组含两条 AP 轨");
        assert_eq!(rows[1].channel, CaChannel::Track(2), "OwnLc 独立行");
        assert_eq!(rows[2].channel, CaChannel::Track(5));
        assert_eq!(rows[3].channel, CaChannel::Audio(0));
        assert_eq!(rows[3].tracks, vec![6]);
        assert_eq!(rows[4].channel, CaChannel::Audio(1));
        assert_eq!(rows[4].tracks, vec![3]);
        // Conductor（索引 0）不归组。
        assert!(groups.iter_all().all(|g| !g.tracks.contains(&0u16)));
    }

    /// 行序跟随轨道出现顺序：后到的 AP 轨并进首条 AP 轨位置的组。
    #[test]
    fn later_ap_track_joins_group_at_first_position() {
        let model = model_with(vec![
            own_lc_track(0, 0), // 1: OwnLc 行
            midi_track(0, 5),   // 2: A06 AP
            midi_track(0, 5),   // 3: A06 另一条 AP
        ]);
        let groups = CaGroups::from_model(&model, Some(0));
        assert_eq!(groups.rows[0].channel, CaChannel::Track(1));
        assert_eq!(groups.rows[1].channel, CaChannel::Midi(5));
        assert_eq!(groups.rows[1].tracks, vec![2, 3]);
    }

    /// 色带取首轨颜色；越界回退默认色。
    #[test]
    fn channel_color_uses_first_track() {
        let g = CaChannelGroup {
            channel: CaChannel::Midi(0),
            tracks: vec![1, 2],
        };
        let colors = vec![[0.9, 0.9, 0.9, 1.0], [0.1, 0.2, 0.3, 1.0]];
        // 首轨索引 = 1 → 取 colors[1]。
        assert_eq!(g.color(&colors), [0.1, 0.2, 0.3, 1.0]);
        let empty = CaChannelGroup {
            channel: CaChannel::Midi(0),
            tracks: vec![],
        };
        assert_eq!(empty.color(&colors), yinhe_core::DEFAULT_TRACK_COLOR);
    }
}
