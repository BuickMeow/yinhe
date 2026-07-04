use serde::{Deserialize, Serialize};

/// A time signature event at a specific tick position.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    /// Denominator as power of 2: 2 means 4 (2^2).
    pub denominator: u8,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct Note {
    pub start_tick: u32,
    pub end_tick: u32,
    pub velocity: u8,
    /// Distinguishes overlapping notes at the same (track, key, start_tick).
    /// 99% of notes have dup_index == 0.
    pub dup_index: u8,
    pub track: u16,
}


