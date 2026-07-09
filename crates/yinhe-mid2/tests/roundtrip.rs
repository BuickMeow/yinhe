//! Round-trip tests: bytes -> YinModel -> bytes -> YinModel

use yinhe_core::{
    ConductorData, NoteEvent, PcEvent, ProjectMeta, TempoEvent, TrackData, YinModel,
};
use yinhe_types::TimeSigEvent;
use yinhe_mid2::{parse_bytes, write_to_bytes};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

/// Hand-craft minimal SMF bytes: 1 track, 1 note (C4 quarter note at 120 BPM).
fn minimal_midi_bytes() -> Vec<u8> {
    let mut data = Vec::new();
    // MThd: format 0, 1 track, 480 ppq
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
    // MTrk: tempo + NoteOn C4 + NoteOff C4
    data.extend_from_slice(b"MTrk");
    let track: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // 120 BPM (mpq=500_000)
        0x00, 0x90, 60, 100, // NoteOn key=60 vel=100
        0x82, 0x40, 0x80, 60, 0, // delta=320 (0x82 0x40 = 320), NoteOff
        0x00, 0xFF, 0x2F, 0x00, // EndOfTrack
    ];
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(track);
    data
}

#[test]
fn parse_minimal_midi() {
    let bytes = minimal_midi_bytes();
    let model = parse_bytes(&bytes).expect("parse failed");

    assert_eq!(model.meta.ppq, 480);
    assert_eq!(model.tracks.len(), 2);
    assert_eq!(model.note_count, 1);
    let n = &model.notes[60][0];
    assert_eq!(n.start_tick, 0);
    assert_eq!(n.end_tick, 320);
    assert_eq!(n.velocity, 100);
    assert_eq!(n.dup_index, 0);

    assert_eq!(model.conductor.tempo.len(), 1);
    assert!((model.conductor.tempo[0].bpm - 120.0).abs() < 0.5);

    assert_eq!(model.note_count, 1);
    assert_eq!(model.tick_length, 320);
    assert_eq!(model.notes[60].len(), 1);
}

#[test]
fn roundtrip_minimal_midi() {
    let bytes1 = minimal_midi_bytes();
    let model1 = parse_bytes(&bytes1).unwrap();
    let bytes2 = write_to_bytes(&model1).unwrap();
    let model2 = parse_bytes(&bytes2).unwrap();

    assert_eq!(model2.meta.ppq, model1.meta.ppq);
    assert_eq!(model2.tracks.len(), model1.tracks.len());
    assert_eq!(model2.note_count, model1.note_count);
    let n1 = &model1.notes[60][0];
    let n2 = &model2.notes[60][0];
    assert_eq!(n2.start_tick, n1.start_tick);
    assert_eq!(n2.end_tick, n1.end_tick);
    assert_eq!(n2.velocity, n1.velocity);
    assert_eq!(model2.conductor.tempo.len(), model1.conductor.tempo.len());
}

fn build_complex_model() -> YinModel {
    use std::sync::Arc;

    let conductor = ConductorData {
        tempo: vec![
            TempoEvent { tick: 0, bpm: 120.0 },
            TempoEvent { tick: 1920, bpm: 60.0 },
        ],
        time_sig: vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 3840, numerator: 3, denominator: 2 },
        ],
    };

    let mut t0 = TrackData::new(0, 0);
    t0.name = "Lead".to_string();
    let t0_notes = vec![
        NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 100, dup_index: 0 },
        NoteEvent { start_tick: 480, end_tick: 960, key: 64, velocity: 90, dup_index: 0 },
        NoteEvent { start_tick: 1000, end_tick: 1500, key: 60, velocity: 80, dup_index: 0 },
        NoteEvent { start_tick: 1000, end_tick: 1400, key: 60, velocity: 70, dup_index: 1 },
    ];
    t0.automation_lanes = vec![
        AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![
                AutomationEvent { tick: 0, value: 100, shape: SegmentShape::Step },
                AutomationEvent { tick: 480, value: 80, shape: SegmentShape::Step },
            ],
        },
        AutomationLane {
            target: AutomationTarget::PitchBend,
            track: 0,
            events: vec![
                AutomationEvent { tick: 200, value: 2000, shape: SegmentShape::Step },
            ],
        },
        AutomationLane {
            target: AutomationTarget::Rpn { parameter: 0x0000 },
            track: 0,
            events: vec![
                AutomationEvent { tick: 100, value: 2, shape: SegmentShape::Step },
            ],
        },
    ];
    t0.program_change = vec![PcEvent {
        tick: 0,
        program: 5,
        bank_msb: 0xFF,
        bank_lsb: 0xFF,
    }];

    let mut t1 = TrackData::new(0, 1);
    t1.name = "Bass".to_string();
    let t1_notes = vec![NoteEvent {
        start_tick: 0,
        end_tick: 1920,
        key: 36,
        velocity: 110,
        dup_index: 0,
    }];

    let per_track_notes = vec![t0_notes, t1_notes];

    let meta = ProjectMeta {
        ppq: 480,
        ..ProjectMeta::default()
    };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t0), Arc::new(t1)],
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

