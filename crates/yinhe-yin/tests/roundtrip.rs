//! Round-trip tests: YinModel -> .yin bytes -> YinModel.

use std::collections::BTreeMap;
use std::sync::Arc;

use yinhe_core::{
    CcEvent, ConductorData, NoteEvent, PcEvent, PitchBendEvent, ProjectMeta, RpnEvent, TempoEvent,
    TimeSigEvent, TrackData, YinModel,
};
use yinhe_yin::{load_yin, load_yin_bytes, save_yin, save_yin_bytes};

fn build_complex_model() -> YinModel {
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
    t0.color = [0.8, 0.3, 0.2];
    t0.muted = false;
    t0.soloed = true;
    t0.notes = vec![
        NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 100, dup_index: 0 },
        NoteEvent { start_tick: 480, end_tick: 960, key: 64, velocity: 90, dup_index: 0 },
        NoteEvent { start_tick: 1000, end_tick: 1500, key: 60, velocity: 80, dup_index: 0 },
        NoteEvent { start_tick: 1000, end_tick: 1400, key: 60, velocity: 70, dup_index: 1 },
    ];
    let mut cc_map: BTreeMap<u8, Vec<CcEvent>> = BTreeMap::new();
    cc_map.insert(7, vec![CcEvent { tick: 0, value: 100 }, CcEvent { tick: 480, value: 80 }]);
    cc_map.insert(11, vec![CcEvent { tick: 100, value: 64 }]);
    t0.cc = cc_map;
    t0.pitch_bend = vec![
        PitchBendEvent { tick: 200, value: 2000 },
        PitchBendEvent { tick: 400, value: -1000 },
    ];
    t0.program_change = vec![PcEvent {
        tick: 0,
        program: 5,
        bank_msb: 0xFF,
        bank_lsb: 0xFF,
    }];
    let mut rpn_map: BTreeMap<u16, Vec<RpnEvent>> = BTreeMap::new();
    rpn_map.insert(0x0000, vec![RpnEvent { tick: 100, value: 2 }]);
    rpn_map.insert(0x0001, vec![RpnEvent { tick: 200, value: 8192 }]);
    t0.rpn = rpn_map;

    let mut t1 = TrackData::new(0, 1);
    t1.name = "Bass".to_string();
    t1.color = [0.2, 0.5, 0.9];
    t1.notes = vec![NoteEvent {
        start_tick: 0,
        end_tick: 1920,
        key: 36,
        velocity: 110,
        dup_index: 0,
    }];

    let mut t2 = TrackData::new(1, 9);
    t2.name = "Drums".to_string();
    t2.notes = vec![
        NoteEvent { start_tick: 0, end_tick: 60, key: 36, velocity: 127, dup_index: 0 },
        NoteEvent { start_tick: 240, end_tick: 300, key: 38, velocity: 100, dup_index: 0 },
    ];

    let meta = ProjectMeta {
        name: "My Black MIDI".to_string(),
        artist: "Jieneng".to_string(),
        description: "Test project".to_string(),
        ppq: 480,
        compression_level: 3,
    };

    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t0), Arc::new(t1), Arc::new(t2)],
        meta,
        ..Default::default()
    };
    model.rebuild();
    model
}

#[test]
fn roundtrip_in_memory() {
    let m1 = build_complex_model();
    let bytes = save_yin_bytes(&m1).unwrap();
    let m2 = load_yin_bytes(&bytes).unwrap();

    assert_eq!(m2.meta.name, "My Black MIDI");
    assert_eq!(m2.meta.artist, "Jieneng");
    assert_eq!(m2.meta.ppq, 480);

    assert_eq!(m2.conductor.tempo.len(), 2);
    assert!((m2.conductor.tempo[1].bpm - 60.0).abs() < 1e-6);
    assert_eq!(m2.conductor.time_sig.len(), 2);

    assert_eq!(m2.tracks.len(), 3);

    let lead = m2.tracks.iter().find(|t| t.name == "Lead").expect("Lead");
    assert_eq!(lead.color, [0.8, 0.3, 0.2]);
    assert!(lead.soloed);
    assert_eq!(lead.notes.len(), 4);
    assert!(lead.cc.contains_key(&7));
    assert_eq!(lead.cc[&7].len(), 2);
    assert!(lead.cc.contains_key(&11));
    assert_eq!(lead.pitch_bend.len(), 2);
    assert_eq!(lead.pitch_bend[1].value, -1000);
    assert_eq!(lead.program_change.len(), 1);
    assert_eq!(lead.program_change[0].program, 5);
    assert!(lead.rpn.contains_key(&0x0000));
    assert!(lead.rpn.contains_key(&0x0001));
    assert_eq!(lead.rpn[&0x0001][0].value, 8192);

    let bass = m2.tracks.iter().find(|t| t.name == "Bass").expect("Bass");
    assert_eq!(bass.notes[0].key, 36);
    assert_eq!(bass.channel, 1);
    assert_eq!(bass.port, 0);

    let drums = m2.tracks.iter().find(|t| t.name == "Drums").expect("Drums");
    assert_eq!(drums.port, 1);
    assert_eq!(drums.channel, 9);
    assert_eq!(drums.notes.len(), 2);

    assert_eq!(m2.note_count, 7);
    assert_eq!(m2.tick_length, 1920);
    assert_eq!(m2.key_notes_cache.len(), 128);
}

