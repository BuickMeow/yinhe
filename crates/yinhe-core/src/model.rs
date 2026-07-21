//! YinModel + TrackData + ConductorData + ProjectMeta + rebuild logic.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use yinhe_types::{AutomationLane, AutomationTarget, PcEvent};
use crate::events::NoteEvent;
use crate::tempo_map::{
    DEFAULT_MPQ, TempoMap, TempoSegment, mpq_from_bpm, recompute_tempo_start_times,
};

// =========================================================
//  Conductor
// =========================================================

/// Global score-level events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConductorData {
    /// Tempo 走 AutomationLane（与 CC/PB/RPN 一致），存于 `target == AutomationTarget::Tempo`。
    /// `events[i].value` 直接装 bpm（f32）。
    pub tempo: AutomationLane,
    pub time_sig: Vec<yinhe_types::TimeSigEvent>,
}

impl Default for ConductorData {
    fn default() -> Self {
        Self {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: Vec::new(),
            },
            time_sig: Vec::new(),
        }
    }
}

// =========================================================
//  TrackData
// =========================================================

/// One MIDI track's complete data.
///
/// Channel/track are held here, not in individual events. NoteEvent is
/// looked up by `(track_idx, key, note.id)`.
///
/// Control events (CC, PitchBend, RPN, NRPN) are unified into
/// `automation_lanes` — one lane per parameter per track. Program Change
/// is stored separately since it is a discrete event, not a continuous
/// automation curve.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackData {
    pub uuid: String,
    pub name: String,
    pub color: [f32; 3],
    /// MIDI port (0..16, displayed as A..P).
    pub port: u8,
    /// MIDI channel (0..16, displayed as 1..16).
    pub channel: u8,
    pub channel_prefix: Option<u8>,
    pub muted: bool,
    pub soloed: bool,

    /// Notes are stored in `YinModel.notes` (by-key store).
    /// This field is only used during parsing and is moved out
    /// by `YinModel::load_track_notes`. At runtime it is empty.
    pub notes: Vec<NoteEvent>,
    /// Unified automation lanes: CC, PitchBend, RPN, NRPN.
    /// Each lane is a sorted list of (tick, value) events.
    pub automation_lanes: Vec<yinhe_types::AutomationLane>,
    /// Program Change events (discrete, not automation).
    pub program_change: Vec<PcEvent>,
}

impl TrackData {
    pub fn new(port: u8, channel: u8) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            color: [0.5, 0.5, 0.5],
            port,
            channel,
            channel_prefix: None,
            muted: false,
            soloed: false,
            notes: Vec::new(),
            automation_lanes: Vec::new(),
            program_change: Vec::new(),
        }
    }

    /// Global channel = port * 16 + channel (0..255).
    pub fn global_channel(&self) -> u8 {
        (self.port & 0x0F) << 4 | (self.channel & 0x0F)
    }
}

// =========================================================
//  Project metadata
// =========================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub artist: String,
    pub description: String,
    pub ppq: u32,
    pub compression_level: i32,
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            artist: String::new(),
            description: String::new(),
            ppq: 480,
            compression_level: 3,
        }
    }
}

// =========================================================
//  YinModel
// =========================================================

/// The single source of truth for a Yinhe project.
///
/// All note data lives in the `notes` array (by key, sorted by start_tick).
/// `TrackData` no longer stores notes — they are accessed via `notes[key]`
/// and filtered by `note.track` when per-track iteration is needed.
///
/// Tracks are held in `Arc<TrackData>` for cheap clone-on-write editing
/// (C1 mode). The conductor is also Arc to avoid copying it when only
/// a track changes. tempo_map is derived from conductor.
#[derive(Clone, Debug)]
pub struct YinModel {
    pub conductor: Arc<ConductorData>,
    pub tracks: Vec<Arc<TrackData>>,
    pub tempo_map: Arc<TempoMap>,
    pub meta: ProjectMeta,

