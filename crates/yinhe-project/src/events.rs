use serde::{Deserialize, Serialize};

use crate::varint::{read_varint, write_varint, zigzag_decode, zigzag_encode};

// ── Delta event trait ──

pub trait DeltaEvent: Sized {
    fn tick(&self) -> u32;
    fn set_tick(&mut self, tick: u32);
    fn encode_payload(&self, out: &mut Vec<u8>);
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self>;
}

pub fn encode_delta_events<T: DeltaEvent>(events: &[T]) -> Vec<u8> {
    let mut out = Vec::with_capacity(events.len() * 4 + 8);
    write_varint(&mut out, events.len() as u64);
    let mut prev_tick: u32 = 0;
    for e in events {
        let delta = e.tick().saturating_sub(prev_tick);
        write_varint(&mut out, delta as u64);
        e.encode_payload(&mut out);
        prev_tick = e.tick();
    }
    out
}

pub fn decode_delta_events<T: DeltaEvent>(buf: &[u8]) -> Vec<T> {
    let mut cursor = 0usize;
    let count = match read_varint(buf, &mut cursor) {
        Some(c) => c as usize,
        None => return Vec::new(),
    };
    let mut events = Vec::with_capacity(count);
    let mut prev_tick: u32 = 0;
    for _ in 0..count {
        let Some(delta) = read_varint(buf, &mut cursor) else { break };
        let Some(mut ev) = T::decode_payload(buf, &mut cursor) else { break };
        let tick = prev_tick.wrapping_add(delta as u32);
        ev.set_tick(tick);
        events.push(ev);
        prev_tick = tick;
    }
    events
}

// ── Track event types ──

/// A single note, stored per-track. Track/channel are in the file header.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Note {
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub velocity: u8,
}

/// FileHeader.version value indicating notes use the compact
/// delta-start + gate-duration varint encoding.
pub const NOTES_VERSION_DELTA_GATE: u8 = 2;

/// Encode notes using the unified delta-event format.
/// `notes` must be sorted by `start_tick`.
pub fn encode_notes_delta_gate(notes: &[Note]) -> Vec<u8> {
    encode_delta_events(notes)
}

/// Decode notes written by `encode_notes_delta_gate`.
pub fn decode_notes_delta_gate(buf: &[u8]) -> Vec<Note> {
    decode_delta_events(buf)
}

impl DeltaEvent for Note {
    fn tick(&self) -> u32 { self.start_tick }
    fn set_tick(&mut self, tick: u32) {
        let duration = self.end_tick.saturating_sub(self.start_tick);
        self.start_tick = tick;
        self.end_tick = tick.saturating_add(duration);
    }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        let duration = self.end_tick.saturating_sub(self.start_tick);
        write_varint(out, duration as u64);
        out.push(self.key);
        out.push(self.velocity);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        let duration = read_varint(buf, cursor)? as u32;
        if *cursor + 2 > buf.len() { return None; }
        let key = buf[*cursor];
        let velocity = buf[*cursor + 1];
        *cursor += 2;
        Some(Note { start_tick: 0, end_tick: duration, key, velocity })
    }
}

/// A control change event, stored per-CC-number.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CcEvent {
    pub tick: u32,
    pub value: u8,
}

impl DeltaEvent for CcEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) { out.push(self.value); }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor >= buf.len() { return None; }
        let value = buf[*cursor];
        *cursor += 1;
        Some(CcEvent { tick: 0, value })
    }
}

/// A pitch bend event.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PitchBendEvent {
    pub tick: u32,
    pub value: i16,
}

impl DeltaEvent for PitchBendEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        write_varint(out, zigzag_encode(self.value as i64));
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        let raw = read_varint(buf, cursor)?;
        let value = zigzag_decode(raw) as i16;
        Some(PitchBendEvent { tick: 0, value })
    }
}

