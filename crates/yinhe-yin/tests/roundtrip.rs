//! Round-trip tests: YinModel -> .yin bytes -> YinModel.

use std::sync::Arc;

use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};
use yinhe_yin::{load_yin, load_yin_bytes, save_yin, save_yin_bytes};

fn build_complex_model() -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 1920,
                    value: 60.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        time_sig: vec![
            TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            },
            TimeSigEvent {
                tick: 3840,
                numerator: 3,
                denominator: 2,
            },
        ],
        key_sig: vec![
            yinhe_types::KeySigEvent {
                tick: 0,
                root: 0, // C
                scale: yinhe_types::ScaleType::Major,
            },
            yinhe_types::KeySigEvent {
                tick: 1920,
                root: 9, // A
                scale: yinhe_types::ScaleType::NaturalMinor,
            },
        ],
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };

    let mut t0 = TrackData::new(0, 0);
    t0.name = "Lead".to_string();
    t0.color = [0.8, 0.3, 0.2, 1.0];
    t0.muted = false;
    t0.soloed = true;
    let t0_notes = vec![
        NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        },
        NoteEvent {
            id: 0,
            start_tick: 480,
            end_tick: 960,
            key: 64,
            velocity: 90,
        },
        NoteEvent {
            id: 0,
            start_tick: 1000,
            end_tick: 1500,
            key: 60,
            velocity: 80,
        },
        NoteEvent {
            id: 0,
            start_tick: 1000,
            end_tick: 1400,
            key: 60,
            velocity: 70,
        },
    ];
    t0.automation_lanes = vec![
        AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 0,
                    value: 100.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 480,
                    value: 80.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: AutomationTarget::CC { controller: 11 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 100,
                value: 64.0,
                shape: SegmentShape::Step,
            }],
        },
        AutomationLane {
            target: AutomationTarget::PitchBend,
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 200,
                    value: 2000.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 400,
                    value: 1000.0,
                    shape: SegmentShape::Step,
                }, // 8192 - 1000 = 7192 → 1000
            ],
        },
        AutomationLane {
            target: AutomationTarget::Rpn { parameter: 0x0000 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 100,
                value: 2.0,
                shape: SegmentShape::Step,
            }],
        },
        AutomationLane {
            target: AutomationTarget::Rpn { parameter: 0x0001 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 200,
                value: 8192.0,
                shape: SegmentShape::Step,
            }],
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
    t1.color = [0.2, 0.5, 0.9, 1.0];
    let t1_notes = vec![NoteEvent {
        id: 0,
        start_tick: 0,
        end_tick: 1920,
        key: 36,
        velocity: 110,
    }];

    let mut t2 = TrackData::new(1, 9);
    t2.name = "Drums".to_string();
    let t2_notes = vec![
        NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 60,
            key: 36,
            velocity: 127,
        },
        NoteEvent {
            id: 0,
            start_tick: 240,
            end_tick: 300,
            key: 38,
            velocity: 100,
        },
    ];

    let per_track_notes = vec![t0_notes, t1_notes, t2_notes];

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
    model.load_track_notes(per_track_notes);
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

    assert_eq!(m2.conductor.tempo.events.len(), 2);
    assert!((m2.conductor.tempo.events[1].value - 60.0).abs() < 1e-6);
    assert_eq!(m2.conductor.time_sig.len(), 2);

    // 回归：key_sig 非空时必须能完成 postcard 往返（曾经 untagged 导致 deserialize_any 失败）
    assert_eq!(m2.conductor.key_sig.len(), 2);
    assert_eq!(m2.conductor.key_sig[0].root, 0);
    assert_eq!(m2.conductor.key_sig[0].scale, yinhe_types::ScaleType::Major);
    assert_eq!(m2.conductor.key_sig[1].tick, 1920);
    assert_eq!(
        m2.conductor.key_sig[1].scale,
        yinhe_types::ScaleType::NaturalMinor
    );

    // id 不序列化：加载后重新分配，从 1 开始且全局唯一（发号器推进到 max+1）
    let mut ids: Vec<u32> = m2
        .notes
        .iter()
        .flat_map(|b| b.iter().map(|n| n.id))
        .collect();
    let id_count = ids.len();
    ids.sort_unstable();
    assert_eq!(ids.first(), Some(&1), "id 应从 1 开始重新分配");
    ids.dedup();
    assert_eq!(id_count, m2.note_count as usize);
    assert_eq!(ids.len(), id_count, "id 必须全局唯一");
    assert_eq!(
        m2.next_note_id,
        m2.note_count as u32 + 1,
        "发号器应推进到 max+1"
    );

    assert_eq!(m2.tracks.len(), 3);

    let lead = m2.tracks.iter().find(|t| t.name == "Lead").expect("Lead");
    let lead_idx = m2.tracks.iter().position(|t| t.name == "Lead").unwrap() as u16;
    assert_eq!(lead.color, [0.8, 0.3, 0.2, 1.0]);
    assert!(lead.soloed);
    assert_eq!(m2.track_note_count[lead_idx as usize], 4);

    // Check CC automation lanes
    let cc7 = lead
        .automation_lanes
        .iter()
        .find(|l| l.target == AutomationTarget::CC { controller: 7 })
        .expect("CC 7 lane");
    assert_eq!(cc7.events.len(), 2);

    let cc11 = lead
        .automation_lanes
        .iter()
        .find(|l| l.target == AutomationTarget::CC { controller: 11 })
        .expect("CC 11 lane");
    assert_eq!(cc11.events.len(), 1);

    // Check PitchBend automation lane
    let pb = lead
        .automation_lanes
        .iter()
        .find(|l| l.target == AutomationTarget::PitchBend)
        .expect("PitchBend lane");
    assert_eq!(pb.events.len(), 2);
    assert_eq!(pb.events[1].value, 1000.0);

    assert_eq!(lead.program_change.len(), 1);
    assert_eq!(lead.program_change[0].program, 5);

    // Check RPN automation lanes
    let rpn0 = lead
        .automation_lanes
        .iter()
        .find(|l| l.target == AutomationTarget::Rpn { parameter: 0x0000 })
        .expect("RPN 0x0000 lane");
    assert_eq!(rpn0.events.len(), 1);

    let rpn1 = lead
        .automation_lanes
        .iter()
        .find(|l| l.target == AutomationTarget::Rpn { parameter: 0x0001 })
        .expect("RPN 0x0001 lane");
    assert_eq!(rpn1.events[0].value, 8192.0);

    let bass = m2.tracks.iter().find(|t| t.name == "Bass").expect("Bass");
    let bass_idx = m2.tracks.iter().position(|t| t.name == "Bass").unwrap() as u16;
    let bass_notes: Vec<&yinhe_core::Note> = m2.notes_for_track(bass_idx).collect();
    assert_eq!(bass_notes[0].start_tick, 0); // key is bucket index, not on Note
    assert_eq!(bass.channel, 1);
    assert_eq!(bass.port, 0);

    let drums = m2.tracks.iter().find(|t| t.name == "Drums").expect("Drums");
    let drums_idx = m2.tracks.iter().position(|t| t.name == "Drums").unwrap() as u16;
    assert_eq!(drums.port, 1);
    assert_eq!(drums.channel, 9);
    assert_eq!(m2.track_note_count[drums_idx as usize], 2);

    assert_eq!(m2.note_count, 7);
    assert_eq!(m2.tick_length, 1920);
    assert_eq!(m2.notes.len(), yinhe_types::KEY_COUNT);
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

