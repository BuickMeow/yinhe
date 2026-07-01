use super::*;
use std::collections::BTreeMap;
use xsynth_core::channel::ControlEvent;
use yinhe_core::{
    ConductorData, NoteEvent, PcEvent, ProjectMeta, TempoEvent, TrackData, YinModel,
};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget};

fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
    let conductor = ConductorData {
        tempo: vec![TempoEvent {
            tick: 0,
            bpm: 120.0,
        }],
        time_sig: Vec::new(),
    };
    let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
    let mut t = TrackData::new(0, first_ch);
    t.name = "Track 1".into();
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![notes
        .into_iter()
        .map(|(key, start, end, vel, _ch)| NoteEvent {
            start_tick: start,
            end_tick: end,
            key,
            velocity: vel,
            dup_index: 0,
        })
        .collect()];
    let meta = ProjectMeta {
        ppq: 480,
        ..ProjectMeta::default()
    };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

fn make_model_3_tracks() -> YinModel {
    let conductor = ConductorData {
        tempo: vec![TempoEvent {
            tick: 0,
            bpm: 120.0,
        }],
        time_sig: Vec::new(),
    };
    let mk = |ch: u8, _key: u8| {
        let t = TrackData::new(0, ch);
        Arc::new(t)
    };
    let meta = ProjectMeta {
        ppq: 480,
        ..ProjectMeta::default()
    };
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            dup_index: 0,
        }],
        vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 64,
            velocity: 100,
            dup_index: 0,
        }],
        vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 67,
            velocity: 100,
            dup_index: 0,
        }],
    ];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![mk(0, 60), mk(1, 64), mk(9, 67)],
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

#[test]
fn test_channels_for_model_basic() {
    let model = make_model_3_tracks();
    let (num_ch, mask) = crate::spawn::channels_for_model(&model);
    assert_eq!(num_ch, 10);
    assert!(mask[0]);
    assert!(mask[1]);
    assert!(mask[9]);
    assert!(!mask[2]);
}

#[test]
fn test_channels_for_model_multi_port() {
    let conductor = ConductorData {
        tempo: vec![TempoEvent {
            tick: 0,
            bpm: 120.0,
        }],
        time_sig: Vec::new(),
    };
    let t1 = TrackData::new(0, 0);
    let t2 = TrackData::new(1, 0);
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            dup_index: 0,
        }],
        vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            dup_index: 0,
        }],
    ];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t1), Arc::new(t2)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    let (num_ch, mask) = crate::spawn::channels_for_model(&model);
    assert_eq!(num_ch, 17);
    assert!(mask[0]);
    assert!(mask[16]);
    assert!(!mask[15]);
}

#[test]
fn test_channels_for_model_skips_velocity_0_1() {
    let model = make_model_with_notes(vec![
        (60, 0, 480, 0, 0),
        (61, 0, 480, 1, 0),
        (62, 0, 480, 2, 0),
    ]);
    let (_num_ch, mask) = crate::spawn::channels_for_model(&model);
    assert!(mask[0]);
}

#[test]
fn test_channels_for_model_cc_activates_channel() {
    let conductor = ConductorData::default();
    let mut t = TrackData::new(0, 5);
    t.automation_lanes = vec![AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 0,
        events: vec![AutomationEvent { tick: 0, value: 100 }],
    }];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();
    let (num_ch, mask) = crate::spawn::channels_for_model(&model);
    assert_eq!(num_ch, 6);
    assert!(mask[5]);
}

#[test]
fn test_channels_for_model_empty() {
    let model = YinModel::default();
    let (num_ch, mask) = crate::spawn::channels_for_model(&model);
    assert_eq!(num_ch, 1);
    assert!(mask.iter().all(|&b| !b));
}