#[test]
fn roundtrip_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.yin");

    let m1 = build_complex_model();
    save_yin(&m1, &path).unwrap();
    assert!(path.exists());

    let m2 = load_yin(&path).unwrap();
    assert_eq!(m2.tracks.len(), m1.tracks.len());
    assert_eq!(m2.note_count, m1.note_count);
}

#[test]
fn empty_model_roundtrips() {
    let m1 = YinModel::default();
    let bytes = save_yin_bytes(&m1).unwrap();
    let m2 = load_yin_bytes(&bytes).unwrap();
    assert_eq!(m2.tracks.len(), 0);
    assert_eq!(m2.note_count, 0);
}

#[test]
fn bad_magic_rejected() {
    let mut bytes = save_yin_bytes(&YinModel::default()).unwrap();
    bytes[0] = b'X';
    let err = load_yin_bytes(&bytes).unwrap_err();
    assert!(matches!(err, yinhe_yin::YinError::BadMagic));
}

#[test]
fn truncated_rejected() {
    let bytes = save_yin_bytes(&YinModel::default()).unwrap();
    let truncated = &bytes[..bytes.len() - 4];
    let err = load_yin_bytes(truncated).unwrap_err();
    assert!(matches!(err, yinhe_yin::YinError::Truncated { .. }));
}

#[test]
fn project_json_is_human_readable() {
    let m = build_complex_model();
    let bytes = save_yin_bytes(&m).unwrap();
    let project_len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    let project_str = std::str::from_utf8(&bytes[10..10 + project_len]).unwrap();
    assert!(project_str.contains("My Black MIDI"));
    assert!(project_str.contains("Jieneng"));
}

#[test]
fn mapping_json_carries_track_metadata() {
    let m = build_complex_model();
    let bytes = save_yin_bytes(&m).unwrap();
    let project_len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    let mapping_len_at = 10 + project_len;
    let mapping_len = u32::from_le_bytes([
        bytes[mapping_len_at],
        bytes[mapping_len_at + 1],
        bytes[mapping_len_at + 2],
        bytes[mapping_len_at + 3],
    ]) as usize;
    let mapping_start = mapping_len_at + 4;
    let mapping_str =
        std::str::from_utf8(&bytes[mapping_start..mapping_start + mapping_len]).unwrap();
    assert!(mapping_str.contains("Lead"));
    assert!(mapping_str.contains("Bass"));
    assert!(mapping_str.contains("Drums"));
    assert!(mapping_str.contains("\"port\": 1"));
}

#[test]
fn dense_score_compresses_well() {
    let mut t = TrackData::new(0, 0);
    t.name = "Stress".to_string();
    t.notes = (0..100_000u32)
        .map(|i| NoteEvent {
            start_tick: i * 10,
            end_tick: i * 10 + 5,
            key: 60,
            velocity: 100,
            dup_index: 0,
        })
        .collect();
    let mut model = YinModel {
        tracks: vec![Arc::new(t)],
        ..Default::default()
    };
    model.meta.compression_level = 3;
    model.rebuild();

    let bytes = save_yin_bytes(&model).unwrap();
    // 100k * 13B = ~1.3 MB raw bincode. zstd should at least beat 50%
    // for highly repetitive data (same key/vel, monotonic ticks).
    assert!(
        bytes.len() < 600_000,
        ".yin compression unexpectedly poor: {} bytes (raw ~1.3 MB)",
        bytes.len()
    );

    // Roundtrip preserves count.
    let m2 = load_yin_bytes(&bytes).unwrap();
    assert_eq!(m2.tracks.len(), 1);
    assert_eq!(m2.tracks[0].notes.len(), 100_000);
    assert_eq!(m2.note_count, 100_000);
}

#[test]
fn version_bump_rejected() {
    let mut bytes = save_yin_bytes(&YinModel::default()).unwrap();
    // Set version to 999 (LE: 0xE7 0x03)
    bytes[4] = 0xE7;
    bytes[5] = 0x03;
    let err = load_yin_bytes(&bytes).unwrap_err();
    assert!(matches!(err, yinhe_yin::YinError::BadVersion(999)));
}

