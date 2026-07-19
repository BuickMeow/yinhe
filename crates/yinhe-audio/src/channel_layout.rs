//! Channel layout: maps source MIDI channels (0..256) to compacted xsynth channel indices.
//!
//! 统一了原来分散在 `spawn::channels_for_model`（算 active_mask）和
//! `AudioEngine::with_parallelism`（算 channel_map）的两套独立扫描逻辑。
//!
//! `ChannelLayout` 在 `AudioEngine` 创建时一次性定型，生命周期内不可变。
//! 若 model 结构变化（增减音轨、改 channel/port），必须 teardown + 重建引擎。

use yinhe_core::YinModel;

use crate::spawn::track_global_channel;

/// 不可变的通道布局，`AudioEngine` 创建时定型。
#[derive(Clone)]
pub struct ChannelLayout {
    /// `active_mask[i] == true` 表示源通道 `i` 有可听音符或控制事件。
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
    /// 源通道"激活"条件：任意 `vel > 1` 的音符落在其上，或所属 track
    /// 有 automation lane / program change。
    pub fn from_model(model: &YinModel) -> Self {
        let mut ch_active = [0u32; 256];

        for bucket in model.notes.iter() {
            for n in bucket.iter() {
                if n.velocity > 1 {
                    let ch = track_global_channel(model, n.track as usize) as usize;
                    if ch < 256 {
                        ch_active[ch] = ch_active[ch].saturating_add(1);
                    }
                }
            }
        }

        for (track_idx, track) in model.tracks.iter().enumerate() {
            let ch = track_global_channel(model, track_idx) as usize;
            let has_ctrl = !track.automation_lanes.is_empty()
                || !track.program_change.is_empty();
            if has_ctrl && ch < 256 {
                ch_active[ch] = ch_active[ch].max(1);
            }
        }

        let max_active_ch = ch_active.iter().rposition(|&c| c > 0).unwrap_or(0);
        let num_channels = (max_active_ch + 1).max(1) as u32;

        let active_mask: Vec<bool> = ch_active[..num_channels as usize]
            .iter()
            .map(|&c| c > 0)
            .collect();

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

    /// 检测当前 layout 与给定 channel 计数是否在激活状态上有差异。
    ///
    /// 用于音频引擎在音符/automation 编辑后判断是否需要 teardown + 重建：
    /// `ChannelLayout` 创建后不可变，只有激活状态翻转（0→1 / 1→0）才必须重建。
    /// 若仅音符数变化但激活状态不变（如已激活 channel 加/删非末音符），
    /// 返回 false，调用方可走便宜的 `UpdateNotes` 路径。
    ///
    /// 激活语义与 `from_model` 完全对齐：
    /// `active(ch) = note_count[ch] > 0 || ctrl_count[ch] > 0`
    pub fn differs_from_counts(
        &self,
        note_counts: &[u32; 256],
        ctrl_counts: &[u32; 256],
    ) -> bool {
        for ch in 0..256 {
            let was_active = self.is_active(ch);
            let now_active = note_counts[ch] > 0 || ctrl_counts[ch] > 0;
            if was_active != now_active {
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
                events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
            },
            time_sig: Vec::new(),
        };
        let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
        let mut t = TrackData::new(0, first_ch);
        t.name = "Track 1".into();
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![notes
            .into_iter()
            .map(|(key, start, end, vel, _ch)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: vel,
                id: 0,
            })
            .collect()];
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
                events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
            },
            time_sig: Vec::new(),
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
            vec![NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 100, id: 0 }],
            vec![NoteEvent { start_tick: 0, end_tick: 480, key: 64, velocity: 100, id: 0 }],
            vec![NoteEvent { start_tick: 0, end_tick: 480, key: 67, velocity: 100, id: 0 }],
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
                events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
            },
            time_sig: Vec::new(),
        };
        let t1 = TrackData::new(0, 0);
        let t2 = TrackData::new(1, 0);
        let per_track_notes: Vec<Vec<NoteEvent>> = vec![
            vec![NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 100, id: 0 }],
            vec![NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 100, id: 0 }],
        ];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t1), Arc::new(t2)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
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
    fn from_model_skips_velocity_0_1() {
        let model = make_model_with_notes(vec![
            (60, 0, 480, 0, 0),
            (61, 0, 480, 1, 0),
            (62, 0, 480, 2, 0),
        ]);
        let layout = ChannelLayout::from_model(&model);
        assert!(layout.is_active(0));
    }

    #[test]
    fn from_model_cc_activates_channel() {
        let conductor = ConductorData::default();
        let mut t = TrackData::new(0, 5);
        t.automation_lanes = vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent { tick: 0, value: 100.0, shape: SegmentShape::Step }],
        }];
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta { ppq: 480, ..ProjectMeta::default() },
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
        mask[0] = true;  // port 0, ch 0
        mask[5] = true;  // port 0, ch 5
        mask[16] = true; // port 1, ch 0
        let layout = ChannelLayout::from_mask(mask);
        let port0 = layout.dense_channels_for_port(0);
        assert_eq!(port0, vec![0, 1]); // dense 0 = src 0, dense 1 = src 5
        let port1 = layout.dense_channels_for_port(1);
        assert_eq!(port1, vec![2]); // dense 2 = src 16
    }

    /// 回归测试：空 model 的 ChannelLayout 不应让任何通道激活。
    /// 这是"新建工程播放无声"bug 的核心：空 model → 全 false active_mask →
    /// 引擎创建时无激活通道 → 后续加音符无法 dispatch。
    /// 修复方案是 add_track/add_note 后 teardown + 重建引擎，让 from_model
    /// 重新计算 ChannelLayout。
    #[test]
    fn empty_model_then_notes_rebuild_layout() {
        // 1. 空 model → 全 false
        let empty = YinModel::default();
        let layout_empty = ChannelLayout::from_model(&empty);
        assert!(!layout_empty.is_active(0));

        // 2. 加音符后重建 → 通道 0 激活
        let with_notes = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout_with = ChannelLayout::from_model(&with_notes);
        assert!(layout_with.is_active(0));
        assert_eq!(layout_with.dense_for(0), 0);
        assert_eq!(layout_with.compacted_channels(), 1);
    }

    // -----------------------------------------------------------------------
    // differs_from_counts 测试：flip 检测的核心逻辑
    // -----------------------------------------------------------------------
    // 这是选项 Z 的关键：用 per-channel 计数器判断 ChannelLayout 是否需要重建。
    // - 加首 audible 音符 → 0→1 翻转 → differs = true → teardown
    // - 删末 audible 音符 → 1→0 翻转 → differs = true → teardown
    // - 已激活 channel 加/删非末音符 → 不翻转 → differs = false → 走 UpdateNotes

    #[test]
    fn differs_from_counts_no_flip_when_adding_non_first_note() {
        // layout: ch 0 已激活（有 1 个 audible 音符）
        // counts: ch 0 有 2 个 audible → 仍然激活，无翻转
        let mut mask = vec![false; 16];
        mask[0] = true;
        let layout = ChannelLayout::from_mask(mask);

        let mut notes = [0u32; 256];
        notes[0] = 2; // ch 0 有 2 个 audible
        let ctrls = [0u32; 256];

        assert!(!layout.differs_from_counts(&notes, &ctrls), "ch 0 仍然激活，无翻转");
    }

    #[test]
    fn differs_from_counts_flip_when_first_note_added() {
        // layout: ch 0 未激活（空 model 创建的）
        // counts: ch 0 有 1 个 audible → 0→1 翻转
        let mut mask = vec![false; 16];
        mask[0] = false;
        let layout = ChannelLayout::from_mask(mask);

        let mut notes = [0u32; 256];
        notes[0] = 1; // 首 audible 音符
        let ctrls = [0u32; 256];

        assert!(layout.differs_from_counts(&notes, &ctrls), "ch 0 0→1 翻转");
    }

    #[test]
    fn differs_from_counts_flip_when_last_note_removed() {
        // layout: ch 0 已激活
        // counts: ch 0 = 0 → 1→0 翻转
        let mut mask = vec![false; 16];
        mask[0] = true;
        let layout = ChannelLayout::from_mask(mask);

        let notes = [0u32; 256]; // ch 0 = 0（末音符被删）
        let ctrls = [0u32; 256];

        assert!(layout.differs_from_counts(&notes, &ctrls), "ch 0 1→0 翻转");
    }

    #[test]
    fn differs_from_counts_ctrl_only_channel() {
        // layout: ch 5 未激活
        // counts: ch 5 note=0 但 ctrl=1 → 0→1 翻转（automation 激活 channel）
        let mask = vec![false; 16];
        let layout = ChannelLayout::from_mask(mask);

        let notes = [0u32; 256];
        let mut ctrls = [0u32; 256];
        ctrls[5] = 1;

        assert!(layout.differs_from_counts(&notes, &ctrls), "ch 5 由 automation 激活");
    }

    #[test]
    fn differs_from_counts_multi_port_flip() {
        // layout: ch 0 (port 0) 和 ch 16 (port 1) 激活
        // counts: ch 0 仍激活，ch 16 失活，ch 32 (port 2) 新激活
        let mut mask = vec![false; 48];
        mask[0] = true;
        mask[16] = true;
        let layout = ChannelLayout::from_mask(mask);

        let mut notes = [0u32; 256];
        notes[0] = 1;   // ch 0 仍激活
        notes[16] = 0;  // ch 16 失活
        notes[32] = 1;  // ch 32 新激活
        let ctrls = [0u32; 256];

        assert!(layout.differs_from_counts(&notes, &ctrls), "多 port 翻转");
    }

    #[test]
    fn differs_from_counts_all_inactive() {
        // layout: 全 false（空 model）
        // counts: 全 0 → 无翻转
        let mask = vec![false; 16];
        let layout = ChannelLayout::from_mask(mask);

        let notes = [0u32; 256];
        let ctrls = [0u32; 256];

        assert!(!layout.differs_from_counts(&notes, &ctrls), "全未激活，无翻转");
    }

    /// 集成测试：完整复现 bug 场景，验证 flip 检测触发 teardown。
    ///
    /// 场景：空 model spawn 引擎 → 加首 audible 音符 →
    /// `differs_from_counts` 必须返回 true（检测到 0→1 翻转）→
    /// App 应 teardown，下一帧用新 model 重建。
    #[test]
    fn differs_from_counts_detects_silent_note_bug_scenario() {
        // 1. 空 model → layout 全 false
        let empty = YinModel::default();
        let layout = ChannelLayout::from_model(&empty);

        // 2. 加首 audible 音符后，model 的 channel_note_count[0] = 1
        let with_note = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        // with_note 已经 rebuild 过，channel_note_count 是新鲜的
        assert_eq!(with_note.channel_note_count[0], 1);

        // 3. 旧 layout 检测新 counts → 必须报告翻转
        assert!(
            layout.differs_from_counts(&with_note.channel_note_count, &with_note.channel_ctrl_count),
            "旧 layout（全 false）检测到 ch 0 0→1 翻转"
        );

        // 4. 用新 model 重建 layout → 与新 counts 一致，不再翻转
        let new_layout = ChannelLayout::from_model(&with_note);
        assert!(
            !new_layout.differs_from_counts(&with_note.channel_note_count, &with_note.channel_ctrl_count),
            "新 layout 与新 counts 一致，无翻转"
        );
    }
}