/// 回归：桶内乱序时保存必须兜底排序（否则 delta 编码下溢 panic / 数据损坏）。
#[test]
fn unsorted_bucket_save_is_safe() {
    let mut m1 = build_complex_model();
    // 手动打乱 key=60 桶（模型不变量被破坏的极端情况）
    let bucket = Arc::make_mut(&mut m1.notes[60]);
    {
        let mut it = bucket.iter_mut();
        let a = it.next().expect("bucket 非空");
        let b = it.next().expect("bucket 非空");
        std::mem::swap(&mut a.start_tick, &mut b.start_tick); // 反序：破坏 start_tick 序
    }

    let bytes = save_yin_bytes(&m1).unwrap();
    let m2 = load_yin_bytes(&bytes).unwrap();

    // 保存时按 start_tick 排序，音符集合不丢；加载后桶内必须有序
    assert_eq!(m2.note_count, m1.note_count);
    assert!(m2.notes[60].is_sorted());
}

/// 回归：音轨 channel 乱序 + port 交错（真实 MIDI 常见：APT.mid 的
/// port=1 音轨插在 port=0 音轨中间）时，保存→加载必须保持音轨号、名字、
/// 通道、音符归属与 automation lane.track 完全不变。曾经 data.bin 的
/// meta payload 按 mapping.flat_tracks() 顺序写，而音符流 track 列仍为
/// model 索引，两空间不一致导致保存→加载后音轨名/通道/内容整体错位。
#[test]
fn track_order_preserved_when_ports_interleave() {
    // 故意交错：t0 port1、t1 port0、t2 port1、t3 port0（channel 也乱序）
    let mut t0 = TrackData::new(1, 0);
    t0.name = "Drums".to_string();
    let mut t1 = TrackData::new(0, 2);
    t1.name = "Solo".to_string();
    let mut t2 = TrackData::new(1, 1);
    t2.name = "Percussion".to_string();
    let mut t3 = TrackData::new(0, 0);
    t3.name = "Lead".to_string();

    t0.automation_lanes = vec![AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 0,
        events: vec![AutomationEvent {
            tick: 0,
            value: 100.0,
            shape: SegmentShape::Step,
        }],
    }];

    let per_track_notes = vec![
        vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 60,
            key: 36,
            velocity: 127,
        }],
        vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }],
        vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 120,
            key: 42,
            velocity: 120,
        }],
        vec![NoteEvent {
            id: 0,
            start_tick: 480,
            end_tick: 960,
            key: 62,
            velocity: 90,
        }],
    ];

    let mut model = YinModel {
        tracks: vec![Arc::new(t0), Arc::new(t1), Arc::new(t2), Arc::new(t3)],
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();

    let m2 = load_yin_bytes(&save_yin_bytes(&model).unwrap()).unwrap();

    // 音轨号（顺序）、名字、port、channel 全部不变
    let expected: [(&str, u8, u8); 4] = [
        ("Drums", 1, 0),
        ("Solo", 0, 2),
        ("Percussion", 1, 1),
        ("Lead", 0, 0),
    ];
    assert_eq!(m2.tracks.len(), expected.len());
    for (i, (name, port, ch)) in expected.iter().enumerate() {
        assert_eq!(m2.tracks[i].name, *name, "track {i} 名字");
        assert_eq!(m2.tracks[i].port, *port, "track {i} port");
        assert_eq!(m2.tracks[i].channel, *ch, "track {i} channel");
    }

    // 每轨音符归属不变（音符 track 索引 = 保存前 model 索引）
    let mut total: u64 = 0;
    for (i, (name, _, _)) in expected.iter().enumerate() {
        let count = m2.notes_for_track(i as u16).count();
        total += count as u64;
        assert!(count > 0, "track {i} ({name}) 必须有音符");
    }
    assert_eq!(total, m2.note_count);

    // automation lane.track 保持 model 索引，仍挂在原音轨上
    assert_eq!(m2.tracks[0].automation_lanes.len(), 1);
    assert_eq!(m2.tracks[0].automation_lanes[0].track, 0);
    assert!(m2.tracks[1..].iter().all(|t| t.automation_lanes.is_empty()));
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
    let t = TrackData::new(0, 0);
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        (0..100_000u32)
            .map(|i| NoteEvent {
                id: 0,
                start_tick: i * 10,
                end_tick: i * 10 + 5,
                key: 60,
                velocity: 100,
            })
            .collect(),
    ];
    let mut model = YinModel {
        tracks: vec![Arc::new(t)],
        ..Default::default()
    };
    model.meta.compression_level = 3;
    model.load_track_notes(per_track_notes);
    model.rebuild();

    let bytes = save_yin_bytes(&model).unwrap();
    // 100k * 16B = ~1.6 MB raw postcard（Note 含 id:u32 后）。
    // zstd 应至少压到 50% 以下：id 序列高度可压缩（差分=常量 1），
    // key/vel 都是常量，tick 单调递增。
    assert!(
        bytes.len() < 800_000,
        ".yin compression unexpectedly poor: {} bytes (raw ~1.6 MB)",
        bytes.len()
    );

    // Roundtrip preserves count.
    let m2 = load_yin_bytes(&bytes).unwrap();
    assert_eq!(m2.tracks.len(), 1);
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
    ProjectSoundFonts, SfEntryJson, SfPortOverride, load_yin_bytes_with_sf, save_yin_bytes_with_sf,
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
    let (m2, sf2, _mapping) = load_yin_bytes_with_sf(&bytes).unwrap();

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