/// A program change event, stored per-channel.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PcEvent {
    pub tick: u32,
    pub program: u8,
    pub bank_msb: u8,
    pub bank_lsb: u8,
}

impl DeltaEvent for PcEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        out.push(self.program);
        out.push(self.bank_msb);
        out.push(self.bank_lsb);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor + 3 > buf.len() { return None; }
        let program = buf[*cursor];
        let bank_msb = buf[*cursor + 1];
        let bank_lsb = buf[*cursor + 2];
        *cursor += 3;
        Some(PcEvent { tick: 0, program, bank_msb, bank_lsb })
    }
}

/// An RPN event, stored per-RPN-number.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RpnEvent {
    pub tick: u32,
    pub value: u16,
}

impl DeltaEvent for RpnEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        write_varint(out, self.value as u64);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        let value = read_varint(buf, cursor)? as u16;
        Some(RpnEvent { tick: 0, value })
    }
}

// ── Conductor event types ──

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f32,
}

impl DeltaEvent for TempoEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.bpm.to_le_bytes());
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor + 4 > buf.len() { return None; }
        let bpm = f32::from_le_bytes([buf[*cursor], buf[*cursor+1], buf[*cursor+2], buf[*cursor+3]]);
        *cursor += 4;
        Some(TempoEvent { tick: 0, bpm })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    /// Denominator as power of 2 (e.g. 2 means 4).
    pub denominator_power: u8,
}

impl DeltaEvent for TimeSigEvent {
    fn tick(&self) -> u32 { self.tick }
    fn set_tick(&mut self, tick: u32) { self.tick = tick; }
    fn encode_payload(&self, out: &mut Vec<u8>) {
        out.push(self.numerator);
        out.push(self.denominator_power);
    }
    fn decode_payload(buf: &[u8], cursor: &mut usize) -> Option<Self> {
        if *cursor + 2 > buf.len() { return None; }
        let numerator = buf[*cursor];
        let denominator_power = buf[*cursor + 1];
        *cursor += 2;
        Some(TimeSigEvent { tick: 0, numerator, denominator_power })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct KeySigEvent {
    pub tick: u32,
    /// Number of sharps (positive) or flats (negative).
    pub sf: i8,
    /// 0 = major, 1 = minor.
    pub mi: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextEvent {
    pub tick: u32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SmpteOffsetEvent {
    pub tick: u32,
    pub hr: u8,
    pub mn: u8,
    pub se: u8,
    pub fr: u8,
    pub ff: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_gate_roundtrip_basic() {
        let notes = vec![
            Note { start_tick: 0, end_tick: 480, key: 60, velocity: 100 },
            Note { start_tick: 240, end_tick: 720, key: 64, velocity: 90 },
            Note { start_tick: 480, end_tick: 960, key: 67, velocity: 80 },
            Note { start_tick: 1920, end_tick: 2400, key: 72, velocity: 110 },
        ];
        let encoded = encode_notes_delta_gate(&notes);
        let decoded = decode_notes_delta_gate(&encoded);
        assert_eq!(decoded.len(), notes.len());
        for (a, b) in notes.iter().zip(decoded.iter()) {
            assert_eq!(a.start_tick, b.start_tick);
            assert_eq!(a.end_tick, b.end_tick);
            assert_eq!(a.key, b.key);
            assert_eq!(a.velocity, b.velocity);
        }
    }

    #[test]
    fn delta_gate_empty() {
        let notes: Vec<Note> = Vec::new();
        let encoded = encode_notes_delta_gate(&notes);
        let decoded = decode_notes_delta_gate(&encoded);
        assert_eq!(decoded.len(), 0);
    }

    #[test]
    fn delta_gate_size_smaller_than_bincode_for_dense_notes() {
        let mut notes = Vec::with_capacity(1000);
        for i in 0..1000u32 {
            notes.push(Note {
                start_tick: i,
                end_tick: i + 10,
                key: 60,
                velocity: 100,
            });
        }
        let dg = encode_notes_delta_gate(&notes);
        let bc = bincode::serialize(&notes).unwrap();
        assert!(dg.len() < bc.len(), "delta+gate {} should be smaller than bincode {}", dg.len(), bc.len());
        assert!(dg.len() * 2 < bc.len(), "delta+gate {} should be < half of bincode {}", dg.len(), bc.len());
    }

    #[test]
    fn encode_delta_events_tempo_roundtrip() {
        let events = vec![
            TempoEvent { tick: 0, bpm: 120.0 },
            TempoEvent { tick: 480, bpm: 140.0 },
            TempoEvent { tick: 1920, bpm: 160.0 },
        ];
        let encoded = encode_delta_events(&events);
        let decoded: Vec<TempoEvent> = decode_delta_events(&encoded);
        assert_eq!(decoded.len(), events.len());
        for (a, b) in events.iter().zip(decoded.iter()) {
            assert_eq!(a.tick, b.tick);
            assert!((a.bpm - b.bpm).abs() < 0.01);
        }
    }

    #[test]
    fn encode_delta_events_empty() {
        let events: Vec<TempoEvent> = Vec::new();
        let encoded = encode_delta_events(&events);
        let decoded: Vec<TempoEvent> = decode_delta_events(&encoded);
        assert!(decoded.is_empty());
    }

    #[test]
    fn encode_delta_events_time_sig_roundtrip() {
        let events = vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator_power: 2 },
            TimeSigEvent { tick: 1920, numerator: 3, denominator_power: 2 },
        ];
        let encoded = encode_delta_events(&events);
        let decoded: Vec<TimeSigEvent> = decode_delta_events(&encoded);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].tick, 0);
        assert_eq!(decoded[0].numerator, 4);
        assert_eq!(decoded[0].denominator_power, 2);
        assert_eq!(decoded[1].tick, 1920);
        assert_eq!(decoded[1].numerator, 3);
        assert_eq!(decoded[1].denominator_power, 2);
    }

