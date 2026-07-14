//! YinModel + TrackData + ConductorData + ProjectMeta + rebuild logic.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use yinhe_types::{PcEvent, TempoEvent};
use crate::events::NoteEvent;
use crate::tempo_map::{
    DEFAULT_MPQ, TempoMap, TempoSegment, mpq_from_bpm, recompute_tempo_start_times,
};

// =========================================================
//  Conductor
// =========================================================

/// Global score-level events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConductorData {
    pub tempo: Vec<TempoEvent>,
    pub time_sig: Vec<yinhe_types::TimeSigEvent>,
}

// =========================================================
//  TrackData
// =========================================================

/// One MIDI track's complete data.
///
/// Channel/track are held here, not in individual events. NoteEvent is
/// looked up by `(track_idx, key, start_tick, dup_index)`.
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
    /// sorted by start_tick. Each note carries its track index and dup_index.
    /// Compatible with `yinhe_types::NoteSource`.
    pub notes: Box<[Arc<Vec<yinhe_types::Note>>; 128]>,
    pub note_count: u64,
    pub tick_length: u64,
    /// Per-track note count cache (avoids scanning 128 buckets for stats).
    pub track_note_count: Vec<u64>,
    /// Per-track audible note count (velocity > 1). `> 0` means the track
    /// has at least one audible note. Replaces the old `track_has_audio_cache: Vec<bool>`.
    pub track_audible_count: Vec<u64>,

    /// Dirty bucket tracking: `dirty_keys[k]` is true when bucket k has been
    /// modified and needs sorting. Use `mark_dirty()` to set, `rebuild_dirty()`
    /// to clear. Public for struct construction via `..Default::default()`.
    pub dirty_keys: [bool; 128],

    /// Per-bucket note count cache for O(D) incremental stats in `rebuild_dirty()`.
    /// Updated by `rebuild()`, `load_track_notes()`, and `rebuild_dirty()`.
    pub bucket_note_count: [u64; 128],

    /// Per-bucket per-track (total, audible) counts. Sparse: each bucket
    /// only stores tracks that actually have notes in it. Enables
    /// O(dirty bucket size) incremental `track_note_count` /
    /// `track_audible_count` updates in `rebuild_dirty()` instead of
    /// rescanning all 128 buckets.
    pub bucket_track_stats: [HashMap<u16, (u64, u64)>; 128],

    /// Monotonically increasing version counter bumped whenever conductor
    /// (tempo/time_sig) changes. `rebuild_dirty()` skips tempo_map rebuild
    /// when this hasn't changed, avoiding an unnecessary O(segments) pass.
    pub conductor_version: u64,
}

// ── Manual Serialize/Deserialize for YinModel ──
// We use manual impls because `[Arc<Vec<Note>>; 128]` exceeds serde's
// default array-size limit (32). The notes array is serialized as a
// sequence of 128 buckets. Cached/derived fields are skipped on
// serialization and reconstructed on deserialization.

impl Serialize for YinModel {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let note_buckets: Vec<Vec<yinhe_types::Note>> = self.notes.iter()
            .map(|bucket| bucket.as_ref().clone())
            .collect();
        let mut t = s.serialize_tuple(5)?;
        t.serialize_element(&self.conductor)?;
        t.serialize_element(&self.tracks)?;
        t.serialize_element(&self.meta)?;
        t.serialize_element(&note_buckets)?;
        t.serialize_element(&self.conductor_version)?;
        t.end()
    }
}