#[test]
fn roundtrip_complex_model_preserves_everything() {
    let model1 = build_complex_model();
    let bytes = write_to_bytes(&model1).unwrap();
    let model2 = parse_bytes(&bytes).unwrap();

    // Parser inserts a conductor track, so model2 has one more track.
    assert_eq!(model2.tracks.len(), model1.tracks.len() + 1);

    let l1 = &model1.tracks[0];
    let l2 = &model2.tracks[1];
    assert_eq!(model1.track_note_count[0], model2.track_note_count[1], "note count mismatch");
    assert_eq!(l2.name, "Lead");
    // Notes equal as a multiset of (start_tick, end_tick, key, velocity).
    // dup_index is a local-stable ordering and may differ across SMF
    // round-trips because the MIDI encoding doesn't preserve which note
    // was "first" among same-tick same-key onsets.
    let mut s1: Vec<_> = Vec::new();
    for (key, bucket) in model1.notes.iter().enumerate() {
        for n in bucket.iter().filter(|n| n.track == 0) {
            s1.push((n.start_tick, n.end_tick, key as u8, n.velocity));
        }
    }
    let mut s2: Vec<_> = Vec::new();
    for (key, bucket) in model2.notes.iter().enumerate() {
        for n in bucket.iter().filter(|n| n.track == 1) {
            s2.push((n.start_tick, n.end_tick, key as u8, n.velocity));
        }
    }
    s1.sort();
    s2.sort();
    assert_eq!(s1, s2, "note multiset differs");
    // Find lanes by target
    let find_lane = |target: &AutomationTarget| -> Option<&AutomationLane> {
        l2.automation_lanes.iter().find(|l| &l.target == target)
    };
    let cc7 = find_lane(&AutomationTarget::CC { controller: 7 }).expect("CC 7 lane");
    assert_eq!(cc7.events.len(), 2);
    assert_eq!(cc7.events[0].value, 100);
    assert_eq!(cc7.events[1].value, 80);
    let pb = find_lane(&AutomationTarget::PitchBend).expect("PitchBend lane");
    assert_eq!(pb.events.len(), 1);
    assert_eq!(pb.events[0].value, 2000);
    let rpn = find_lane(&AutomationTarget::Rpn { parameter: 0x0000 }).expect("RPN 0 lane");
    assert_eq!(rpn.events.len(), 1);
    assert_eq!(rpn.events[0].value, 2);
    assert_eq!(l2.program_change.len(), 1);
    assert_eq!(l2.program_change[0].program, 5);

    assert_eq!(model2.conductor.tempo.len(), 2);
    assert!((model2.conductor.tempo[1].bpm - 60.0).abs() < 0.5);
    assert_eq!(model2.conductor.time_sig.len(), 2);

    let b1 = &model1.tracks[1];
    let b2 = &model2.tracks[2];
    assert_eq!(model2.track_note_count[2], model1.track_note_count[1]);
    assert_eq!(b2.channel, 1);
}

