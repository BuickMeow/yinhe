use yinhe_project::conversion::{midi_to_archive, archive_to_midi};
use yinhe_midi::{MidiFile, MidiControlEvent};
use yinhe_test_helpers::*;

#[test]
fn roundtrip_notes_and_channels() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);

    assert_eq!(restored.ticks_per_beat, 480);
    assert_eq!(restored.track_ports, vec![0, 0, 1]);
    assert_eq!(restored.key_notes[60].len(), 2);
    assert!(restored.key_notes[60].iter().all(|n| n.track == 0));
    assert_eq!(restored.key_notes[48].len(), 1);
    assert_eq!(restored.key_notes[48][0].track, 1);
    assert_eq!(restored.key_notes[36].len(), 1);
    assert_eq!(restored.key_notes[36][0].track, 2);
}

#[test]
fn roundtrip_control_events() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);

    let cc_count = restored.control_events.iter()
        .filter(|e| matches!(e, MidiControlEvent::ControlChange { .. }))
        .count();
    assert_eq!(cc_count, 2);
    let pb_count = restored.control_events.iter()
        .filter(|e| matches!(e, MidiControlEvent::PitchBend { .. }))
        .count();
    assert_eq!(pb_count, 1);
    let pc_count = restored.control_events.iter()
        .filter(|e| matches!(e, MidiControlEvent::ProgramChange { .. }))
        .count();
    assert_eq!(pc_count, 1);
}

#[test]
fn roundtrip_tempo_and_time_sig() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);

    let bpm0 = yinhe_midi::bpm_from_mpq(restored.tempo_segments[0].micros_per_quarter);
    assert!((bpm0 - 120.0).abs() < 0.5, "expected ~120 BPM, got {bpm0}");
    assert_eq!(restored.time_sig_events.len(), 2);
    assert_eq!(restored.time_sig_events[0].numerator, 4);
    assert_eq!(restored.time_sig_events[1].numerator, 3);
}

#[test]
fn roundtrip_track_names() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);
    assert_eq!(restored.track_names, vec!["Lead", "Bass", "Drums"]);
}

#[test]
fn roundtrip_preserves_tick_length() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);
    assert_eq!(restored.tick_length, original.tick_length);
}

#[test]
fn archive_json_roundtrip() {
    use yinhe_project::schema::ProjectJson;
    use yinhe_project::header::FileHeader;

    let original = make_test_model();
    let mut archive = midi_to_archive(&original);

    let proj = ProjectJson {
        version: 1,
        name: "Test Project".to_string(),
        artist: "Test Artist".to_string(),
        ppq: 480,
        zstd_level: 0,
        description: "Test description".to_string(),
        soundfont_project_mode: false,
        soundfont_overrides: Vec::new(),
    };
    archive.set_json("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &proj);

    let loaded: ProjectJson = archive.get_json("project.json").expect("get_json failed");
    assert_eq!(loaded.name, "Test Project");
    assert_eq!(loaded.artist, "Test Artist");
    assert_eq!(loaded.ppq, 480);
}

#[test]
fn archive_empty_midi_roundtrip() {
    let m = MidiFile::default();
    let archive = midi_to_archive(&m);
    let restored = archive_to_midi(&archive);
    assert_eq!(restored.note_count, 0);
}

#[test]
fn archive_stress_many_notes() {
    let original = make_stress_midi(4, 500);
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);
    assert_eq!(restored.note_count, 2000);
}

#[test]
fn roundtrip_pitchbend_value() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);
    let restored = archive_to_midi(&archive);

    let pb = restored.control_events.iter().find_map(|e| match e {
        MidiControlEvent::PitchBend { tick, value, track } => Some((*tick, *value, *track)),
        _ => None,
    }).unwrap();
    assert_eq!(pb, (100, 1024, 1));
}

#[test]
fn roundtrip_rpn_events() {
    let mut m = make_test_model();
    m.control_events.push(MidiControlEvent::ControlChange {
        tick: 100, controller: 6, value: 2, track: 0,
    });
    m.control_events.push(MidiControlEvent::ControlChange {
        tick: 100, controller: 101, value: 0, track: 0,
    });
    m.control_events.push(MidiControlEvent::ControlChange {
        tick: 100, controller: 100, value: 0, track: 0,
    });

    let archive = midi_to_archive(&m);
    let restored = archive_to_midi(&archive);

    let rpn_ccs: Vec<_> = restored.control_events.iter()
        .filter_map(|ev| match ev {
            MidiControlEvent::ControlChange { controller: 101 | 100 | 6, .. } => Some(ev),
            _ => None,
        })
        .collect();
    assert_eq!(rpn_ccs.len(), 3, "expected 3 RPN-related CCs");
}

#[test]
fn archive_write_to_file_and_read_back() {
    let original = make_test_model();
    let archive = midi_to_archive(&original);

    let tmp = tempfile::NamedTempFile::new().expect("tmp file");
    let path = tmp.path().to_str().unwrap();
    archive.write_to(path).expect("write_to failed");

    let loaded = yinhe_project::ProjectArchive::read_from(path).expect("read_from failed");
    let restored = archive_to_midi(&loaded);
    assert_eq!(restored.track_names, vec!["Lead", "Bass", "Drums"]);
    assert_eq!(restored.note_count, 4);
}
