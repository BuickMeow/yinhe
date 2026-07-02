use serde::{Deserialize, Serialize};
use crate::NoteSource;

/// A time signature event at a specific tick position.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    /// Denominator as power of 2: 2 means 4 (2^2).
    pub denominator: u8,
}

/// Non-note MIDI events (CC, Program Change, Pitch Bend) stored for audio synthesis.
#[derive(Clone, Debug)]
pub enum MidiControlEvent {
    ControlChange {
        tick: u32,
        controller: u8,
        value: u8,
        track: u16,
    },
    ProgramChange {
        tick: u32,
        program: u8,
        track: u16,
    },
    PitchBend {
        tick: u32,
        value: i16,
        track: u16,
    },
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


