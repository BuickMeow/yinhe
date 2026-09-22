//! Channel layout: maps source MIDI channels (0..256) to compacted xsynth channel indices.
//!
//! 统一了原来分散在 `spawn::channels_for_model`（算 active_mask）和
//! `AudioEngine::with_parallelism`（算 channel_map）的两套独立扫描逻辑。
//!
//! `ChannelLayout` 在 `AudioEngine` 创建时一次性定型，生命周期内不可变。
//! 若 model 结构变化（增减音轨、改 channel/port），必须 teardown + 重建引擎。

use yinhe_core::{TrackKind, YinModel};

/// dense 通道所属的命名空间（strip/insert/send 反查路由用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelNamespace {
    /// 源 MIDI 通道（0..256，global = port<<4 | channel）。
    /// 该通道同时是乐器的挂载点（插件存在则输出插件音源，否则 XSynth）。
    Midi(usize),
    /// 音频通道（`TrackData::audio_channel`）。
    Audio(usize),
}

/// 不可变的通道布局，`AudioEngine` 创建时定型。
///
/// dense 通道空间如下（引擎/混音台/预览合成器按 `compacted_channels()` 分配）：
/// - `[0, midi_compacted)`：MIDI 源通道（`global_channel`）压缩后的通道，
///   输出 = 该通道挂载的插件乐器（若有）或 XSynth；
/// - `[midi_compacted, compacted)`：**音频通道**（`TrackData::audio_channel`），
///   每个用到的音频通道独占一条 dense 通道（音频片段回放混入混音台）。
#[derive(Clone)]
pub struct ChannelLayout {
    /// `active_mask[i] == true` 表示源 MIDI 通道 `i` 被某条音轨使用（存在即激活）。
    /// 长度 = `num_channels`，超出部分视为未激活。
    active_mask: Vec<bool>,
    /// `channel_map[src] = dense`（激活）或 `u32::MAX`（未激活）。
    /// `dense` 是 `ChannelGroup` 压缩后的通道索引。
    channel_map: Box<[u32; 256]>,
    /// `active_mask` 覆盖的源 MIDI 通道数（= `active_mask.len()`）。
    num_channels: u32,
    /// 用到的音频通道（升序去重，来自音频轨的 `audio_channel`）。
    /// 第 `i` 个的 dense = `midi_compacted + i`。
    audio_channels: Vec<u16>,
    /// MIDI 通道的激活数（= xsynth `ChannelGroup` 的通道数）。
    midi_compacted: u32,
    /// 总激活通道数 = `midi_compacted + audio_channels.len()`。
    compacted_channels: u32,
}

impl ChannelLayout {
    /// 分析 `YinModel` 构建通道布局。
    ///
    /// 源通道"激活"条件：存在 MIDI 音轨使用该通道（`TrackData::global_channel`）。
    /// 音轨存在即激活——空音轨的通道也随时可用（首音符预览/播放立即有声），
    /// 不再按音符/CC 数量推断。成本 O(tracks)，与音符总数无关。
    /// 音频轨没有 MIDI 通道语义，不参与 MIDI 激活（否则默认 port/channel 0
    /// 会凭空激活 A01，混音台/音色库 UI 出现幽灵通道）。
    pub fn from_model(model: &YinModel) -> Self {
        let mut ch_active = [false; 256];
        let mut audio_channels: Vec<u16> = Vec::new();
        for track in model.tracks.iter() {
            if track.kind != TrackKind::Audio {
                let ch = track.global_channel() as usize;
                if ch < 256 {
                    ch_active[ch] = true;
                }
            } else if let Some(ach) = track.audio_channel
                && !audio_channels.contains(&ach)
            {
                audio_channels.push(ach);
            }
        }
        audio_channels.sort_unstable();

        let max_active_ch = ch_active.iter().rposition(|&c| c).unwrap_or(0);
        let num_channels = (max_active_ch + 1).max(1) as u32;

        let active_mask: Vec<bool> = ch_active[..num_channels as usize].to_vec();

        Self::from_mask_full(active_mask, audio_channels)
    }

    /// 从 `active_mask` 构建压缩后的 `channel_map`（无音频通道）。
    pub fn from_mask(active_mask: Vec<bool>) -> Self {
        Self::from_mask_full(active_mask, Vec::new())
    }