/// Build SMF bytes with two NoteOns on key=60 at tick=0, then two NoteOffs.
fn build_overlap_midi_bytes() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
    data.extend_from_slice(b"MTrk");
    let track: &[u8] = &[
        0x00, 0x90, 60, 100, // NoteOn at tick 0, vel=100
        0x00, 0x90, 60, 80,  // NoteOn at tick 0 (overlap), vel=80
        0x81, 0x70, 0x80, 60, 0, // delta=240, NoteOff (matches second NoteOn LIFO)
        0x81, 0x70, 0x80, 60, 0, // delta=240 (tick=480), NoteOff (matches first NoteOn)
        0x00, 0xFF, 0x2F, 0x00,
    ];
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(track);
    data
}

#[test]
fn dup_index_assigned_for_overlapping_notes() {
    let bytes = build_overlap_midi_bytes();
    let model = parse_bytes(&bytes).expect("parse failed");
    assert_eq!(model.tracks.len(), 2);
    let key60_at_0: Vec<&yinhe_core::Note> = model.notes[60]
        .iter()
        .filter(|n| n.track == 1 && n.start_tick == 0)
        .collect();
    assert_eq!(key60_at_0.len(), 2, "expected two overlapping notes at tick 0");
    let dups: Vec<u8> = key60_at_0.iter().map(|n| n.dup_index).collect();
    assert!(dups.contains(&0));
    assert!(dups.contains(&1));
}

/// Build SMF bytes with a CC101+CC100+CC6 RPN sequence.
fn build_rpn_midi_bytes() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
    data.extend_from_slice(b"MTrk");
    let track: &[u8] = &[
        // RPN MSB=0 (CC101=0)
        0x00, 0xB0, 101, 0,
        // RPN LSB=0 (CC100=0) → selects RPN 0/0 (Pitch Bend Sensitivity)
        0x00, 0xB0, 100, 0,
        // Data Entry MSB=2 (CC6=2)
        0x00, 0xB0, 6, 2,
        0x00, 0xFF, 0x2F, 0x00,
    ];
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(track);
    data
}

#[test]
fn rpn_sequence_decodes_to_rpn_event() {
    let bytes = build_rpn_midi_bytes();
    let model = parse_bytes(&bytes).expect("parse failed");
    assert_eq!(model.tracks.len(), 2);
    let t = &model.tracks[1];
    // CC101/100/6 should NOT appear as plain CC lanes
    assert!(t.automation_lanes.iter().all(|l| match &l.target {
        AutomationTarget::CC { controller } => *controller != 101 && *controller != 100 && *controller != 6,
        _ => true,
    }));
    // Should have one RPN lane for parameter 0x0000
    let rpn_lane = t
        .automation_lanes
        .iter()
        .find(|l| l.target == AutomationTarget::Rpn { parameter: 0x0000 })
        .expect("RPN 0 lane");
    assert_eq!(rpn_lane.events.len(), 1);
    // CC6=2 is stored as 7-bit value: 2 (Pitch Bend Sensitivity, semitones)
    assert_eq!(rpn_lane.events[0].value, 2);
}

/// Build SMF with port + channel-prefix metas
fn build_port_channel_midi() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
    data.extend_from_slice(b"MTrk");
    let track: &[u8] = &[
        // MidiPort = 2 (FF 21 01 02)
        0x00, 0xFF, 0x21, 0x01, 0x02,
        // MidiChannel prefix = 5 (FF 20 01 05)
        0x00, 0xFF, 0x20, 0x01, 0x05,
        // NoteOn channel=3 key=60
        0x00, 0x93, 60, 100,
        0x81, 0x70, 0x83, 60, 0, // NoteOff
        0x00, 0xFF, 0x2F, 0x00,
    ];
    data.extend_from_slice(&(track.len() as u32).to_be_bytes());
    data.extend_from_slice(track);
    data
}

#[test]
fn port_and_channel_prefix_captured() {
    let bytes = build_port_channel_midi();
    let model = parse_bytes(&bytes).unwrap();
    let t = &model.tracks[1];
    assert_eq!(t.port, 2);
    assert_eq!(t.channel_prefix, Some(5));
    // First MIDI event uses channel 3 (raw); td.channel reflects that
    assert_eq!(t.channel, 3);
}
