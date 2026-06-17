//! Bridge tests: assert that new yinhe-mid2 produces a YinModel
//! semantically equivalent to old yinhe-midi's MidiFile, on the same
//! .mid bytes.
//!
//! This file (and the dev-dependency on `yinhe-midi`) is DELETED on
//! switchover day, after the old parser is removed.
//!
//! Three comparisons (kept minimal — see docs/yinhe-architecture-v2.md):
//! 1. Note multiset (key, start, end, vel) equality
//! 2. TempoMap.tick_to_seconds equality at sampled ticks
//! 3. CC / PB multiset equality (with RPN sequences filtered from old
//!    side, since new parser consumes them into RpnEvent)

use std::collections::BTreeSet;

use yinhe_core::YinModel;
use yinhe_mid2::parse_bytes as parse_new;
use yinhe_midi::{MidiControlEvent, MidiFile};

// ─────────────────────────────────────────────────────────────────
//  Test fixtures
// ─────────────────────────────────────────────────────────────────

/// Single track, 1 note: C4 quarter at 120 BPM.
fn fixture_minimal() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
    data.extend_from_slice(b"MTrk");
    let track: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // 120 BPM
        0x00, 0x90, 60, 100,
        0x82, 0x40, 0x80, 60, 0,
        0x00, 0xFF, 0x2F, 0x00,
    ];
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(track);
    data
}

/// Multiple notes, multiple tracks (format-1), tempo + timesig changes,
/// CC + PB events. No RPN here (RPN tested separately in fixture_rpn).
fn fixture_complex() -> Vec<u8> {
    // Build via midly's writer to avoid hand-coding variable-length fields.
    use midly::num::{u15, u24, u4, u7};
    use midly::{
        Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent,
        TrackEventKind,
    };

    fn ev<'a>(delta: u32, kind: TrackEventKind<'a>) -> TrackEvent<'a> {
        TrackEvent {
            delta: delta.into(),
            kind,
        }
    }
    fn note_on(delta: u32, ch: u8, key: u8, vel: u8) -> TrackEvent<'static> {
        ev(
            delta,
            TrackEventKind::Midi {
                channel: u4::new(ch),
                message: MidiMessage::NoteOn {
                    key: u7::new(key),
                    vel: u7::new(vel),
                },
            },
        )
    }
    fn note_off(delta: u32, ch: u8, key: u8) -> TrackEvent<'static> {
        ev(
            delta,
            TrackEventKind::Midi {
                channel: u4::new(ch),
                message: MidiMessage::NoteOff {
                    key: u7::new(key),
                    vel: u7::new(0),
                },
            },
        )
    }
    fn cc(delta: u32, ch: u8, controller: u8, value: u8) -> TrackEvent<'static> {
        ev(
            delta,
            TrackEventKind::Midi {
                channel: u4::new(ch),
                message: MidiMessage::Controller {
                    controller: u7::new(controller),
                    value: u7::new(value),
                },
            },
        )
    }
    fn pb(delta: u32, ch: u8, bend: i16) -> TrackEvent<'static> {
        ev(
            delta,
            TrackEventKind::Midi {
                channel: u4::new(ch),
                message: MidiMessage::PitchBend {
                    bend: PitchBend::from_int(bend),
                },
            },
        )
    }

    // Conductor track
    let conductor: Vec<TrackEvent> = vec![
        ev(
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500_000))),
        ),
        ev(
            0,
            TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
        ),
        ev(
            1920,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(1_000_000))),
        ),
        ev(0, TrackEventKind::Meta(MetaMessage::EndOfTrack)),
    ];

    // Track 1: melody on channel 0 with CC + PB
    let mel: Vec<TrackEvent> = vec![
        ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"Lead"))),
        cc(0, 0, 7, 100),
        note_on(0, 0, 60, 110),
        note_off(480, 0, 60),
        note_on(0, 0, 64, 95),
        cc(120, 0, 11, 64),
        pb(120, 0, 2000),
        note_off(240, 0, 64),
        ev(0, TrackEventKind::Meta(MetaMessage::EndOfTrack)),
    ];

    // Track 2: bass on channel 1
    let bass: Vec<TrackEvent> = vec![
        ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"Bass"))),
        note_on(0, 1, 36, 100),
        note_off(1920, 1, 36),
        ev(0, TrackEventKind::Meta(MetaMessage::EndOfTrack)),
    ];

    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(u15::new(480)),
        },
        tracks: vec![conductor, mel, bass],
    };
    let mut buf = Vec::new();
    smf.write(&mut buf).unwrap();
    buf
}