    /// Single authoritative note store: `notes[key]` = all notes at that key,
    /// sorted by start_tick. Each note carries its track index and全局唯一 id。
    /// Compatible with `yinhe_types::NoteSource`.
    pub notes: Box<[Arc<Vec<yinhe_types::Note>>; 128]>,
    pub note_count: u64,
    pub tick_length: u64,
    /// Per-track note count cache (avoids scanning 128 buckets for stats).
    pub track_note_count: Vec<u64>,
    /// Per-track audible note count (velocity > 1). `> 0` means the track
    /// has at least one audible note. Replaces the old `track_has_audio_cache: Vec<bool>`.
    pub track_audible_count: Vec<u64>,

    /// Per-global-channel audible note count (velocity > 1).
    /// `channel_note_count[ch] > 0` 表示源通道 ch 上有至少一个发声音符。
    /// 用于音频引擎检测 ChannelLayout 激活状态翻转（0→1 / 1→0），
    /// 触发 teardown + 重建。索引 = `TrackData::global_channel()`。
    pub channel_note_count: Box<[u32; 256]>,
    /// Per-global-channel 控制事件计数（automation_lanes 或 program_change 非空的轨道数）。
    /// 与 `channel_note_count` 一起决定 channel 是否激活：
    /// `active(ch) = channel_note_count[ch] > 0 || channel_ctrl_count[ch] > 0`。
    pub channel_ctrl_count: Box<[u32; 256]>,

    /// Dirty bucket tracking: `dirty_keys[k]` is true when bucket k has been
    /// modified and needs sorting. Use `mark_dirty()` to set, `rebuild_dirty()`
    /// to clear. Public for struct construction via `..Default::default()`.
    pub dirty_keys: [bool; 128],

    /// Per-key note revision counter. Bumped every time `mark_dirty(k)` is
    /// called, **not** cleared by `rebuild_dirty()`. Consumers (e.g. GPU cull
    /// buffer upload) compare these to detect which keys need incremental
    /// re-upload without rescanning all 128 buckets.
    pub note_revisions: [u64; 128],

    /// Per-bucket note count cache for O(D) incremental stats in `rebuild_dirty()`.
    /// Updated by `rebuild()`, `load_track_notes()`, and `rebuild_dirty()`.
    pub bucket_note_count: [u64; 128],

    /// Per-bucket max end_tick cache. Enables correct incremental `tick_length`
    /// in `rebuild_dirty()`: shrink-aware (max over all 128 buckets) without
    /// O(N) full scan. Updated by `rebuild()`, `load_track_notes()`, and `rebuild_dirty()`.
    pub bucket_max_end_tick: [u64; 128],

    /// Per-bucket per-track (total, audible) counts. Sparse: each bucket
    /// only stores tracks that actually have notes in it. Enables
    /// O(dirty bucket size) incremental `track_note_count` /
    /// `track_audible_count` updates in `rebuild_dirty()` instead of
    /// rescanning all 128 buckets.
    pub bucket_track_stats: [HashMap<u16, (u64, u64)>; 128],

    /// 全局音符 id 发号器（下一个待分配的 id）。
    /// 0 保留为"未分配"哨兵，实际 id 从 1 开始。
    /// 编辑时调 `alloc_note_id()`，加载时由 `load_track_notes` 统一分配。
    pub next_note_id: u32,
}

impl Default for YinModel {
    fn default() -> Self {
        Self {
            conductor: Arc::new(ConductorData::default()),
            tracks: Vec::new(),
            tempo_map: Arc::new(TempoMap::default()),
            meta: ProjectMeta::default(),
            notes: Box::new(core::array::from_fn(|_| Arc::new(Vec::new()))),
            note_count: 0,
            tick_length: 0,
            track_note_count: Vec::new(),
            track_audible_count: Vec::new(),
            channel_note_count: Box::new([0u32; 256]),
            channel_ctrl_count: Box::new([0u32; 256]),
            dirty_keys: [false; 128],
            note_revisions: [0; 128],
            bucket_note_count: [0; 128],
            bucket_max_end_tick: [0; 128],
            bucket_track_stats: core::array::from_fn(|_| HashMap::new()),
            next_note_id: 1,
        }
    }
}