// ──────────────────────── 旧存档颜色兼容 ────────────────────────

/// 旧版 .yin 的 mapping 里颜色是 RGB 三元素数组，
/// 反序列化时应自动补 alpha=1.0，避免旧存档无法打开。
#[test]
fn legacy_rgb_color_deserializes_with_alpha() {
    let json = r#"{"version":1,"ports":[{"port":0,"channels":[{"channel":0,"tracks":[{"uuid":"u1","name":"t","color":[0.5,0.25,0.125]}]}]}]}"#;
    let mf: yinhe_yin::MappingFile = serde_json::from_str(json).expect("parse mapping");
    let tm = &mf.ports[0].channels[0].tracks[0];
    assert_eq!(tm.color, [0.5, 0.25, 0.125, 1.0]);
}

/// 新格式 RGBA 四元素数组正常反序列化，alpha 原样保留。
#[test]
fn rgba_color_deserializes_as_is() {
    let json = r#"{"version":1,"ports":[{"port":0,"channels":[{"channel":0,"tracks":[{"uuid":"u1","name":"t","color":[1.0,0.0,0.0,0.5]}]}]}]}"#;
    let mf: yinhe_yin::MappingFile = serde_json::from_str(json).expect("parse mapping");
    let tm = &mf.ports[0].channels[0].tracks[0];
    assert_eq!(tm.color, [1.0, 0.0, 0.0, 0.5]);
}

