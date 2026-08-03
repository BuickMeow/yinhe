//! Channel layout: maps source MIDI channels (0..256) to compacted xsynth channel indices.
//!
//! 统一了原来分散在 `spawn::channels_for_model`（算 active_mask）和
//! `AudioEngine::with_parallelism`（算 channel_map）的两套独立扫描逻辑。
//!
//! `ChannelLayout` 在 `AudioEngine` 创建时一次性定型，生命周期内不可变。
//! 若 model 结构变化（增减音轨、改 channel/port），必须 teardown + 重建引擎。

use yinhe_core::YinModel;

/// 不可变的通道布局，`AudioEngine` 创建时定型。
#[derive(Clone)]
pub struct ChannelLayout {
    /// `active_mask[i] == true` 表示源通道 `i` 被某条音轨使用（存在即激活）。
    /// 长度 = `num_channels`，超出部分视为未激活。
    active_mask: Vec<bool>,
    /// `channel_map[src] = dense`（激活）或 `u32::MAX`（未激活）。
    /// `dense` 是 xsynth `ChannelGroup` 压缩后的通道索引。
    channel_map: Box<[u32; 256]>,
    /// `active_mask` 覆盖的源通道数（= `active_mask.len()`）。
    num_channels: u32,
    /// 激活通道数 = xsynth `ChannelGroup` 的通道数。
    compacted_channels: u32,
}

impl ChannelLayout {
    /// 分析 `YinModel` 构建通道布局。
    ///
    /// 源通道"激活"条件：存在音轨使用该通道（`TrackData::global_channel`）。
    /// 音轨存在即激活——空音轨的通道也随时可用（首音符预览/播放立即有声），
    /// 不再按音符/CC 数量推断。成本 O(tracks)，与音符总数无关。
    pub fn from_model(model: &YinModel) -> Self {
        let mut ch_active = [false; 256];

        for track in model.tracks.iter() {
            let ch = track.global_channel() as usize;
            if ch < 256 {
                ch_active[ch] = true;
            }
        }

        let max_active_ch = ch_active.iter().rposition(|&c| c).unwrap_or(0);
        let num_channels = (max_active_ch + 1).max(1) as u32;

        let active_mask: Vec<bool> = ch_active[..num_channels as usize].to_vec();

        Self::from_mask(active_mask)
    }

    /// 从 `active_mask` 构建压缩后的 `channel_map`。
    pub fn from_mask(active_mask: Vec<bool>) -> Self {
        let mut channel_map = Box::new([u32::MAX; 256]);
        let mut next_dense: u32 = 0;
        for (src, &alive) in active_mask.iter().enumerate().take(256) {
            if alive {
                channel_map[src] = next_dense;
                next_dense += 1;
            }
        }
        let compacted_channels = next_dense.max(1);
        let num_channels = active_mask.len() as u32;
        Self {
            active_mask,
            channel_map,
            num_channels,
            compacted_channels,
        }
    }

    pub fn active_mask(&self) -> &[bool] {
        &self.active_mask
    }

    pub fn channel_map(&self) -> &[u32; 256] {
        &self.channel_map
    }

    pub fn num_channels(&self) -> u32 {
        self.num_channels
    }

    pub fn compacted_channels(&self) -> u32 {
        self.compacted_channels
    }

    /// 源通道 `ch` 是否激活。
    #[inline]
    pub fn is_active(&self, ch: usize) -> bool {
        self.active_mask.get(ch).copied().unwrap_or(false)
    }

    /// 源通道 `ch` 的 dense 索引，未激活返回 `u32::MAX`。
    #[inline]
    pub fn dense_for(&self, ch: usize) -> u32 {
        self.channel_map.get(ch).copied().unwrap_or(u32::MAX)
    }

    /// 返回 port 下所有激活通道的 dense 索引列表。
    pub fn dense_channels_for_port(&self, port: u8) -> Vec<u32> {
        let base_src = (port as u32 * 16) as usize;
        let end_src = (base_src + 16).min(256);
        let mut dense_channels: Vec<u32> = Vec::with_capacity(16);
        for src in base_src..end_src {
            if self.is_active(src) {
                let dense = self.channel_map[src];
                if dense != u32::MAX {
                    dense_channels.push(dense);
                }
            }
        }
        dense_channels
    }

