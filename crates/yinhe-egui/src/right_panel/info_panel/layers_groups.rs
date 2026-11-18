//! MIX 图层面板的通道分组。
//!
//! MIX 模式的图层面板以「通道」为组织单位（而非轨道）：按实际混音通道
//! （MIDI = 端口/通道，音频 = 音频通道）归组，与混音台条一致。
//! Conductor（Master）轨单独成行，不归入任何通道。
//!
//! 只列出「active」通道（工程中至少有一条轨道使用的通道），与混音台一致。
//! 原通道 AR 视图已删除；本分组只服务右侧图层面板。

use std::collections::BTreeMap;

use yinhe_core::{TrackKind, YinModel};

/// 通道 AR 的通道命名空间。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum CaChannel {
    /// MIDI 源通道（global channel 0..=255，A01 = 0）。
    Midi(u8),
    /// 音频通道（与混音台 Audio-XX 一致）。
    Audio(u16),
}

/// 一条通道的组成。
#[derive(Clone, Debug)]
pub(crate) struct CaChannelGroup {
    pub channel: CaChannel,
    /// 该通道上的轨道索引（YinModel.tracks 位置，升序）。
    pub tracks: Vec<u16>,
}

impl CaChannelGroup {
    /// 通道色带 / 合并行音符颜色 = 首条轨道的显示色（与混音台色带同源）。
    pub fn color(&self, track_colors: &[[f32; 4]]) -> [f32; 4] {
        self.tracks
            .first()
            .and_then(|&t| track_colors.get(t as usize).copied())
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR)
    }
}

/// 全部 active 通道分组（MIDI 在前、音频在后，各自按通道号升序）。
#[derive(Default)]
pub(crate) struct CaGroups {
    pub midi: Vec<CaChannelGroup>,
    pub audio: Vec<CaChannelGroup>,
}

impl CaGroups {
    /// 从模型构建。`conductor_idx` 指定的 Master 轨不归入任何通道。
    pub fn from_model(model: &YinModel, conductor_idx: Option<u16>) -> Self {
        let mut midi_map: BTreeMap<u8, Vec<u16>> = BTreeMap::new();
        let mut audio_map: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
        for (i, track) in model.tracks.iter().enumerate() {
            let i = i as u16;
            if Some(i) == conductor_idx {
                continue;
            }
            match track.kind {
                // 按实际混音通道归组：显式改道的轨道跟随目标通道，
                // 否则回退到源端口/通道（或音频通道）。
                TrackKind::Midi => midi_map
                    .entry(
                        track
                            .midi_target_channel()
                            .unwrap_or_else(|| track.global_channel()),
                    )
                    .or_default()
                    .push(i),
                // 音频轨统一由 `audio_target_channel` 兜底到 0。
                TrackKind::Audio => audio_map
                    .entry(track.audio_target_channel().unwrap_or(0))
                    .or_default()
                    .push(i),
                TrackKind::Folder | TrackKind::Master => {}
            }
        }
        let midi = midi_map
            .into_iter()
            .map(|(ch, tracks)| CaChannelGroup {
                channel: CaChannel::Midi(ch),
                tracks,
            })
            .collect();
        let audio = audio_map
            .into_iter()
            .map(|(ch, tracks)| CaChannelGroup {
                channel: CaChannel::Audio(ch),
                tracks,
            })
            .collect();
        Self { midi, audio }
    }

    /// 全部 active 通道（MIDI 在前、音频在后），行序与之一致。
    pub fn iter_all(&self) -> impl Iterator<Item = &CaChannelGroup> {
        self.midi.iter().chain(self.audio.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::TrackData;

    fn midi_track(port: u8, channel: u8) -> TrackData {
        TrackData::new(port, channel)
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

    /// 同通道多轨归入一组；音频通道独立；通道号升序；Conductor 不入组。
    #[test]
    fn groups_by_channel_with_conductor_excluded() {
        let model = model_with(vec![
            midi_track(0, 0), // A01
            midi_track(0, 2), // A03
            audio_track(1),   // Audio-02
            midi_track(0, 0), // A01 另一轨
            audio_track(0),   // Audio-01
        ]);
        let groups = CaGroups::from_model(&model, Some(0));
        assert_eq!(groups.midi.len(), 2, "A01 / A03 两个 MIDI 通道");
        assert_eq!(groups.midi[0].channel, CaChannel::Midi(0));
        assert_eq!(groups.midi[0].tracks, vec![1, 4]);
        assert_eq!(groups.midi[1].channel, CaChannel::Midi(2));
        assert_eq!(groups.midi[1].tracks, vec![2]);
        assert_eq!(groups.audio.len(), 2, "Audio-01 / Audio-02 两个音频通道");
        assert_eq!(groups.audio[0].channel, CaChannel::Audio(0));
        assert_eq!(groups.audio[0].tracks, vec![5]);
        assert_eq!(groups.audio[1].channel, CaChannel::Audio(1));
        assert_eq!(groups.audio[1].tracks, vec![3]);
        // Conductor（索引 0）不归组。
        assert!(groups.iter_all().all(|g| !g.tracks.contains(&0u16)));
    }

    /// 通道色带取首轨颜色；越界回退默认色。
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