#[test]
fn test_sorted_cc_ordering() {
    let mut cc = vec![
        SortedCC {
            sample: 100,
            channel: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
        },
        SortedCC {
            sample: 50,
            channel: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        SortedCC {
            sample: 200,
            channel: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 60)),
        },
    ];
    cc.sort_by_key(|e| e.sample);
    assert_eq!(cc[0].sample, 50);
    assert_eq!(cc[1].sample, 100);
    assert_eq!(cc[2].sample, 200);
}

#[test]
fn test_render_dispatches_note_inside_large_buffer_at_exact_sample() {
    let model = make_model_with_notes(vec![(60, 960, 1440, 100, 0)]);
    assert_eq!(model.notes[60].len(), 1);
    let model = Arc::new(model);
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(48000, 16, mask);
    engine.load_model(&model);
    engine.playing = true;

    // Note at key 60, start_tick=960, velocity=100 → should dispatch at sample 48000.
    assert_eq!(engine.next_event_sample(0, 60000), Some(48000));
    engine.dispatch_cc_until(48000);
    engine.dispatch_notes_at(48000);

    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
    assert_eq!(engine.sample_position(), 0);
}

#[test]
fn test_active_mask_length() {
    let mask = vec![false; 16];
    let _engine = AudioEngine::new(44100, 16, mask);
}

#[test]
fn test_audible_index_filters_vel_and_inactive_channel() {
    let conductor = ConductorData {
        tempo: vec![TempoEvent {
            tick: 0,
            bpm: 120.0,
        }],
        time_sig: Vec::new(),
    };
    let t0 = TrackData::new(0, 0);
    let t1 = TrackData::new(0, 3);
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        vec![
            NoteEvent {
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 0,
                dup_index: 0,
            },
            NoteEvent {
                start_tick: 480,
                end_tick: 960,
                key: 60,
                velocity: 1,
                dup_index: 0,
            },
            NoteEvent {
                start_tick: 960,
                end_tick: 1440,
                key: 60,
                velocity: 100,
                dup_index: 0,
            },
        ],
        vec![NoteEvent {
            start_tick: 1440,
            end_tick: 1920,
            key: 60,
            velocity: 100,
            dup_index: 0,
        }],
    ];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t0), Arc::new(t1)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    let model = Arc::new(model);

    let mut mask = vec![false; 16];
    mask[0] = true;
    let mut engine = AudioEngine::new(44100, 16, mask);
    engine.load_model(&model);

    assert_eq!(engine.note_cursor[60], 0);
    // Note at key 60, start_tick=960, velocity=100 → should dispatch at sample 44100.
    assert_eq!(engine.next_event_sample(0, 60000), Some(44100));
    engine.dispatch_cc_until(44100);
    engine.dispatch_notes_at(44100);
    // Cursor = 3: 2 low-vel skipped + 1 dispatched (4th note's start_sample > 44100).
    assert_eq!(engine.note_cursor[60], 3);
    assert_eq!(engine.active_notes.len(), 1);
    for key in 0..128usize {
        if key != 60 {
            assert_eq!(engine.note_cursor[key], 0);
        }
    }
}

#[test]
fn test_audible_index_empty_when_all_filtered() {
    let model = Arc::new(make_model_with_notes(vec![
        (60, 0, 480, 0, 0),
        (61, 0, 480, 1, 0),
    ]));
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);
    engine.load_model(&model);

    // All notes have velocity ≤ 1 → no events should dispatch.
    assert_eq!(engine.next_event_sample(0, 60000), None);
    for key in 0..128usize {
        assert_eq!(engine.note_cursor[key], 0);
    }
}