/// Track with RPN sequence: CC101=0, CC100=0, CC6=2 (Pitch Bend Sensitivity)
/// plus a regular CC7. Used to verify RPN handling differs predictably.
fn fixture_rpn() -> Vec<u8> {
    use midly::num::{u15, u4, u7};
    use midly::{
        Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
    };

    fn ev<'a>(delta: u32, kind: TrackEventKind<'a>) -> TrackEvent<'a> {
        TrackEvent {
            delta: delta.into(),
            kind,
        }
    }
    fn cc(delta: u32, controller: u8, value: u8) -> TrackEvent<'static> {
        ev(
            delta,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::Controller {
                    controller: u7::new(controller),
                    value: u7::new(value),
                },
            },
        )
    }

    let track: Vec<TrackEvent> = vec![
        ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"RPN"))),
        cc(0, 7, 100), // plain CC7
        cc(100, 101, 0), // RPN MSB
        cc(0, 100, 0),   // RPN LSB
        cc(0, 6, 2),     // Data Entry MSB → forms RPN 0/0
        cc(100, 7, 80),  // plain CC7 again
        ev(0, TrackEventKind::Meta(MetaMessage::EndOfTrack)),
    ];
    let smf = Smf {
        header: Header {
            format: Format::SingleTrack,
            timing: Timing::Metrical(u15::new(480)),
        },
        tracks: vec![track],
    };
    let mut buf = Vec::new();
    smf.write(&mut buf).unwrap();
    buf
}

// ─────────────────────────────────────────────────────────────────
//  Helpers: extract semantic sets from old / new
// ─────────────────────────────────────────────────────────────────

/// Old MidiFile note multiset: (start_tick, end_tick, key, velocity).
/// Track index excluded — it's an indexing detail that legitimately
/// differs (old keeps conductor track, new skips it).
fn old_notes(midi: &MidiFile) -> Vec<(u32, u32, u8, u8)> {
    let mut out = Vec::new();
    for (key, notes) in midi.key_notes.iter().enumerate() {
        for n in notes {
            out.push((n.start_tick, n.end_tick, key as u8, n.velocity));
        }
    }
    out.sort();
    out
}

/// New YinModel note multiset: same shape, flattened across tracks.
fn new_notes(model: &YinModel) -> Vec<(u32, u32, u8, u8)> {
    let mut out = Vec::new();
    for t in &model.tracks {
        for n in &t.notes {
            out.push((n.start_tick, n.end_tick, n.key, n.velocity));
        }
    }
    out.sort();
    out
}

/// Old MidiFile CC events as (tick, controller, value), with RPN-related
/// CCs (101, 100, 6, 38) FILTERED at ticks where they form an RPN sequence.
/// We keep it simple: drop ALL CC101/100/6/38 — anything CC6/38 leaks
/// without a matching 101/100 is rare and skipping is fine for equivalence.
fn old_cc_filtered(midi: &MidiFile) -> Vec<(u32, u8, u8)> {
    let mut out = Vec::new();
    for ev in &midi.control_events {
        if let MidiControlEvent::ControlChange {
            tick,
            controller,
            value,
            ..
        } = ev
        {
            if matches!(*controller, 101 | 100 | 6 | 38) {
                continue;
            }
            out.push((*tick, *controller, *value));
        }
    }
    out.sort();
    out
}

/// New YinModel CC events as (tick, controller, value), flattened.
fn new_cc(model: &YinModel) -> Vec<(u32, u8, u8)> {
    let mut out = Vec::new();
    for t in &model.tracks {
        for (&controller, evs) in &t.cc {
            for e in evs {
                out.push((e.tick, controller, e.value));
            }
        }
    }
    out.sort();
    out
}

/// Old MidiFile pitch bend events as (tick, value).
fn old_pb(midi: &MidiFile) -> Vec<(u32, i16)> {
    let mut out = Vec::new();
    for ev in &midi.control_events {
        if let MidiControlEvent::PitchBend { tick, value, .. } = ev {
            out.push((*tick, *value));
        }
    }
    out.sort();
    out
}

/// New YinModel pitch bend events flattened.
fn new_pb(model: &YinModel) -> Vec<(u32, i16)> {
    let mut out = Vec::new();
    for t in &model.tracks {
        for e in &t.pitch_bend {
            out.push((e.tick, e.value));
        }
    }
    out.sort();
    out
}

