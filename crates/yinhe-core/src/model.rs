//! YinModel + TrackData + ConductorData + ProjectMeta + rebuild logic.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::events::{CcEvent, NoteEvent, PcEvent, PitchBendEvent, RpnEvent};
use crate::tempo_map::{
    DEFAULT_MPQ, TempoMap, TempoSegment, mpq_from_bpm, recompute_tempo_start_times,
};

// =========================================================
//  Conductor
// =========================================================

/// Tempo event (BPM at a specific tick).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f64,
}

/// Time signature event (denominator stored as power of 2: 2 means 4 = 2^2).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    pub denominator: u8,
}

/// Global score-level events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConductorData {
    pub tempo: Vec<TempoEvent>,
    pub time_sig: Vec<TimeSigEvent>,
}

// =========================================================
//  TrackData
// =========================================================

/// One MIDI track's complete data.
///
/// Channel/track are held here, not in individual events. NoteEvent is
/// looked up by `(track_idx, key, start_tick, dup_index)`.
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

    pub notes: Vec<NoteEvent>,
    pub cc: BTreeMap<u8, Vec<CcEvent>>,
    pub pitch_bend: Vec<PitchBendEvent>,
    pub program_change: Vec<PcEvent>,
    /// RPN keyed by (msb << 8) | lsb.
    pub rpn: BTreeMap<u16, Vec<RpnEvent>>,
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
            cc: BTreeMap::new(),
            pitch_bend: Vec::new(),
            program_change: Vec::new(),
            rpn: BTreeMap::new(),
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
/// Tracks are held in `Arc<TrackData>` for cheap clone-on-write editing
/// (C1 mode). The conductor is also Arc to avoid copying it when only
/// a track changes. tempo_map is derived from conductor.
#[derive(Clone, Debug)]
pub struct YinModel {
    pub conductor: Arc<ConductorData>,
    pub tracks: Vec<Arc<TrackData>>,
    pub tempo_map: Arc<TempoMap>,
    pub meta: ProjectMeta,

    // Derived caches (rebuilt by `rebuild()`).
    /// `key_notes_cache[k]` = all notes with `note.key == k`, sorted by start_tick.
    /// Compatible with `yinhe_types::NoteSource`.
    pub key_notes_cache: Vec<Vec<yinhe_types::Note>>,
    pub note_count: u64,
    pub tick_length: u64,

    // Optional indices for fast range queries.
    pub scan_index: Option<yinhe_types::NoteScanIndex>,
    pub tick_buckets: Option<yinhe_types::TickBuckets>,
}

impl Default for YinModel {
    fn default() -> Self {
        Self {
            conductor: Arc::new(ConductorData::default()),
            tracks: Vec::new(),
            tempo_map: Arc::new(TempoMap::default()),
            meta: ProjectMeta::default(),
            key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
            note_count: 0,
            tick_length: 0,
            scan_index: None,
            tick_buckets: None,
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

        // Convert TimeSigEvent -> yinhe_types::TimeSigEvent.
        let mut ts_events: Vec<yinhe_types::TimeSigEvent> = self
            .conductor
            .time_sig
            .iter()
            .map(|ts| yinhe_types::TimeSigEvent {
                tick: ts.tick,
                numerator: ts.numerator,
                denominator: ts.denominator,
            })
            .collect();
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

    /// Rebuild all derived data from scratch.
    ///
    /// Call this after any mutation that changes notes, conductor, or
    /// track structure. O(N) where N = total note count.
    pub fn rebuild(&mut self) {
        // Pass 1: scan all notes, build per-key buckets, compute counts.
        let mut key_notes: [Vec<yinhe_types::Note>; 128] =
            core::array::from_fn(|_| Vec::new());
        let mut note_count: u64 = 0;
        let mut max_tick: u64 = 0;

        for (track_idx, track) in self.tracks.iter().enumerate() {
            for note in &track.notes {
                note_count += 1;
                let end = note.end_tick as u64;
                if end > max_tick {
                    max_tick = end;
                }
                key_notes[note.key as usize].push(yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    track: track_idx as u16,
                });
            }
        }
        for bucket in key_notes.iter_mut() {
            bucket.sort_by_key(|n| n.start_tick);
        }

        self.note_count = note_count;
        self.tick_length = max_tick;

        // Pass 2: build scan_index + tick_buckets.
        const BUCKET_SIZE: u32 = 65536;
        let scan_index = yinhe_types::NoteScanIndex::build(&key_notes, max_tick);
        let tick_buckets = yinhe_types::TickBuckets::build(&key_notes, max_tick, BUCKET_SIZE);

        self.scan_index = Some(scan_index);
        self.tick_buckets = Some(tick_buckets);
        self.key_notes_cache = key_notes.to_vec();

        // Pass 3: rebuild tempo_map (depends on tick_length we just computed).
        self.tempo_map = Arc::new(self.build_tempo_map());
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

    fn track_with_notes(notes: Vec<NoteEvent>) -> TrackData {
        let mut t = TrackData::new(0, 0);
        t.notes = notes;
        t
    }

    #[test]
    fn empty_model_rebuild() {
        let mut m = YinModel::default();
        m.rebuild();
        assert_eq!(m.note_count, 0);
        assert_eq!(m.tick_length, 0);
        assert_eq!(m.key_notes_cache.len(), 128);
        assert!(m.key_notes_cache.iter().all(|v| v.is_empty()));
    }

    #[test]
    fn rebuild_counts_and_buckets_notes() {
        let t = track_with_notes(vec![
            note(0, 480, 60),
            note(480, 960, 64),
            note(960, 1920, 60),
        ]);
        let mut m = YinModel {
            tracks: vec![Arc::new(t)],
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.note_count, 3);
        assert_eq!(m.tick_length, 1920);
        assert_eq!(m.key_notes_cache[60].len(), 2);
        assert_eq!(m.key_notes_cache[64].len(), 1);
        assert_eq!(m.key_notes_cache[60][0].start_tick, 0);
        assert_eq!(m.key_notes_cache[60][1].start_tick, 960);
    }

    #[test]
    fn rebuild_sorts_per_key() {
        // Insert notes out-of-order; cache should be sorted by start_tick.
        let t = track_with_notes(vec![note(960, 1920, 60), note(0, 480, 60)]);
        let mut m = YinModel {
            tracks: vec![Arc::new(t)],
            ..Default::default()
        };
        m.rebuild();
        assert_eq!(m.key_notes_cache[60][0].start_tick, 0);
        assert_eq!(m.key_notes_cache[60][1].start_tick, 960);
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