#[test]
fn test_audible_index_uses_per_key_tempo_cursor() {
    let conductor = ConductorData {
        tempo: vec![
            TempoEvent { tick: 0, bpm: 120.0 },
            TempoEvent { tick: 1000, bpm: 60.0 },
        ],
        time_sig: Vec::new(),
    };
    let t = TrackData::new(0, 0);
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![vec![
        NoteEvent {
            start_tick: 2000,
            end_tick: 2480,
            key: 0,
            velocity: 100,
            dup_index: 0,
        },
        NoteEvent {
            start_tick: 480,
            end_tick: 960,
            key: 60,
            velocity: 100,
            dup_index: 0,
        },
    ]];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();

    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(48000, 16, mask);
    engine.load_model(&Arc::new(model));

    // Note at key 0, start_tick=2000 → ~150000 samples at 48000 Hz (120→60 BPM at tick 1000).
    // Note at key 60, start_tick=480 → 24000 samples at 48000 Hz.
    assert_eq!(engine.next_event_sample(0, 200000), Some(24000));
    engine.dispatch_cc_until(24000);
    engine.dispatch_notes_at(24000);
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
    engine.dispatch_cc_until(150000);
    engine.dispatch_notes_at(150000);
    assert_eq!(engine.note_cursor[0], 1);
    // Note at key 60 ended at sample 48000, so only key 0 is active.
    assert_eq!(engine.active_notes.len(), 1);
}

#[test]
fn test_engine_accessors() {
    let mask = vec![true; 16];
    let engine = AudioEngine::new(44100, 16, mask);
    assert_eq!(engine.sample_rate_hz(), 44100);
    assert_eq!(engine.sample_position(), 0);
    assert!(!engine.playing());
}

#[test]
fn test_engine_handle_command_play_pause_stop() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);

    engine.handle_command(AudioCommand::Play { from_sample: 0 });
    assert!(engine.playing());
    assert_eq!(engine.sample_position(), 0);

    engine.handle_command(AudioCommand::Pause);
    assert!(!engine.playing());

    engine.handle_command(AudioCommand::Resume);
    assert!(engine.playing());

    engine.handle_command(AudioCommand::Stop);
    assert!(!engine.playing());
    assert_eq!(engine.sample_position(), 0);
}

#[test]
fn test_engine_handle_command_seek() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);
    engine.handle_command(AudioCommand::Seek { sample: 44100 });
    assert_eq!(engine.sample_position(), 44100);
}

#[test]
fn test_engine_handle_command_skip_tracks() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);
    let skip = vec![false, true, false];
    engine.handle_command(AudioCommand::SkipTracks { skip });
    assert_eq!(engine.skip_track, vec![false, true, false]);
}

#[test]
fn test_engine_render_not_playing() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);
    let mut output = vec![1.0f32; 100];
    engine.render(&mut output);
    assert!(output.iter().all(|&s| s == 0.0));
}

#[test]
fn test_engine_render_zero_frames() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);
    engine.handle_command(AudioCommand::Play { from_sample: 0 });
    let mut output: Vec<f32> = Vec::new();
    engine.render(&mut output);
}

fn make_model_with_controls(
    cc: Vec<(u8, u32, u8)>,
    pb: Vec<(u32, i16)>,
    pc: Vec<(u32, u8)>,
    rpn: Vec<(u16, u32, u16)>,
) -> YinModel {
    let conductor = ConductorData {
        tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
        time_sig: Vec::new(),
    };
    let mut t = TrackData::new(0, 0);

    // Build automation lanes from CC events
    let mut lanes: Vec<AutomationLane> = Vec::new();
    if !cc.is_empty() {
        let mut cc_by_controller: BTreeMap<u8, Vec<AutomationEvent>> = BTreeMap::new();
        for (controller, tick, value) in cc {
            cc_by_controller
                .entry(controller)
                .or_default()
                .push(AutomationEvent {
                    tick,
                    value: value as u16,
                });
        }
        for (controller, events) in cc_by_controller {
            lanes.push(AutomationLane {
                target: AutomationTarget::CC { controller },
                track: 0,
                events,
            });
        }
    }

    // Pitch bend lane
    if !pb.is_empty() {
        let events: Vec<AutomationEvent> = pb
            .into_iter()
            .map(|(tick, value)| AutomationEvent {
                tick,
                value: (value + 8192) as u16,
            })
            .collect();
        lanes.push(AutomationLane {
            target: AutomationTarget::PitchBend,
            track: 0,
            events,
        });
    }

    // RPN lanes
    for (key, tick, value) in rpn {
        lanes.push(AutomationLane {
            target: AutomationTarget::Rpn { parameter: key },
            track: 0,
            events: vec![AutomationEvent { tick, value }],
        });
    }

    t.automation_lanes = lanes;
    t.program_change = pc
        .into_iter()
        .map(|(tick, program)| PcEvent {
            tick,
            program,
            bank_msb: 0,
            bank_lsb: 0,
        })
        .collect();
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();
    model
}