/// New YinModel RPN events: (tick, msb, lsb, value) flattened.
fn new_rpn(model: &YinModel) -> BTreeSet<(u32, u8, u8, u16)> {
    let mut out = BTreeSet::new();
    for t in &model.tracks {
        for (&rpn_key, evs) in &t.rpn {
            let msb = ((rpn_key >> 8) & 0xFF) as u8;
            let lsb = (rpn_key & 0xFF) as u8;
            for e in evs {
                out.insert((e.tick, msb, lsb, e.value));
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────
//  Phase 0.1: Note multiset equivalence
// ─────────────────────────────────────────────────────────────────

#[test]
fn notes_match_minimal() {
    let bytes = fixture_minimal();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();
    assert_eq!(old_notes(&old), new_notes(&new), "minimal note multiset");
}

#[test]
fn notes_match_complex() {
    let bytes = fixture_complex();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();
    assert_eq!(old_notes(&old), new_notes(&new), "complex note multiset");
}

#[test]
fn notes_match_rpn_fixture() {
    let bytes = fixture_rpn();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();
    // RPN fixture has no notes
    assert_eq!(old_notes(&old), new_notes(&new));
}

// ─────────────────────────────────────────────────────────────────
//  Phase 0.2: TempoMap.tick_to_seconds equivalence
// ─────────────────────────────────────────────────────────────────

#[test]
fn tempo_map_matches_at_sampled_ticks() {
    let bytes = fixture_complex();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();

    // Sample 10 ticks across the song's range.
    let ticks: [u64; 10] = [0, 240, 480, 960, 1440, 1920, 2400, 3000, 3840, 4800];
    for &tk in &ticks {
        let old_secs = old.tick_to_seconds(tk);
        let new_secs = new.tempo_map.tick_to_seconds(tk);
        assert!(
            (old_secs - new_secs).abs() < 1e-6,
            "tick={} old={} new={}",
            tk,
            old_secs,
            new_secs
        );
    }
}

#[test]
fn tempo_map_matches_minimal() {
    let bytes = fixture_minimal();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();
    for &tk in &[0u64, 100, 320, 480, 1000] {
        let old_secs = old.tick_to_seconds(tk);
        let new_secs = new.tempo_map.tick_to_seconds(tk);
        assert!((old_secs - new_secs).abs() < 1e-6, "tick={}", tk);
    }
}

// ─────────────────────────────────────────────────────────────────
//  Phase 0.3: CC / PB equivalence (with RPN filtered)
// ─────────────────────────────────────────────────────────────────

#[test]
fn cc_match_complex() {
    let bytes = fixture_complex();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();
    assert_eq!(old_cc_filtered(&old), new_cc(&new), "CC multiset");
}

#[test]
fn pitch_bend_match_complex() {
    let bytes = fixture_complex();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();
    assert_eq!(old_pb(&old), new_pb(&new), "PB multiset");
}

#[test]
fn rpn_fixture_separates_correctly() {
    // Old keeps CC101/100/6 in control_events.
    // New consumes them into RpnEvent.
    // After RPN-filtering on the old side, plain CC7 events should match.
    let bytes = fixture_rpn();
    let old = MidiFile::load_from_bytes(&bytes).unwrap();
    let new = parse_new(&bytes).unwrap();

    // Plain CC stream equivalence (CC7 events only)
    assert_eq!(
        old_cc_filtered(&old),
        new_cc(&new),
        "non-RPN CC stream should match"
    );

    // Old has CC101/100/6 raw; new has them as RpnEvent
    let old_rpn_ccs: Vec<_> = old
        .control_events
        .iter()
        .filter_map(|e| match e {
            MidiControlEvent::ControlChange {
                tick,
                controller,
                value,
                ..
            } if matches!(*controller, 101 | 100 | 6) => Some((*tick, *controller, *value)),
            _ => None,
        })
        .collect();
    assert_eq!(old_rpn_ccs.len(), 3, "old expects CC101+CC100+CC6");

    let new_rpn = new_rpn(&new);
    assert_eq!(new_rpn.len(), 1, "new expects exactly 1 RpnEvent");
    let (tick, msb, lsb, value) = new_rpn.into_iter().next().unwrap();
    assert_eq!(tick, 100);
    assert_eq!(msb, 0);
    assert_eq!(lsb, 0);
    assert_eq!(value, 2);
}
