/// A note event stored per-track.
///
/// `tick` + `duration` is preferred over `start_tick` + `end_tick` because
/// moving a note only requires updating `tick`, leaving `duration` unchanged.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct NoteEvent {
    pub tick: u32,
    pub duration: u32,
    pub key: u8,
    pub velocity: u8,
}

/// A Control Change event. CC number is stored as the key in the track's
/// `BTreeMap<u8, Vec<CcEvent>>`, so the event body only carries tick + value.
#[derive(Clone, Copy, Debug, Default)]
pub struct CcEvent {
    pub tick: u32,
    pub value: u8,
}

/// A Pitch Bend event. Value range -8192 to +8191, 0 = center (no bend).
/// This matches the MIDI standard signed representation.
#[derive(Clone, Copy, Debug, Default)]
pub struct PitchBendEvent {
    pub tick: u32,
    pub value: i16,
}

/// A Program Change event.
#[derive(Clone, Copy, Debug, Default)]
pub struct PcEvent {
    pub tick: u32,
    pub program: u8,
    pub bank_msb: u8,
    pub bank_lsb: u8,
}

/// A Registered Parameter Number event.
///
/// Supported RPNs:
/// - 0: Pitch Bend Sensitivity (value = semitones, 0–127)
/// - 1: Fine Tune (value = 14-bit, 8192 = 0 cents)
/// - 2: Coarse Tune (value = 14-bit, 8192 = 0 semitones)
#[derive(Clone, Copy, Debug, Default)]
pub struct RpnEvent {
    pub tick: u32,
    pub value: u16,
}
