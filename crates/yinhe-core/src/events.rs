use serde::{Deserialize, Serialize};

/// A note event stored per-track.
///
/// Channel / track are NOT stored here — they are implied by the owning
/// `TrackData`. This saves 3+ bytes per note in dense scores.
///
/// Memory representation uses `start_tick + end_tick` (rather than tick + duration)
/// for fast playback scheduling without addition.
///
/// `dup_index` distinguishes overlapping notes at the same `(key, start_tick)`.
/// 99% of notes have `dup_index == 0`. Selection identity is the compound key
/// `(track_idx, key, start_tick, dup_index)` — no separate UUID required.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct NoteEvent {
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub velocity: u8,
    pub dup_index: u8,
}


