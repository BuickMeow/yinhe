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

/// Control Change event. The CC controller number is the key in
/// `TrackData::cc: BTreeMap<u8, Vec<CcEvent>>`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct CcEvent {
    pub tick: u32,
    pub value: u8,
}

/// Pitch Bend event. Value range -8192 to +8191, 0 = center.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PitchBendEvent {
    pub tick: u32,
    pub value: i16,
}

/// Program Change event. Bank MSB/LSB are stored alongside for SF2 mapping.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PcEvent {
    pub tick: u32,
    pub program: u8,
    pub bank_msb: u8,
    pub bank_lsb: u8,
}

/// Registered Parameter Number event. The RPN selector is the key in
/// `TrackData::rpn: BTreeMap<u16, Vec<RpnEvent>>` where key = (msb << 8) | lsb.
///
/// Common RPNs:
/// - 0x0000: Pitch Bend Sensitivity (semitones)
/// - 0x0001: Fine Tune (14-bit, 8192 = 0 cents)
/// - 0x0002: Coarse Tune (14-bit, 8192 = 0 semitones)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct RpnEvent {
    pub tick: u32,
    pub value: u16,
}