impl YinModel {
    /// 新建一个空工程：Conductor 轨（120 BPM + 4/4 拍号）+ 16 条 A1..A16 轨道。
    ///
    /// 工厂方法封装了 `Document::empty()` 之前内联构造的样板代码，
    /// 避免 document.rs 直接操作 YinModel 内部字段。
    /// 调用者拿到模型后自行决定是否 `rebuild()`（首次构造不需要，
    /// 因为没有音符；但 `Document::empty()` 仍会调用以初始化 tempo_map）。
    pub fn new_empty_with_16_tracks() -> Self {
        let conductor = Arc::new(ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![yinhe_types::AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: yinhe_types::SegmentShape::Step,
                }],
            },
            time_sig: vec![yinhe_types::TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            }],
        });

        let mut tracks: Vec<Arc<TrackData>> = Vec::with_capacity(17);
        let mut t = TrackData::new(0, 0);
        t.name = "Conductor".to_string();
        tracks.push(Arc::new(t));
        for ch in 0..16u8 {
            let mut t = TrackData::new(0, ch);
            t.name = format!("A{}", ch + 1);
            tracks.push(Arc::new(t));
        }

        Self {
            conductor,
            tracks,
            ..Default::default()
        }
    }

    /// Build TempoMap from conductor.tempo / conductor.time_sig.
    pub(crate) fn build_tempo_map(&self) -> TempoMap {
        let ppq = self.meta.ppq;

        // Convert AutomationEvent -> TempoSegment, sorted by tick.
        let mut segments: Vec<TempoSegment> = if self.conductor.tempo.events.is_empty() {
            vec![TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: DEFAULT_MPQ,
            }]
        } else {
            let mut segs: Vec<TempoSegment> = self
                .conductor
                .tempo
                .events
                .iter()
                .map(|t| TempoSegment {
                    start_tick: t.tick,
                    start_time: 0.0,
                    micros_per_quarter: mpq_from_bpm(t.value),
                })
                .collect();
            segs.sort_by_key(|s| s.start_tick);
            // Ensure a segment exists at tick 0 so lookups before the first
            // tempo event have something to find.
            if segs[0].start_tick != 0 {
                segs.insert(
                    0,
                    TempoSegment {
                        start_tick: 0,
                        start_time: 0.0,
                        micros_per_quarter: DEFAULT_MPQ,
                    },
                );
            }
            segs
        };
        recompute_tempo_start_times(&mut segments, ppq);

        let mut ts_events = self.conductor.time_sig.clone();
        ts_events.sort_by_key(|e| e.tick);

        let time_sig_default = ts_events
            .first()
            .map(|e| (e.numerator, e.denominator))
            .unwrap_or((4, 2));

        TempoMap {
            ticks_per_beat: ppq,
            tempo_segments: segments,
            time_sig_events: ts_events,
            time_sig_default,
            tick_length: self.tick_length,
        }
    }

    /// Only rebuild `tempo_map` from `conductor.tempo` / `conductor.time_sig`.
    ///
    /// Use after editing Tempo automation events when notes are untouched.
    /// O(tempo_events + time_sig_events), typically < 100 events — near-instant
    /// even for 100M-note projects. Cheaper than `rebuild()` / `rebuild_dirty()`
    /// which also sort/rescan note buckets.
    pub fn rebuild_tempo_map(&mut self) {
        self.tempo_map = Arc::new(self.build_tempo_map());
    }

    /// Iterate all notes belonging to a specific track.
    ///
    /// Scans all 128 key buckets and yields notes where `note.track == track_idx`.
    /// O(N) in total notes. For statistics use `track_note_count[track_idx]`.
    pub fn notes_for_track(&self, track_idx: u16) -> impl Iterator<Item = &yinhe_types::Note> {
        self.notes
            .iter()
            .flat_map(move |bucket| bucket.iter().filter(move |n| n.track == track_idx))
    }

    /// Check if a track has any audible notes (velocity > 1).
    pub fn track_has_audio(&self, track_idx: u16) -> bool {
        self.track_audible_count
            .get(track_idx as usize)
            .copied()
            .unwrap_or(0)
            > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
        }
    }

    #[test]
    fn empty_model_rebuild() {
        let mut m = YinModel::default();
        m.rebuild();
        assert_eq!(m.note_count, 0);
        assert_eq!(m.tick_length, 0);
        assert!(m.notes.iter().all(|v| v.is_empty()));
    }

    #[test]
    fn rebuild_counts_and_buckets_notes() {
        let per_track = vec![vec![
            note(0, 480, 60),
            note(480, 960, 64),
            note(960, 1920, 60),
        ]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.note_count, 3);
        assert_eq!(m.tick_length, 1920);
        assert_eq!(m.notes[60].len(), 2);
        assert_eq!(m.notes[64].len(), 1);
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][1].start_tick, 960);
    }

    #[test]
    fn rebuild_sorts_per_key() {
        // Insert notes out-of-order; cache should be sorted by start_tick.
        let per_track = vec![vec![note(960, 1920, 60), note(0, 480, 60)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][1].start_tick, 960);
    }

    #[test]
    fn rebuild_builds_tempo_map_with_default_when_no_tempo() {
        let mut m = YinModel::default();
        m.rebuild();
        // Default tempo segment at tick 0 with 120 BPM.
        assert_eq!(m.tempo_map.tempo_segments.len(), 1);
        assert!((m.tempo_map.tempo_segments[0].start_time - 0.0).abs() < 1e-9);
    }

    #[test]
    fn rebuild_builds_tempo_map_from_conductor() {
        let mut conductor = ConductorData::default();
        conductor.tempo.events.push(yinhe_types::AutomationEvent { tick: 0, value: 120.0, shape: yinhe_types::SegmentShape::Step });
        conductor.tempo.events.push(yinhe_types::AutomationEvent { tick: 1920, value: 60.0, shape: yinhe_types::SegmentShape::Step });

        let mut m = YinModel {
            conductor: Arc::new(conductor),
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.tempo_map.tempo_segments.len(), 2);
        // Second segment at tick 1920, time = 1920/480 * 0.5s/quarter = 2s
        let expected = 1920.0 / 480.0 * 0.5;
        assert!((m.tempo_map.tempo_segments[1].start_time - expected).abs() < 1e-6);
    }

    #[test]
    fn rebuild_inserts_implicit_zero_tempo_segment() {
        // Tempo events that don't start at tick 0 should still produce a
        // segment at tick 0 (default 120 BPM) so lookups before the first
        // tempo find something.
        let mut conductor = ConductorData::default();
        conductor.tempo.events.push(yinhe_types::AutomationEvent { tick: 1920, value: 60.0, shape: yinhe_types::SegmentShape::Step });

        let mut m = YinModel {
            conductor: Arc::new(conductor),
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.tempo_map.tempo_segments.len(), 2);
        assert_eq!(m.tempo_map.tempo_segments[0].start_tick, 0);
    }

    #[test]
    fn track_global_channel() {
        let t = TrackData::new(2, 5);
        // (port=2 << 4) | (channel=5) = 0x25
        assert_eq!(t.global_channel(), 0x25);
    }

    #[test]
    fn track_uuid_is_unique() {
        let a = TrackData::new(0, 0);
        let b = TrackData::new(0, 0);
        assert_ne!(a.uuid, b.uuid);
    }

    /// Verify `rebuild_dirty` keeps `track_note_count` / `track_audible_count`
    /// consistent when notes are added/removed/edited. We compare the
    /// incremental update path against a full `rebuild()` from scratch.
    fn note_audible(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
        }
    }

    fn note_silent(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 0, // silent — must not count toward `track_audible_count`
        }
    }

    #[test]
    fn rebuild_dirty_keeps_track_stats_consistent() {
        // Build a model with 2 tracks and 4 notes, then mutate it and
        // compare `rebuild_dirty`'s incremental track stats against a
        // full `rebuild()` of the same final state.
        let per_track = vec![
            vec![note_audible(0, 480, 60), note_audible(480, 960, 64)],
            vec![note_silent(0, 480, 60), note_audible(0, 240, 62)],
        ];
        let mut base = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0)), Arc::new(TrackData::new(0, 1))],
            ..Default::default()
        };
        base.load_track_notes(per_track);

        // Final desired state: same notes as `base` plus one new silent
        // note in bucket 60 (track 1) and one new audible note in
        // bucket 62 (track 1). We compute this in two ways and compare.
        let mut m_inc = base.clone();
        {
            let model = Arc::make_mut(&mut m_inc.notes[60]);
            model.push(yinhe_types::Note {
                id: 0,
                start_tick: 960,
                end_tick: 1440,
                velocity: 0,
                track: 1,
            });
        }
        {
            let model = Arc::make_mut(&mut m_inc.notes[62]);
            model.push(yinhe_types::Note {
                id: 0,
                start_tick: 240,
                end_tick: 480,
                velocity: 80,
                track: 1,
            });
        }
        m_inc.mark_dirty(60);
        m_inc.mark_dirty(62);
        m_inc.rebuild_dirty();

        let mut m_full = base.clone();
        {
            let model = Arc::make_mut(&mut m_full.notes[60]);
            model.push(yinhe_types::Note {
                id: 0,
                start_tick: 960,
                end_tick: 1440,
                velocity: 0,
                track: 1,
            });
        }
        {
            let model = Arc::make_mut(&mut m_full.notes[62]);
            model.push(yinhe_types::Note {
                id: 0,
                start_tick: 240,
                end_tick: 480,
                velocity: 80,
                track: 1,
            });
        }
        m_full.rebuild();

        assert_eq!(m_inc.track_note_count, m_full.track_note_count, "track_note_count drift");
        assert_eq!(m_inc.track_audible_count, m_full.track_audible_count, "track_audible_count drift");
        assert_eq!(m_inc.note_count, m_full.note_count);
        assert_eq!(m_inc.tick_length, m_full.tick_length);

        // Sanity: track 0 = 2 audible, 0 silent. track 1 = 2 audible, 2 silent.
        assert_eq!(m_full.track_note_count, vec![2, 4]);
        assert_eq!(m_full.track_audible_count, vec![2, 2]);

        // Now test removal: remove a note from bucket 60 in track 0.
        let mut m_del = m_full.clone();
        {
            let model = Arc::make_mut(&mut m_del.notes[60]);
            model.retain(|n| !(n.track == 0 && n.start_tick == 0));
        }
        m_del.mark_dirty(60);
        m_del.rebuild_dirty();

        let mut m_del_ref = m_full.clone();
        {
            let model = Arc::make_mut(&mut m_del_ref.notes[60]);
            model.retain(|n| !(n.track == 0 && n.start_tick == 0));
        }
        m_del_ref.rebuild();

        assert_eq!(m_del.track_note_count, m_del_ref.track_note_count, "track_note_count drift after remove");
        assert_eq!(m_del.track_audible_count, m_del_ref.track_audible_count, "track_audible_count drift after remove");
    }

    /// 回归测试：删除全曲最后一个音符（max end_tick 所在音符）后，
    /// `rebuild_dirty` 必须收缩 `tick_length`。
    /// 之前 bug：`new_tick_length` 从 `self.tick_length` 起步只取 max，
    /// 永远不会缩小，导致 select_all_pr/ar 范围、播放结束判定错误。
    #[test]
    fn rebuild_dirty_tick_length_shrinks_after_deleting_last_note() {
        let per_track = vec![
            // track 0: 两个音符，end_tick 分别为 480 和 1920（末尾）
            vec![note_audible(0, 480, 60), note_audible(480, 1920, 62)],
        ];
        let mut m = YinModel::default();
        m.tracks = vec![Arc::new(super::TrackData::new(0, 0))];
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.tick_length, 1920, "初始 tick_length 应为末尾音符 end_tick");

        // 删除 end_tick=1920 的末尾音符
        {
            let bucket = Arc::make_mut(&mut m.notes[62]);
            bucket.retain(|n| !(n.start_tick == 480 && n.end_tick == 1920));
        }
        m.mark_dirty(62);
        m.rebuild_dirty();

        // tick_length 必须收缩到 480（剩下的音符 end_tick）
        assert_eq!(
            m.tick_length, 480,
            "删除末尾音符后 tick_length 必须收缩（bug C-3 回归）"
        );

        // 再删掉剩下的音符 → tick_length 应为 0
        {
            let bucket = Arc::make_mut(&mut m.notes[60]);
            bucket.clear();
        }
        m.mark_dirty(60);
        m.rebuild_dirty();
        assert_eq!(m.tick_length, 0, "清空所有音符后 tick_length 必须为 0");
    }

    // -----------------------------------------------------------------------
    // channel_note_count / channel_ctrl_count 测试
    // -----------------------------------------------------------------------
    // 这些计数器是音频引擎做 ChannelLayout flip 检测的关键依据：
    // active(ch) = channel_note_count[ch] > 0 || channel_ctrl_count[ch] > 0
    // 任何 channel 的 active 翻转（0→1 / 1→0）都意味着 ChannelLayout 变了，
    // 必须 teardown + 重建引擎。

    #[test]
    fn channel_counts_empty_model() {
        let m = YinModel::default();
        assert!(m.channel_note_count.iter().all(|&c| c == 0));
        assert!(m.channel_ctrl_count.iter().all(|&c| c == 0));
    }

    #[test]
    fn channel_counts_after_load_track_notes() {
        // track 0 (ch 0): 2 audible + 1 silent
        // track 1 (ch 1): 1 audible
        // track 2 (ch 9): 1 audible
        let per_track = vec![
            vec![note_audible(0, 480, 60), note_audible(480, 960, 64), note_silent(0, 480, 60)],
            vec![note_audible(0, 480, 60)],
            vec![note_audible(0, 480, 60)],
        ];
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
                Arc::new(TrackData::new(0, 9)),
            ],
            ..Default::default()
        };
        m.load_track_notes(per_track);

        assert_eq!(m.channel_note_count[0], 2, "ch 0: 2 audible (silent 不计)");
        assert_eq!(m.channel_note_count[1], 1);
        assert_eq!(m.channel_note_count[9], 1);
        assert!(m.channel_ctrl_count.iter().all(|&c| c == 0), "无 automation / PC");
    }

    #[test]
    fn channel_counts_rebuild_dirty_matches_rebuild() {
        // 增量 rebuild_dirty 算出的 channel counts 必须与全量 rebuild 一致。
        let mut base = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        base.load_track_notes(vec![
            vec![note_audible(0, 480, 60)],
            vec![note_audible(0, 480, 64)],
        ]);

        // 改动：在 bucket 60 加一个 track 0 的 audible 音符 + 一个 silent 音符
        let mut m_inc = base.clone();
        {
            let bucket = Arc::make_mut(&mut m_inc.notes[60]);
            bucket.push(yinhe_types::Note {
                id: 0, start_tick: 960, end_tick: 1440, velocity: 80, track: 0,
            });
            bucket.push(yinhe_types::Note {
                id: 0, start_tick: 1440, end_tick: 1920, velocity: 0, track: 0,
            });
        }
        m_inc.mark_dirty(60);
        m_inc.rebuild_dirty();

        let mut m_full = base.clone();
        {
            let bucket = Arc::make_mut(&mut m_full.notes[60]);
            bucket.push(yinhe_types::Note {
                id: 0, start_tick: 960, end_tick: 1440, velocity: 80, track: 0,
            });
            bucket.push(yinhe_types::Note {
                id: 0, start_tick: 1440, end_tick: 1920, velocity: 0, track: 0,
            });
        }
        m_full.rebuild();

        assert_eq!(
            m_inc.channel_note_count, m_full.channel_note_count,
            "channel_note_count 增量 vs 全量不一致"
        );
        assert_eq!(
            m_inc.channel_ctrl_count, m_full.channel_ctrl_count,
            "channel_ctrl_count 增量 vs 全量不一致"
        );
        // ch 0 现在 2 个 audible（base 1 个 + 新增 1 个），silent 不计
        assert_eq!(m_full.channel_note_count[0], 2);
    }

    #[test]
    fn channel_counts_ctrl_tracks() {
        // track 0 (ch 0): 无音符，但有 automation_lanes → ctrl_count[0] = 1
        // track 1 (ch 5): 无音符，但有 program_change → ctrl_count[5] = 1
        use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, PcEvent, SegmentShape};
        let mut t0 = TrackData::new(0, 0);
        t0.automation_lanes = vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent { tick: 0, value: 100.0, shape: SegmentShape::Step }],
        }];
        let mut t1 = TrackData::new(0, 5);
        t1.program_change = vec![PcEvent { tick: 0, program: 5, bank_msb: 0, bank_lsb: 0 }];
        let mut m = YinModel {
            tracks: vec![Arc::new(t0), Arc::new(t1)],
            ..Default::default()
        };
        m.rebuild();

        assert_eq!(m.channel_ctrl_count[0], 1, "ch 0 有 automation");
        assert_eq!(m.channel_ctrl_count[5], 1, "ch 5 有 PC");
        assert!(m.channel_note_count.iter().all(|&c| c == 0), "无音符");
    }

    #[test]
    fn channel_counts_first_audible_note_addition() {
        // 模拟 bug 复现场景：空 model 加第一个 audible 音符 → channel_note_count[0] 从 0→1
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.channel_note_count[0], 0, "空 model: ch 0 未激活");

        // 加一个 audible 音符
        let bucket = Arc::make_mut(&mut m.notes[60]);
        bucket.push(yinhe_types::Note {
            id: 0, start_tick: 0, end_tick: 480, velocity: 100, track: 0,
        });
        m.mark_dirty(60);
        m.rebuild_dirty();

        assert_eq!(m.channel_note_count[0], 1, "加首 audible 音符后: ch 0 激活");
    }

    #[test]
    fn channel_counts_last_audible_note_removal() {
        // ch 0 上唯一一个 audible 音符被删 → channel_note_count[0] 从 1→0
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![note_audible(0, 480, 60)]]);
        assert_eq!(m.channel_note_count[0], 1);

        // 删掉这个音符
        let bucket = Arc::make_mut(&mut m.notes[60]);
        bucket.clear();
        m.mark_dirty(60);
        m.rebuild_dirty();

        assert_eq!(m.channel_note_count[0], 0, "删末 audible 音符后: ch 0 失活");
    }

    // -----------------------------------------------------------------------
    // rescale_ppq 测试
    // -----------------------------------------------------------------------

    #[test]
    fn rescale_ppq_doubles_ticks_for_integer_ratio() {
        // 480 → 960：所有 tick ×2，整数比例，精确。
        let per_track = vec![vec![note(0, 480, 60), note(480, 960, 64)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        assert_eq!(m.meta.ppq, 480);

        m.rescale_ppq(960);
        assert_eq!(m.meta.ppq, 960);
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][0].end_tick, 960);
        assert_eq!(m.notes[64][0].start_tick, 960);
        assert_eq!(m.notes[64][0].end_tick, 1920);
        assert_eq!(m.tick_length, 1920);
        assert_eq!(m.tempo_map.ticks_per_beat, 960);
    }

    #[test]
    fn rescale_ppq_halves_ticks() {
        // 960 → 480：所有 tick ÷2，整数比例，精确。
        let per_track = vec![vec![note(0, 960, 60), note(960, 1920, 64)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            meta: ProjectMeta { ppq: 960, ..Default::default() },
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();

        m.rescale_ppq(480);
        assert_eq!(m.meta.ppq, 480);
        assert_eq!(m.notes[60][0].start_tick, 0);
        assert_eq!(m.notes[60][0].end_tick, 480);
        assert_eq!(m.notes[64][0].start_tick, 480);
        assert_eq!(m.notes[64][0].end_tick, 960);
    }

    #[test]
    fn rescale_ppq_scales_automation_and_conductor() {
        // 验证 conductor.tempo / time_sig / track automation / pc 的 tick 也被缩放。
        use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, PcEvent, SegmentShape, TimeSigEvent};
        let mut conductor = ConductorData::default();
        conductor.tempo.events.push(AutomationEvent { tick: 480, value: 120.0, shape: SegmentShape::Step });
        conductor.time_sig.push(TimeSigEvent { tick: 960, numerator: 4, denominator: 2 });

        let mut t0 = TrackData::new(0, 0);
        t0.automation_lanes = vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent { tick: 240, value: 100.0, shape: SegmentShape::Step }],
        }];
        t0.program_change = vec![PcEvent { tick: 120, program: 5, bank_msb: 0, bank_lsb: 0 }];

        let mut m = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t0)],
            ..Default::default()
        };
        m.rebuild();

        m.rescale_ppq(960);
        // 480→960: ×2
        assert_eq!(m.conductor.tempo.events[0].tick, 960);
        assert_eq!(m.conductor.time_sig[0].tick, 1920);
        assert_eq!(m.tracks[0].automation_lanes[0].events[0].tick, 480);
        assert_eq!(m.tracks[0].program_change[0].tick, 240);
    }

    #[test]
    fn rescale_ppq_noop_when_same() {
        let per_track = vec![vec![note(0, 480, 60)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        let before = m.clone();
        m.rescale_ppq(480);
        assert_eq!(m.notes[60][0].start_tick, before.notes[60][0].start_tick);
        assert_eq!(m.notes[60][0].end_tick, before.notes[60][0].end_tick);
    }

    #[test]
    fn rescale_ppq_round_trip_restores_original() {
        // 480 → 960 → 480：整数比例下 round-trip 精确还原。
        let per_track = vec![vec![note(0, 480, 60), note(480, 960, 64)]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        let original = m.clone();

        m.rescale_ppq(960);
        m.rescale_ppq(480);

        assert_eq!(m.meta.ppq, 480);
        assert_eq!(m.notes[60][0].start_tick, original.notes[60][0].start_tick);
        assert_eq!(m.notes[60][0].end_tick, original.notes[60][0].end_tick);
        assert_eq!(m.notes[64][0].start_tick, original.notes[64][0].start_tick);
        assert_eq!(m.notes[64][0].end_tick, original.notes[64][0].end_tick);
    }

    // -----------------------------------------------------------------------
    // new_empty_with_16_tracks 测试
    // -----------------------------------------------------------------------

    #[test]
    fn new_empty_with_16_tracks_creates_conductor_plus_16_tracks() {
        let mut m = YinModel::new_empty_with_16_tracks();
        m.rebuild();
        assert_eq!(m.tracks.len(), 17);
        assert_eq!(m.tracks[0].name, "Conductor");
        assert_eq!(m.tracks[1].name, "A1");
        assert_eq!(m.tracks[16].name, "A16");
        // Conductor 应在 ch 0
        assert_eq!(m.tracks[0].channel, 0);
        // A1 在 ch 0, A16 在 ch 15
        assert_eq!(m.tracks[1].channel, 0);
        assert_eq!(m.tracks[16].channel, 15);
        // 120 BPM + 4/4 拍号
        assert_eq!(m.conductor.tempo.events.len(), 1);
        assert_eq!(m.conductor.tempo.events[0].value, 120.0);
        assert_eq!(m.conductor.time_sig.len(), 1);
        assert_eq!(m.conductor.time_sig[0].numerator, 4);
        // tempo_map 应被 rebuild 出来
        assert_eq!(m.tempo_map.tempo_segments.len(), 1);
        assert_eq!(m.tempo_map.time_sig_default, (4, 2));
    }
}

/// Display-oriented track info derived from `TrackData` for UI panels.
#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub index: u16,
    pub name: String,
    pub note_count: u64,
    pub event_count: u64,
    pub port: u8,
    pub channel: u8,
}