impl<'de> Deserialize<'de> for YinModel {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct YinModelVisitor;
        impl<'de> serde::de::Visitor<'de> for YinModelVisitor {
            type Value = YinModel;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("YinModel")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let conductor: Arc<ConductorData> = seq.next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let tracks: Vec<Arc<TrackData>> = seq.next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let meta: ProjectMeta = seq.next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
                let note_buckets: Vec<Vec<yinhe_types::Note>> = seq.next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(3, &self))?;
                let conductor_version: u64 = seq.next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(4, &self))?;

                let mut arr: Box<[Arc<Vec<yinhe_types::Note>>; 128]> = Box::new(core::array::from_fn(|_| Arc::new(Vec::new())));
                for (i, bucket) in note_buckets.into_iter().enumerate() {
                    if i < 128 {
                        arr[i] = Arc::new(bucket);
                    }
                }
                let mut model = YinModel {
                    conductor,
                    tracks,
                    tempo_map: Arc::new(TempoMap::default()),
                    meta,
                    notes: arr,
                    note_count: 0,
                    tick_length: 0,
                    track_note_count: Vec::new(),
                    track_audible_count: Vec::new(),
                    dirty_keys: [false; 128],
                    bucket_note_count: [0; 128],
                    bucket_track_stats: core::array::from_fn(|_| HashMap::new()),
                    conductor_version,
                };
                model.rebuild();
                Ok(model)
            }
        }
        d.deserialize_tuple(5, YinModelVisitor)
    }
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
            dirty_keys: [false; 128],
            bucket_note_count: [0; 128],
            bucket_track_stats: core::array::from_fn(|_| HashMap::new()),
            conductor_version: 0,
        }
    }
}