#[test]
fn sf_save_without_state_loads_as_empty() {
    // Old-style save (no SF) round-trips through with-sf load as default state.
    let model = build_complex_model();
    let bytes = save_yin_bytes(&model).unwrap();
    let (_m2, sf2, _mapping) = load_yin_bytes_with_sf(&bytes).unwrap();
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
    let (_m2, sf2, _mapping) = load_yin_bytes_with_sf(&bytes).unwrap();
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
        mode: false,                            // global mode
        overrides: sample_sf_state().overrides, // but per-port list intact
    };
    let bytes = save_yin_bytes_with_sf(&model, &sf).unwrap();
    let (_m2, sf2, _mapping) = load_yin_bytes_with_sf(&bytes).unwrap();
    assert!(!sf2.mode);
    assert_eq!(sf2.overrides.len(), 2);
    assert_eq!(sf2.overrides[0].entries[0].path, "/sf2/piano.sf2");
}
// ---------------------------------------------------------------------------
//  混音段（可选第 4 段）roundtrip
// ---------------------------------------------------------------------------

#[test]
fn mixer_section_roundtrips() {
    use yinhe_mixer::{InsertRef, MixerParams, StripParams};

    let m = build_complex_model();
    let mut mixer = MixerParams::default();
    mixer.channels[3] = StripParams {
        gain: 0.7,
        pan: -0.5,
        mute: true,
        solo: false,
    };
    mixer.master.gain = 0.8;
    mixer.channel_inserts[3].push(InsertRef {
        plugin_path: "/Library/Audio/Plug-Ins/CLAP/Example.clap".into(),
        plugin_id: "com.example.effect".into(),
        name: "Example FX".into(),
        bypassed: true,
        state: Some(vec![1, 2, 3, 4, 5]),
    });
    mixer.master_inserts.push(InsertRef {
        plugin_path: "/x.clap".into(),
        plugin_id: "com.example.limiter".into(),
        name: "Limiter".into(),
        bypassed: false,
        state: None,
    });

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixer.yin");
    let project = yinhe_yin::ProjectFile::from_meta(&m.meta);
    let mapping = yinhe_yin::MappingFile::from_tracks(&m.tracks);
    yinhe_yin::save_yin_with_files(&m, &path, &project, &mapping, Some(&mixer)).unwrap();

    let (_m2, _sf, _mapping, mixer2) = yinhe_yin::load_yin_with_sf(&path).unwrap();
    let mixer2 = mixer2.expect("mixer section should round-trip");
    assert_eq!(mixer2.channels[3], mixer.channels[3]);
    assert_eq!(mixer2.master, mixer.master);
    assert_eq!(mixer2.channel_inserts[3], mixer.channel_inserts[3]);
    assert_eq!(mixer2.master_inserts, mixer.master_inserts);
}

#[test]
fn v5_file_without_mixer_section_loads_with_none() {
    // 无混音段的文件（旧保存路径）加载后 mixer 为 None，不报错。
    let m = build_complex_model();
    let bytes = save_yin_bytes(&m).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.yin");
    std::fs::write(&path, &bytes).unwrap();
    let (_m2, _sf, _mapping, mixer) = yinhe_yin::load_yin_with_sf(&path).unwrap();
    assert!(mixer.is_none());
}

#[test]
fn extended_keys_roundtrip() {
    // 256 键数据回归：边界键 0/127/128/200/255 的音符在保存→重开后逐条一致。
    let t0 = TrackData::new(0, 0);
    let mut t1 = TrackData::new(0, 1);
    t1.name = "Wide".to_string();
    let notes: Vec<NoteEvent> = [0u8, 127, 128, 200, 255]
        .into_iter()
        .map(|key| NoteEvent {
            id: 0,
            start_tick: key as u32 * 10,
            end_tick: key as u32 * 10 + 480,
            key,
            velocity: 100,
        })
        .collect();
    let mut model = YinModel {
        conductor: Arc::new(ConductorData::default()),
        tracks: vec![Arc::new(t0), Arc::new(t1)],
        meta: ProjectMeta::default(),
        ..Default::default()
    };
    model.load_track_notes(vec![vec![], notes]);
    model.rebuild();

    let bytes = save_yin_bytes(&model).unwrap();
    let m2 = load_yin_bytes(&bytes).unwrap();

    assert_eq!(m2.note_count, 5);
    for key in [0u8, 127, 128, 200, 255] {
        assert_eq!(
            m2.notes[key as usize].len(),
            1,
            "key {key} 的音符必须跨保存/重开保持"
        );
    }
}