    #[test]
    fn encode_delta_events_cc_roundtrip() {
        let events = vec![
            CcEvent { tick: 0, value: 100 },
            CcEvent { tick: 100, value: 64 },
            CcEvent { tick: 200, value: 80 },
        ];
        let encoded = encode_delta_events(&events);
        let decoded: Vec<CcEvent> = decode_delta_events(&encoded);
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].tick, 0);
        assert_eq!(decoded[0].value, 100);
        assert_eq!(decoded[1].tick, 100);
        assert_eq!(decoded[2].tick, 200);
    }

    #[test]
    fn encode_delta_events_pitch_bend_roundtrip() {
        let events = vec![
            PitchBendEvent { tick: 0, value: 8192 },
            PitchBendEvent { tick: 480, value: 10000 },
        ];
        let encoded = encode_delta_events(&events);
        let decoded: Vec<PitchBendEvent> = decode_delta_events(&encoded);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].value, 8192);
        assert_eq!(decoded[1].value, 10000);
    }

    #[test]
    fn encode_delta_events_pc_roundtrip() {
        let events = vec![
            PcEvent { tick: 0, program: 5, bank_msb: 0, bank_lsb: 0 },
            PcEvent { tick: 480, program: 42, bank_msb: 0, bank_lsb: 0 },
        ];
        let encoded = encode_delta_events(&events);
        let decoded: Vec<PcEvent> = decode_delta_events(&encoded);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].program, 5);
        assert_eq!(decoded[1].program, 42);
    }

    #[test]
    fn encode_delta_events_rpn_roundtrip() {
        let events = vec![
            RpnEvent { tick: 0, value: 2 },
            RpnEvent { tick: 480, value: 0 },
        ];
        let encoded = encode_delta_events(&events);
        let decoded: Vec<RpnEvent> = decode_delta_events(&encoded);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].value, 2);
        assert_eq!(decoded[1].value, 0);
    }
}