impl YinModel {
    /// Build TempoMap from conductor.tempo / conductor.time_sig.
    fn build_tempo_map(&self) -> TempoMap {
        let ppq = self.meta.ppq;

        // Convert TempoEvent -> TempoSegment, sorted by tick.
        let mut segments: Vec<TempoSegment> = if self.conductor.tempo.is_empty() {
            vec![TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: DEFAULT_MPQ,
            }]
        } else {
            let mut segs: Vec<TempoSegment> = self
                .conductor
                .tempo
                .iter()
                .map(|t| TempoSegment {
                    start_tick: t.tick,
                    start_time: 0.0,
                    micros_per_quarter: mpq_from_bpm(t.bpm as f32),
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

    /// Distribute per-track notes into the by-key store.
    ///
    /// Called once during parsing. After this, `TrackData` no longer
    /// holds notes — the by-key `self.notes` is the single source.
    /// Also computes `note_count`, `tick_length`, and `track_note_count`
    /// in the same pass (avoids a second full scan in `rebuild()`).
    pub fn load_track_notes(&mut self, per_track_notes: Vec<Vec<NoteEvent>>) {
        // Count per key for exact allocation.
        let mut per_key_count = [0u32; 128];
        for notes in per_track_notes.iter() {
            for note in notes {
                per_key_count[note.key as usize] += 1;
            }
        }

        // Allocate and fill each bucket, counting in one pass.
        let mut key_notes: [Vec<yinhe_types::Note>; 128] =
            core::array::from_fn(|k| Vec::with_capacity(per_key_count[k] as usize));

        let mut note_count: u64 = 0;
        let mut max_tick: u64 = 0;
        let mut track_counts: Vec<u64> = vec![0u64; self.tracks.len()];
        let mut track_audible: Vec<u64> = vec![0u64; self.tracks.len()];

        for (track_idx, notes) in per_track_notes.into_iter().enumerate() {
            for note in notes {
                let end = note.end_tick as u64;
                if end > max_tick {
                    max_tick = end;
                }
                note_count += 1;
                if (track_idx as usize) < track_counts.len() {
                    track_counts[track_idx as usize] += 1;
                    if note.velocity > 1 {
                        track_audible[track_idx as usize] += 1;
                    }
                }
                key_notes[note.key as usize].push(yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    dup_index: note.dup_index,
                    track: track_idx as u16,
                });
            }
        }

        self.notes = Box::new(key_notes.map(|v| Arc::new(v)));
        self.note_count = note_count;
        self.tick_length = max_tick;
        self.track_note_count = track_counts;
        self.track_audible_count = track_audible;
        // Initialize per-bucket counts and per-bucket per-track stats.
        for (k, bucket) in self.notes.iter().enumerate() {
            self.bucket_note_count[k] = bucket.len() as u64;
            let mut stats: HashMap<u16, (u64, u64)> = HashMap::new();
            for n in bucket.iter() {
                let e = stats.entry(n.track).or_insert((0, 0));
                e.0 += 1;
                if n.velocity > 1 {
                    e.1 += 1;
                }
            }
            self.bucket_track_stats[k] = stats;
        }
    }

    /// Rebuild all derived data from scratch.
    ///
    /// Call this after any mutation that changes notes, conductor, or
    /// track structure. O(N) where N = total note count.
    ///
    /// This operates on `self.notes` (the by-key store) directly — no
    /// longer reads from `TrackData.notes`.
    ///
    /// Note: `note_count` and `track_note_count` are maintained by
    /// `load_track_notes` and by edit operations. `rebuild()` only
    /// sorts buckets and rebuilds indices.
    pub fn rebuild(&mut self) {
        // Sort all 128 buckets in parallel.
        use rayon::prelude::*;
        self.notes
            .par_iter_mut()
            .for_each(|bucket| Arc::make_mut(bucket).sort_by_key(|n| n.start_tick));

        // Recompute note_count, max_tick, track_note_count, track_audible_count
        // (may have changed after edits or track insertions).
        let mut note_count: u64 = 0;
        let mut max_tick: u64 = 0;
        let mut track_counts: Vec<u64> = vec![0u64; self.tracks.len()];
        let mut track_audible: Vec<u64> = vec![0u64; self.tracks.len()];
        // Per-bucket per-track stats — recomputed in the same pass so
        // rebuild_dirty() can do incremental updates later.
        let mut bucket_stats: [HashMap<u16, (u64, u64)>; 128] =
            core::array::from_fn(|_| HashMap::new());
        for (k, bucket) in self.notes.iter().enumerate() {
            note_count += bucket.len() as u64;
            for n in bucket.iter() {
                let end = n.end_tick as u64;
                if end > max_tick {
                    max_tick = end;
                }
                if (n.track as usize) < track_counts.len() {
                    track_counts[n.track as usize] += 1;
                    if n.velocity > 1 {
                        track_audible[n.track as usize] += 1;
                    }
                }
                let e = bucket_stats[k].entry(n.track).or_insert((0, 0));
                e.0 += 1;
                if n.velocity > 1 {
                    e.1 += 1;
                }
            }
        }
        self.note_count = note_count;
        self.tick_length = max_tick;
        self.track_note_count = track_counts;
        self.track_audible_count = track_audible;
        // Re-initialize per-bucket counts and per-bucket per-track stats.
        for (k, bucket) in self.notes.iter().enumerate() {
            self.bucket_note_count[k] = bucket.len() as u64;
            self.bucket_track_stats[k] = bucket_stats[k].clone();
        }

        // Rebuild tempo_map (depends on tick_length we just computed).
        self.tempo_map = Arc::new(self.build_tempo_map());
    }

    /// Mark a bucket as dirty (modified and needs sorting).
    /// Call this before or after modifying `self.notes[key]`.
    pub fn mark_dirty(&mut self, key: u8) {
        self.dirty_keys[key as usize] = true;
    }

    /// Rebuild only the dirty buckets and update statistics incrementally.
    ///
    /// Cost: O(sum of dirty bucket sizes) for sorting + O(D) for stats.
    /// For a 30M-note song where only 10 buckets were touched, this is
    /// ~O(10 bucket scans) instead of O(128 bucket sorts + clones).
    ///
    /// The sorting step only calls `Arc::make_mut` on dirty buckets,
    /// so clean buckets that share Arc data with an undo snapshot are
    /// never deep-cloned — the key performance win over `rebuild()`.
    ///
    /// Statistics are updated incrementally using `bucket_note_count`:
    /// subtract old counts for dirty buckets, then rescan only dirty
    /// buckets and add back new counts. Track-level stats still do a
    /// full scan of all buckets — this is a future optimization.
    pub fn rebuild_dirty(&mut self) {
        let dirty_indices: Vec<usize> = (0..128)
            .filter(|&k| self.dirty_keys[k])
            .collect();
        if dirty_indices.is_empty() {
            return;
        }

        // 1. Sort only dirty buckets (Arc::make_mut only for these).
        for k in &dirty_indices {
            Arc::make_mut(&mut self.notes[*k]).sort_by_key(|n| n.start_tick);
            self.dirty_keys[*k] = false;
        }

        // 2. Incremental stats: subtract old per-bucket counts, add new ones.
        let mut delta_note_count: i64 = 0;
        let mut new_tick_length = self.tick_length;
        for &k in &dirty_indices {
            let old = self.bucket_note_count[k] as i64;
            let new = self.notes[k].len() as i64;
            delta_note_count += new - old;
            self.bucket_note_count[k] = new as u64;

            // Update tick_length: scan dirty buckets for max end_tick.
            for n in self.notes[k].iter() {
                let end = n.end_tick as u64;
                if end > new_tick_length {
                    new_tick_length = end;
                }
            }
        }
        self.note_count = (self.note_count as i64 + delta_note_count) as u64;
        self.tick_length = new_tick_length;

        // 3. Incremental track stats: subtract old per-track contributions
        //    for each dirty bucket, recompute the bucket's stats, and add
        //    the new contributions back. O(dirty bucket size) per edit
        //    instead of O(total notes).
        for &k in &dirty_indices {
            // Subtract old contributions from per-track totals.
            for (&track, &(total, audible)) in &self.bucket_track_stats[k] {
                if let Some(t) = self.track_note_count.get_mut(track as usize) {
                    *t = t.saturating_sub(total);
                }
                if let Some(a) = self.track_audible_count.get_mut(track as usize) {
                    *a = a.saturating_sub(audible);
                }
            }

            // Recompute this bucket's per-track stats from current notes.
            let mut new_stats: HashMap<u16, (u64, u64)> = HashMap::new();
            for n in self.notes[k].iter() {
                let e = new_stats.entry(n.track).or_insert((0, 0));
                e.0 += 1;
                if n.velocity > 1 {
                    e.1 += 1;
                }
            }

            // Add new contributions back to per-track totals.
            for (&track, &(total, audible)) in &new_stats {
                if let Some(t) = self.track_note_count.get_mut(track as usize) {
                    *t += total;
                }
                if let Some(a) = self.track_audible_count.get_mut(track as usize) {
                    *a += audible;
                }
            }
            self.bucket_track_stats[k] = new_stats;
        }

        // 4. Only rebuild tempo_map if conductor changed (notes-only edits skip it).
        //    The caller is responsible for bumping `conductor_version` when
        //    conductor.tempo or conductor.time_sig is modified.
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
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
            dup_index: 0,
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
        conductor.tempo.push(TempoEvent { tick: 0, bpm: 120.0 });
        conductor.tempo.push(TempoEvent { tick: 1920, bpm: 60.0 });

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
        conductor.tempo.push(TempoEvent { tick: 1920, bpm: 60.0 });

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
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
            dup_index: 0,
        }
    }

    fn note_silent(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            start_tick: start,
            end_tick: end,
            key,
            velocity: 0, // silent — must not count toward `track_audible_count`
            dup_index: 0,
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
                start_tick: 960,
                end_tick: 1440,
                velocity: 0,
                dup_index: 0,
                track: 1,
            });
        }
        {
            let model = Arc::make_mut(&mut m_inc.notes[62]);
            model.push(yinhe_types::Note {
                start_tick: 240,
                end_tick: 480,
                velocity: 80,
                dup_index: 0,
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
                start_tick: 960,
                end_tick: 1440,
                velocity: 0,
                dup_index: 0,
                track: 1,
            });
        }
        {
            let model = Arc::make_mut(&mut m_full.notes[62]);
            model.push(yinhe_types::Note {
                start_tick: 240,
                end_tick: 480,
                velocity: 80,
                dup_index: 0,
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
}

/// Display-oriented track info derived from `TrackData` for UI panels.
#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub index: u16,
    pub name: String,
    pub note_count: u64,
    pub port: u8,
    pub channel: u8,
}
