/// A time signature event at a specific tick position.
#[derive(Clone, Debug)]
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
        channel: u8,
        controller: u8,
        value: u8,
    },
    ProgramChange {
        tick: u32,
        channel: u8,
        program: u8,
    },
    PitchBend {
        tick: u32,
        channel: u8,
        value: i16,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Note {
    pub key: u8,
    pub start: f64,
    pub end: f64,
    pub start_tick: u32,
    pub end_tick: u32,
    pub velocity: u8,
    pub channel: u8,
    pub track: u16,
}