// ──────────────────────── SoundFont persistence ────────────────────────

use yinhe_yin::{
    ProjectSoundFonts, SfEntryJson, SfPortOverride, load_yin_bytes_with_sf,
    save_yin_bytes_with_sf,
};

fn sample_sf_state() -> ProjectSoundFonts {
    ProjectSoundFonts {
        mode: true,
        overrides: vec![
            SfPortOverride {
                port: 0,
                entries: vec![
                    SfEntryJson {
                        path: "/sf2/piano.sf2".to_string(),
                        name: "Piano".to_string(),
                        enabled: true,
                    },
                    SfEntryJson {
                        path: "/sf2/strings.sf2".to_string(),
                        name: "Strings".to_string(),
                        enabled: false,
                    },
                ],
            },
            SfPortOverride {
                port: 3,
                entries: vec![SfEntryJson {
                    path: "/sf2/drums.sf2".to_string(),
                    name: "Drums".to_string(),
                    enabled: true,
                }],
            },
        ],
    }
}

#[test]
fn sf_roundtrip_preserves_mode_and_entries() {
    let model = build_complex_model();
    let sf = sample_sf_state();

    let bytes = save_yin_bytes_with_sf(&model, &sf).unwrap();
    let (m2, sf2) = load_yin_bytes_with_sf(&bytes).unwrap();

    // Model is intact.
    assert_eq!(m2.tracks.len(), model.tracks.len());
    assert_eq!(m2.note_count, model.note_count);

    // SF state is intact.
    assert!(sf2.mode);
    assert_eq!(sf2.overrides.len(), 2);

    let p0 = &sf2.overrides[0];
    assert_eq!(p0.port, 0);
    assert_eq!(p0.entries.len(), 2);
    assert_eq!(p0.entries[0].path, "/sf2/piano.sf2");
    assert_eq!(p0.entries[0].name, "Piano");
    assert!(p0.entries[0].enabled);
    assert_eq!(p0.entries[1].path, "/sf2/strings.sf2");
    assert!(!p0.entries[1].enabled);

    let p3 = &sf2.overrides[1];
    assert_eq!(p3.port, 3);
    assert_eq!(p3.entries.len(), 1);
    assert_eq!(p3.entries[0].name, "Drums");
}

#[test]
fn sf_save_without_state_loads_as_empty() {
    // Old-style save (no SF) round-trips through with-sf load as default state.
    let model = build_complex_model();
    let bytes = save_yin_bytes(&model).unwrap();
    let (_m2, sf2) = load_yin_bytes_with_sf(&bytes).unwrap();
    assert!(!sf2.mode);
    assert!(sf2.overrides.is_empty());
}

#[test]
fn sf_save_with_state_loads_through_plain_load() {
    // SF-bearing file should still load fine through the plain `load_yin`
    // API (SF info is silently dropped).
    let model = build_complex_model();
    let sf = sample_sf_state();
    let bytes = save_yin_bytes_with_sf(&model, &sf).unwrap();
    let m2 = load_yin_bytes(&bytes).unwrap();
    assert_eq!(m2.tracks.len(), model.tracks.len());
    assert_eq!(m2.note_count, model.note_count);
}

#[test]
fn sf_global_mode_and_empty_overrides() {
    // mode=false (global mode) with empty overrides should also round-trip.
    let model = build_complex_model();
    let sf = ProjectSoundFonts {
        mode: false,
        overrides: vec![],
    };
    let bytes = save_yin_bytes_with_sf(&model, &sf).unwrap();
    let (_m2, sf2) = load_yin_bytes_with_sf(&bytes).unwrap();
    assert!(!sf2.mode);
    assert!(sf2.overrides.is_empty());
}

#[test]
fn sf_global_mode_preserves_overrides_list() {
    // User had per-port entries configured, then switched to global mode and
    // saved. The overrides list should still survive the round-trip so that
    // switching back to project mode restores their configuration.
    let model = build_complex_model();
    let sf = ProjectSoundFonts {
        mode: false, // global mode
        overrides: sample_sf_state().overrides, // but per-port list intact
    };
    let bytes = save_yin_bytes_with_sf(&model, &sf).unwrap();
    let (_m2, sf2) = load_yin_bytes_with_sf(&bytes).unwrap();
    assert!(!sf2.mode);
    assert_eq!(sf2.overrides.len(), 2);
    assert_eq!(sf2.overrides[0].entries[0].path, "/sf2/piano.sf2");
}