#[test]
fn test_engine_load_model_and_reload() {
    let model = Arc::new(make_model_with_notes(vec![(60, 0, 480, 100, 0)]));
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);

    engine.handle_command(AudioCommand::LoadModel {
        model: model.clone(),
    });
    assert!(!engine.playing());

    engine.handle_command(AudioCommand::ReloadNotes { model });
}

/// Regression test: the MIMO refactor originally forgot to call
/// `load_model()` inside `ReloadNotes`, which meant CC / pitch-bend /
/// program-change / RPN events were never rebuilt after editing — they
/// stayed at whatever the *previous* model had.  This test loads model
/// A (rich controllers), reloads with model B (different controllers),
/// and asserts `cc_events` reflects model B.
#[test]
fn test_reload_notes_rebuilds_cc_pb_pc_rpn() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, 16, mask);

    let model_a = Arc::new(make_model_with_controls(
        vec![(7, 0, 100), (10, 0, 64)],
        vec![(0, 0)],
        vec![(0, 5)],
        vec![],
    ));
    engine.handle_command(AudioCommand::LoadModel { model: model_a });
    let cc_count_a = engine.cc_events.len();
    assert!(cc_count_a > 0, "model A should produce some events");

    // Model B: completely different shape — 3 CCs at different ticks,
    // 2 pitch bends, 2 program changes, 1 RPN (which expands to 3 raw CCs).
    let model_b = Arc::new(make_model_with_controls(
        vec![
            (7, 480, 80),
            (7, 960, 90),
            (11, 240, 100),
        ],
        vec![(120, 4096), (600, -2048)],
        vec![(0, 1), (480, 2)],
        vec![(0x0000, 240, 0x0200)],
    ));
    engine.handle_command(AudioCommand::ReloadNotes { model: model_b });

    // 3 CC + 2 PB + 2 PC (each with bank_msb=0 + bank_lsb=0 → 2 extra) + 3 RPN-expanded = 14
    assert_eq!(
        engine.cc_events.len(),
        14,
        "ReloadNotes must rebuild cc_events from the new model (was {} from model A)",
        cc_count_a
    );

    // Assert events are sorted (so the schedule loop's monotonic cursor works).
    for w in engine.cc_events.windows(2) {
        assert!(w[0].sample <= w[1].sample, "cc_events must be sorted by sample");
    }

    // Reload again with an empty model — cc_events must drain to zero.
    let model_c = Arc::new(make_model_with_controls(vec![], vec![], vec![], vec![]));
    engine.handle_command(AudioCommand::ReloadNotes { model: model_c });
    assert_eq!(
        engine.cc_events.len(),
        0,
        "ReloadNotes with empty model must clear cc_events"
    );
}

#[test]
fn test_engine_channel_map_inactive_channel() {
    let mut mask = vec![false; 16];
    mask[5] = true;
    let engine = AudioEngine::new(44100, 16, mask);
    assert_eq!(engine.channel_map[5], 0);
    assert_eq!(engine.channel_map[0], u32::MAX);
}

#[test]
fn test_engine_channel_map_multiple_active() {
    let mut mask = vec![false; 256];
    mask[0] = true;
    mask[2] = true;
    mask[10] = true;
    let engine = AudioEngine::new(44100, 256, mask);
    assert_eq!(engine.channel_map[0], 0);
    assert_eq!(engine.channel_map[1], u32::MAX);
    assert_eq!(engine.channel_map[2], 1);
    assert_eq!(engine.channel_map[10], 2);
}