    /// 从 `active_mask` + 音频通道构建完整布局。
    fn from_mask_full(active_mask: Vec<bool>, audio_channels: Vec<u16>) -> Self {
        let mut channel_map = Box::new([u32::MAX; 256]);
        let mut next_dense: u32 = 0;
        for (src, &alive) in active_mask.iter().enumerate().take(256) {
            if alive {
                channel_map[src] = next_dense;
                next_dense += 1;
            }
        }
        let midi_compacted = next_dense.max(1);
        let compacted_channels = midi_compacted + audio_channels.len() as u32;
        let num_channels = active_mask.len() as u32;
        Self {
            active_mask,
            channel_map,
            num_channels,
            audio_channels,
            midi_compacted,
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

    /// MIDI 通道的激活数（音频 dense 通道从该值起）。
    pub fn midi_compacted(&self) -> u32 {
        self.midi_compacted
    }

    /// 用到的音频通道列表（升序去重）。
    pub fn audio_channels(&self) -> &[u16] {
        &self.audio_channels
    }

    /// 音频通道 `ach` 的 dense 索引（= midi_compacted + 排序位置），
    /// 未用到返回 `u32::MAX`。
    #[inline]
    pub fn audio_dense_for(&self, ach: u16) -> u32 {
        match self.audio_channels.binary_search(&ach) {
            Ok(i) => self.midi_compacted + i as u32,
            Err(_) => u32::MAX,
        }
    }

    /// dense 通道是不是音频通道（`[midi_compacted, compacted)`）。
    #[inline]
    pub fn is_audio_dense(&self, dense: usize) -> bool {
        dense as u32 >= self.midi_compacted && (dense as u32) < self.compacted_channels
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

    /// 本布局是否覆盖 `model` 的通道需求（引擎"过户复用"判定）。
    ///
    /// 覆盖 = model 用到的**每个**源 MIDI 通道都已在本布局中激活，且 model
    /// 用到的每个音频通道都已在布局里。覆盖成立时引擎无需 teardown 重建，
    /// 直接 `LoadModel` / `UpdateNotes` 复用（布局里多出来的通道保持静默）。
    ///
    /// 与"完全一致"的区别：
    /// - model 少用/不再用某个已激活通道（删音轨、改 channel 移走）→ 仍覆盖；
    /// - model 用到未激活通道（加音轨、改 port/channel 指向新通道）→ 不覆盖，
    ///   必须重建（`ChannelLayout` 创建后不可变，新通道无法被 dispatch）。
    ///
    /// 成本 O(tracks)，与音符总数无关。
    pub fn covers_model(&self, model: &YinModel) -> bool {
        for track in model.tracks.iter() {
            if track.kind != TrackKind::Audio {
                let ch = track.global_channel() as usize;
                if ch < 256 && !self.is_active(ch) {
                    return false;
                }
            } else if let Some(ach) = track.audio_channel
                && self.audio_dense_for(ach) == u32::MAX
            {
                return false;
            }
        }
        true
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
                    id: 0,
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
                    id: 0,
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
                    id: 0,
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
                id: 0,
                tick: 0,
                value: 100.0 / 127.0,
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
    // covers_model 测试：引擎复用（过户）判定核心逻辑
    // -----------------------------------------------------------------------
    // 激活完全由音轨决定：
    // - 加音轨/改到未激活 channel/port → 不覆盖（需重建）
    // - 删音轨/移到已激活 channel → 仍覆盖（复用引擎，直接 UpdateNotes）
    // - 音符增删 → 不改变通道需求 → 覆盖

    #[test]
    fn covers_model_same_layout() {
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);
        assert!(layout.covers_model(&model), "同 model 覆盖");
    }

    #[test]
    fn covers_model_false_when_new_channel_appears() {
        // layout: ch 0 激活；model 新增 ch 1 音轨 → 未覆盖
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);

        let mut extended = model.clone();
        extended.tracks.push(Arc::new(TrackData::new(0, 1)));
        assert!(!layout.covers_model(&extended), "ch 1 未激活，不覆盖");
    }

    #[test]
    fn covers_model_true_when_channel_freed() {
        // layout: ch 0 激活；model 删掉唯一音轨 → 仍覆盖（多余通道静默）
        let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
        let layout = ChannelLayout::from_model(&model);

        let mut reduced = model.clone();
        reduced.tracks.clear();
        assert!(layout.covers_model(&reduced), "占用减少，仍覆盖");
    }

    #[test]
    fn covers_model_channel_move() {
        // 双通道 layout：ch 0→ch 1 移动仍在覆盖内；移到未激活的 ch 2 不覆盖
        let conductor = ConductorData::default();
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        let layout = ChannelLayout::from_model(&model);

        let mut moved = model.clone();
        let t = Arc::make_mut(&mut moved.tracks[0]);
        t.channel = 1;
        assert!(layout.covers_model(&moved), "移到已激活 ch1，仍覆盖");

        let t = Arc::make_mut(&mut moved.tracks[0]);
        t.channel = 2;
        assert!(!layout.covers_model(&moved), "移到未激活 ch2，不覆盖");
    }

    #[test]
    fn covers_model_multi_port_growth() {
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
        assert!(!layout.covers_model(&extended), "多 port 新增通道，不覆盖");
    }

    #[test]
    fn covers_model_all_inactive() {
        let empty = YinModel::default();
        let layout = ChannelLayout::from_model(&empty);
        assert!(layout.covers_model(&empty), "空 model 恒覆盖");
    }

    /// 集成测试：完整复现 bug 场景——空工程写第一个音符必须立即有声。
    ///
    /// 场景：空 model spawn 引擎（无音轨）→ 加音轨（即使还没有音符）→
    /// 未覆盖 → teardown；重建后通道已激活，再写第一个音符无需任何重建。
    #[test]
    fn covers_model_first_track_activation_needs_rebuild() {
        // 1. 空 model → layout 全 false
        let empty = YinModel::default();
        let layout = ChannelLayout::from_model(&empty);

        // 2. 加音轨（ch 0，无音符）→ 未覆盖，必须重建
        let mut with_track = empty.clone();
        with_track.tracks.push(Arc::new(TrackData::new(0, 0)));
        assert!(!layout.covers_model(&with_track), "ch 0 未激活，不覆盖");

        // 3. 重建 layout → 覆盖成立；空音轨的通道已激活
        let new_layout = ChannelLayout::from_model(&with_track);
        assert!(new_layout.covers_model(&with_track), "新 layout 覆盖");
        assert!(new_layout.is_active(0), "空音轨通道已激活");
    }

    /// 多条 MIDI 轨共享同一 MIDI 通道 → 只占一条 dense（乐器挂载不参与布局）。
    #[test]
    fn shared_midi_channel_deduplicates_dense() {
        let conductor = ConductorData::default();
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 3)),
                Arc::new(TrackData::new(0, 0)),
            ],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        let layout = ChannelLayout::from_model(&model);
        assert_eq!(layout.midi_compacted(), 2);
        assert_eq!(layout.compacted_channels(), 2);
        assert_eq!(layout.dense_for(0), 0);
        assert_eq!(layout.dense_for(3), 1);
        assert!(!layout.is_audio_dense(0));
        assert!(!layout.is_audio_dense(1));
    }

    /// 音频通道 dense 从 midi_compacted 起，MIDI 通道数不受影响。
    #[test]
    fn audio_channels_follow_midi_segment() {
        let conductor = ConductorData::default();
        let mut audio = TrackData::new(0, 0);
        audio.kind = TrackKind::Audio;
        audio.audio_channel = Some(7);
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(TrackData::new(0, 0)), Arc::new(audio)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        let layout = ChannelLayout::from_model(&model);
        assert_eq!(layout.midi_compacted(), 1);
        assert_eq!(layout.audio_channels(), &[7]);
        assert_eq!(layout.audio_dense_for(7), 1);
        assert_eq!(layout.compacted_channels(), 2);
        assert!(layout.is_audio_dense(1));
        assert!(!layout.is_audio_dense(0));
        assert!(layout.covers_model(&model));

        // 新增未覆盖的音频通道 → 不覆盖；移除音频轨 → 仍覆盖
        let mut extra = model.clone();
        let mut audio2 = TrackData::new(0, 0);
        audio2.kind = TrackKind::Audio;
        audio2.audio_channel = Some(9);
        extra.tracks.push(Arc::new(audio2));
        extra.rebuild();
        assert!(!layout.covers_model(&extra), "音频通道 9 未覆盖");

        let mut removed = model.clone();
        removed.tracks.retain(|t| t.kind != TrackKind::Audio);
        removed.rebuild();
        assert!(layout.covers_model(&removed), "音频轨移除仍覆盖");
    }
}