    /// 检测当前 layout 与 model 的 tracks 是否在激活状态上有差异。
    ///
    /// 用于音频引擎在编辑后判断是否需要 teardown + 重建：`ChannelLayout`
    /// 创建后不可变，只有激活状态翻转才必须重建。
    ///
    /// 激活语义与 `from_model` 完全对齐：`active(ch) = ch 上有音轨`。
    /// 音轨增删/改 port 或 channel → 翻转 → 重建；音符增删不改变激活状态，
    /// 走便宜的 `UpdateNotes` 路径即可。成本 O(tracks)，与音符总数无关。
    pub fn differs_from_model(&self, model: &YinModel) -> bool {
        let mut now_active = [false; 256];
        for track in model.tracks.iter() {
            let ch = track.global_channel() as usize;
            if ch < 256 {
                now_active[ch] = true;
            }
        }
        for (ch, &now) in now_active.iter().enumerate() {
            if self.is_active(ch) != now {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_core::{ConductorData, NoteEvent, ProjectMeta, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
        let mut t = TrackData::new(0, first_ch);
        t.name = "Track 1".into();
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            notes
                .into_iter()
                .map(|(key, start, end, vel, _ch)| NoteEvent {
                    start_tick: start,
                    end_tick: end,
                    key,
                    velocity: vel,
                    id: 0,
                })
                .collect(),
        ];
        let meta = ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta,
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        model
    }

    fn make_model_3_tracks() -> YinModel {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let mk = |ch: u8, _key: u8| {
            let t = TrackData::new(0, ch);
            Arc::new(t)
        };
        let meta = ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        };
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
                id: 0,
            }],
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 64,
                velocity: 100,
                id: 0,
            }],
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 67,
                velocity: 100,
                id: 0,
            }],
        ];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![mk(0, 60), mk(1, 64), mk(9, 67)],
            meta,
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        model
    }

    #[test]
    fn from_model_basic() {
        let model = make_model_3_tracks();
        let layout = ChannelLayout::from_model(&model);
        assert_eq!(layout.num_channels(), 10);
        assert!(layout.is_active(0));
        assert!(layout.is_active(1));
        assert!(layout.is_active(9));
        assert!(!layout.is_active(2));
    }

    #[test]
    fn from_model_multi_port() {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let t1 = TrackData::new(0, 0);
        let t2 = TrackData::new(1, 0);
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
                id: 0,
            }],
            vec![NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
                id: 0,
            }],
        ];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t1), Arc::new(t2)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        let layout = ChannelLayout::from_model(&model);
        assert_eq!(layout.num_channels(), 17);
        assert!(layout.is_active(0));
        assert!(layout.is_active(16));
        assert!(!layout.is_active(15));
    }

    #[test]
    fn from_model_empty_track_activates_channel() {
        // 音轨存在即激活：即使没有任何音符（vel 0/1 或空音轨），通道也随时可用。
        let model = make_model_with_notes(vec![(60, 0, 480, 0, 0)]);
        let layout = ChannelLayout::from_model(&model);
        assert!(layout.is_active(0));
    }

    #[test]
    fn from_model_track_activates_channel_regardless_of_notes() {
        // 只有 automation 的音轨也能激活通道（音轨存在即激活，与音符/CC 无关）。
        let conductor = ConductorData::default();
        let mut t = TrackData::new(0, 5);
        t.automation_lanes = vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 100.0,
                shape: SegmentShape::Step,
            }],
        }];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        let layout = ChannelLayout::from_model(&model);
        assert_eq!(layout.num_channels(), 6);
        assert!(layout.is_active(5));
    }

    #[test]
    fn from_model_empty() {
        let model = YinModel::default();
        let layout = ChannelLayout::from_model(&model);
        assert_eq!(layout.num_channels(), 1);
        assert!(layout.active_mask().iter().all(|&b| !b));
        assert_eq!(layout.compacted_channels(), 1);
        // 空布局：所有源通道都映射到 u32::MAX
        assert_eq!(layout.dense_for(0), u32::MAX);
    }

    #[test]
    fn channel_map_inactive_channel() {
        let mut mask = vec![false; 16];
        mask[5] = true;
        let layout = ChannelLayout::from_mask(mask);
        assert_eq!(layout.dense_for(5), 0);
        assert_eq!(layout.dense_for(0), u32::MAX);
    }

    #[test]
    fn channel_map_multiple_active() {
        let mut mask = vec![false; 256];
        mask[0] = true;
        mask[2] = true;
        mask[10] = true;
        let layout = ChannelLayout::from_mask(mask);
        assert_eq!(layout.dense_for(0), 0);
        assert_eq!(layout.dense_for(1), u32::MAX);
        assert_eq!(layout.dense_for(2), 1);
        assert_eq!(layout.dense_for(10), 2);
    }

    #[test]
    fn dense_channels_for_port_collects_active() {
        let mut mask = vec![false; 32];
        mask[0] = true; // port 0, ch 0
        mask[5] = true; // port 0, ch 5
        mask[16] = true; // port 1, ch 0
        let layout = ChannelLayout::from_mask(mask);
        let port0 = layout.dense_channels_for_port(0);
        assert_eq!(port0, vec![0, 1]); // dense 0 = src 0, dense 1 = src 5
        let port1 = layout.dense_channels_for_port(1);
        assert_eq!(port1, vec![2]); // dense 2 = src 16
    }

    /// 回归测试：通道激活完全由音轨决定。空 model（无音轨）→ 全 false；
    /// 加音轨后重建 → 该音轨的通道被激活，空音轨也能立即发声。
    #[test]
    fn empty_model_then_track_rebuild_layout() {
        // 1. 空 model → 全 false
        let empty = YinModel::default();
        let layout_empty = ChannelLayout::from_model(&empty);
        assert!(!layout_empty.is_active(0));

        // 2. 加音轨（ch 0，带音符）后重建 → 通道 0 激活
        let with_track = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout_with = ChannelLayout::from_model(&with_track);
        assert!(layout_with.is_active(0));
        assert_eq!(layout_with.dense_for(0), 0);
        assert_eq!(layout_with.compacted_channels(), 1);
    }

    // -----------------------------------------------------------------------
    // differs_from_model 测试：flip 检测的核心逻辑
    // -----------------------------------------------------------------------
    // 激活完全由音轨决定：
    // - 加音轨/改音轨 channel → 0→1 翻转 → differs = true → teardown
    // - 删音轨 → 1→0 翻转 → differs = true → teardown
    // - 音符增删 → 激活状态不变 → differs = false → 走 UpdateNotes

    #[test]
    fn differs_from_model_no_flip_on_note_edits() {
        // 音符增删不改变激活状态：layout 与 model 的音轨集合一致 → 无翻转
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);
        assert!(!layout.differs_from_model(&model), "同 model 无翻转");
    }

    #[test]
    fn differs_from_model_flip_when_track_added() {
        // layout: ch 0 激活；model 新增 ch 1 音轨 → 0→1 翻转
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);

        let mut extended = model.clone();
        extended.tracks.push(Arc::new(TrackData::new(0, 1)));
        assert!(layout.differs_from_model(&extended), "ch 1 0→1 翻转");
    }

    #[test]
    fn differs_from_model_flip_when_track_removed() {
        // layout: ch 0 激活；model 删掉唯一音轨 → 1→0 翻转
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);

        let mut reduced = model.clone();
        reduced.tracks.clear();
        assert!(layout.differs_from_model(&reduced), "ch 0 1→0 翻转");
    }

    #[test]
    fn differs_from_model_flip_when_track_changes_channel() {
        // 音轨从 ch 0 改到 ch 1 → 0→1 翻转
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);

        let mut moved = model.clone();
        let t = Arc::make_mut(&mut moved.tracks[0]);
        t.channel = 1;
        assert!(layout.differs_from_model(&moved), "ch 0→1 翻转");
    }

    #[test]
    fn differs_from_model_multi_port_flip() {
        // layout: ch 0 (port 0) 和 ch 16 (port 1) 激活；model 新增 port 2 音轨
        let conductor = ConductorData::default();
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        }]];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(1, 0)),
            ],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();
        let layout = ChannelLayout::from_model(&model);
        assert!(layout.is_active(0));
        assert!(layout.is_active(16));

        let mut extended = model.clone();
        extended.tracks.push(Arc::new(TrackData::new(2, 0)));
        assert!(layout.differs_from_model(&extended), "多 port 翻转");
    }

    #[test]
    fn differs_from_model_all_inactive() {
        // layout: 全 false（空 model）；model 也无音轨 → 无翻转
        let empty = YinModel::default();
        let layout = ChannelLayout::from_model(&empty);
        assert!(!layout.differs_from_model(&empty), "全未激活，无翻转");
    }

    /// 集成测试：完整复现 bug 场景——空工程写第一个音符必须立即有声。
    ///
    /// 场景：空 model spawn 引擎（无音轨）→ 加音轨（即使还没有音符）→
    /// `differs_from_model` 报告翻转 → teardown；重建后通道已激活，
    /// 再写第一个音符无需任何重建即可发声。
    #[test]
    fn differs_from_model_detects_first_track_activation() {
        // 1. 空 model → layout 全 false
        let empty = YinModel::default();
        let layout = ChannelLayout::from_model(&empty);

        // 2. 加音轨（ch 0，无音符）→ 0→1 翻转
        let mut with_track = empty.clone();
        with_track.tracks.push(Arc::new(TrackData::new(0, 0)));
        assert!(layout.differs_from_model(&with_track), "ch 0 0→1 翻转");

        // 3. 重建 layout → 与 model 一致，不再翻转；空音轨的通道已激活
        let new_layout = ChannelLayout::from_model(&with_track);
        assert!(
            !new_layout.differs_from_model(&with_track),
            "新 layout 一致"
        );
        assert!(new_layout.is_active(0), "空音轨通道已激活");
    }
}
