use std::collections::BTreeMap;

use crate::events::*;

// ═══════════════════════════════════════════════════════════════
//  YinModel — the single source of truth
// ═══════════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct YinModel {
    pub conductor: ConductorData,
    pub tracks: Vec<TrackData>,
    pub meta: ProjectMeta,
    /// Derived key-based index for fast Piano Roll access.
    pub key_index: KeyIndex,
    /// Cached key_notes array for NoteSource compatibility.
    /// Rebuilt during rebuild(). Index = MIDI key (0-127).
    pub key_notes_cache: Vec<Vec<yinhe_types::Note>>,
    pub note_count: u64,
    pub tick_length: u64,
}

// ═══════════════════════════════════════════════════════════════
//  Conductor — global events (tempo, time signature)
// ═══════════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct ConductorData {
    pub tempo: Vec<TempoEvent>,
    pub time_sig: Vec<TimeSigEvent>,
}

#[derive(Clone, Copy, Debug)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    pub denominator: u8,
}

// ═══════════════════════════════════════════════════════════════
//  Track — per-channel event container
// ═══════════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct TrackData {
    pub uuid: String,
    pub name: String,
    pub port: u8,
    pub channel: u8,
    pub notes: Vec<NoteEvent>,
    pub cc: BTreeMap<u8, Vec<CcEvent>>,
    pub pitch_bend: Vec<PitchBendEvent>,
    pub program_change: Vec<PcEvent>,
    pub rpn: BTreeMap<u8, Vec<RpnEvent>>,
}

// ═══════════════════════════════════════════════════════════════
//  Project metadata
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
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
            compression_level: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  KeyIndex — derived per-key index for Piano Roll
// ═══════════════════════════════════════════════════════════════

/// A reference to a note inside `TrackData::notes`.
#[derive(Clone, Copy)]
pub struct NoteRef {
    pub track: u16,
    pub note_idx: u32,
}

#[derive(Clone)]
pub struct KeyIndex {
    /// `notes_by_key[k]` holds all notes on MIDI key `k`, sorted by tick.
    pub notes_by_key: Vec<Vec<NoteRef>>,
    pub scan_index: Option<yinhe_types::NoteScanIndex>,
    pub tick_buckets: Option<yinhe_types::TickBuckets>,
}

impl Default for KeyIndex {
    fn default() -> Self {
        Self {
            notes_by_key: (0..128).map(|_| Vec::new()).collect(),
            scan_index: None,
            tick_buckets: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  YinModel methods
// ═══════════════════════════════════════════════════════════════

impl YinModel {
    /// Rebuild all derived indices from the track data.
    pub fn rebuild(&mut self) {
        self.note_count = 0;
        let mut max_tick = 0u64;
        let mut notes_by_key: Vec<Vec<NoteRef>> = (0..128).map(|_| Vec::new()).collect();
        let mut key_notes: [Vec<yinhe_types::Note>; 128] = core::array::from_fn(|_| Vec::new());

        for (track_idx, track) in self.tracks.iter().enumerate() {
            for (note_idx, note) in track.notes.iter().enumerate() {
                self.note_count += 1;
                let end = (note.tick + note.duration) as u64;
                if end > max_tick {
                    max_tick = end;
                }
                notes_by_key[note.key as usize].push(NoteRef {
                    track: track_idx as u16,
                    note_idx: note_idx as u32,
                });
                key_notes[note.key as usize].push(yinhe_types::Note {
                    start_tick: note.tick,
                    end_tick: note.tick + note.duration,
                    velocity: note.velocity,
                    track: track_idx as u16,
                });
            }
        }

        self.tick_length = max_tick;

        // Sort each key bucket by tick
        for (key, bucket) in notes_by_key.iter_mut().enumerate() {
            bucket.sort_by_key(|r| {
                let track = &self.tracks[r.track as usize];
                track.notes[r.note_idx as usize].tick
            });
            key_notes[key].sort_by_key(|n| n.start_tick);
        }

        // Build scan_index and tick_buckets using existing yinhe-types infrastructure
        const BUCKET_SIZE: u32 = 65536;
        let scan_index = yinhe_types::NoteScanIndex::build(&key_notes, max_tick);
        let tick_buckets = yinhe_types::TickBuckets::build(&key_notes, max_tick, BUCKET_SIZE);

        self.key_index = KeyIndex {
            notes_by_key,
            scan_index: Some(scan_index),
            tick_buckets: Some(tick_buckets),
        };

        // Store key_notes cache for NoteSource compatibility
        self.key_notes_cache = key_notes.to_vec();
    }

    /// Get notes for a given key, as (track_idx, NoteEvent) pairs.
    pub fn notes_for_key(&self, key: u8) -> Vec<(u16, &NoteEvent)> {
        self.key_index.notes_by_key[key as usize]
            .iter()
            .map(|r| {
                let track = &self.tracks[r.track as usize];
                (r.track, &track.notes[r.note_idx as usize])
            })
            .collect()
    }
}
