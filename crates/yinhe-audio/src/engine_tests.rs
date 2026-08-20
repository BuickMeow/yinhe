use super::*;
use std::collections::BTreeMap;
use xsynth_core::channel::ControlEvent;
use xsynth_core::channel_group::ParallelismOptions;
use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, KEY_COUNT, SegmentShape};

use crate::channel_layout::ChannelLayout;

fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 120.0,
                shape: SegmentShape::Step,
            }],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let first_ch = notes.first().map(|n| n.4).unwrap_or(0);
    let mut t = TrackData::new(0, first_ch);
    t.name = "Track 1".into();
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        notes
            .into_iter()
            .map(|(key, start, end, vel, _ch)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: vel,
                id: 0,
            })
            .collect(),
    ];
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

#[test]
fn test_sorted_cc_ordering() {
    let mut cc = [
        SortedCC {
            tick: 100,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
        },
        SortedCC {
            tick: 50,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        SortedCC {
            tick: 200,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 60)),
        },
    ];
    cc.sort_by_key(|e| e.tick);
    assert_eq!(cc[0].tick, 50);
    assert_eq!(cc[1].tick, 100);
    assert_eq!(cc[2].tick, 200);
}

#[test]
fn test_render_dispatches_note_inside_large_buffer_at_exact_sample() {
    let model = make_model_with_notes(vec![(60, 960, 1440, 100, 0)]);
    assert_eq!(model.notes[60].len(), 1);
    let model = Arc::new(model);
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(mask));
    engine.load_model(&model);
    engine.playing = true;

    // Note at key 60, start_tick=960, velocity=100 → should dispatch at tick 960.
    // @48000Hz 1 tick = 50 sample：960 tick = 48000 sample，1200 tick = 60000 sample。
    let next = engine.dispatch_and_find_next(960, 1200);
    // NoteOff at tick 1440 > block_end 1200, so no next event in range.
    assert_eq!(next, None);

    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
    assert_eq!(engine.sample_position(), 0);
}

#[test]
fn test_active_mask_length() {
    let mask = vec![false; 16];
    let _engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
}

#[test]
fn test_audible_index_filters_vel_and_inactive_channel() {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 120.0,
                shape: SegmentShape::Step,
            }],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
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
                id: 0,
            },
            NoteEvent {
                start_tick: 480,
                end_tick: 960,
                key: 60,
                velocity: 1,
                id: 0,
            },
            NoteEvent {
                start_tick: 960,
                end_tick: 1440,
                key: 60,
                velocity: 100,
                id: 0,
            },
        ],
        vec![NoteEvent {
            start_tick: 1440,
            end_tick: 1920,
            key: 60,
            velocity: 100,
            id: 0,
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
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.load_model(&model);

    assert_eq!(engine.note_cursor[60], 0);
    // Note at key 60, start_tick=960, velocity=100 → should dispatch at tick 960
    //（44100 Hz：1 tick = 45.94 sample，960 tick = 44100 sample）。
    let next = engine.dispatch_and_find_next(960, 1306);
    // Next note (other track) starts at tick1440 = 66150 sample > block_end, so no next event.
    assert_eq!(next, None);
    // audible_notes 桶里只有 vel>1 的音符（哑音在 worker 线程已剔除）。
    // key 60 桶：1 个 vel=100 音符（start=44100），dispatch 后 cursor=1。
    assert_eq!(engine.note_cursor[60], 1);
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
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.load_model(&model);

    // All notes have velocity ≤ 1 → no events should dispatch.
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, None);
    // audible_notes 桶为空（哑音在 worker 线程已剔除），cursor 保持 0。
    assert_eq!(engine.note_cursor[60], 0);
    assert_eq!(engine.note_cursor[61], 0);
}

#[test]
fn test_audible_index_uses_per_key_tempo_cursor() {
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
                    tick: 1000,
                    value: 60.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let t = TrackData::new(0, 0);
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![vec![
        NoteEvent {
            start_tick: 2000,
            end_tick: 2480,
            key: 0,
            velocity: 100,
            id: 0,
        },
        NoteEvent {
            start_tick: 480,
            end_tick: 960,
            key: 60,
            velocity: 100,
            id: 0,
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
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(mask));
    engine.load_model(&Arc::new(model));

    // Note at key 0, start_tick=2000（60 BPM 段，1 tick = 100 sample @48000Hz）。
    // Note at key 60, start_tick=480 → 24000 samples（120 BPM 段，1 tick = 50 sample）。
    let next = engine.dispatch_and_find_next(480, 2500);
    // NoteOff at end_tick=960 是下一个事件（早于 key 0 的 NoteOn at 2000）。
    assert_eq!(next, Some(960));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(960, 2500);
    // 处理 NoteOff 960 后，下一个事件是 key 0 的 NoteOn at 2000。
    assert_eq!(next, Some(2000));
    // key 60 ended, so only key 0 is active.
    assert_eq!(engine.active_notes.len(), 0);

    let next = engine.dispatch_and_find_next(2000, 2500);
    // key 0 NoteOn 后，NoteOff at end_tick=2480。
    assert_eq!(next, Some(2480));
    assert_eq!(engine.note_cursor[0], 1);
    // key 0 is active.
    assert_eq!(engine.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(2480, 2500);
    // [2480, 2500) 内无更多事件。
    assert_eq!(next, None);
    assert_eq!(engine.active_notes.len(), 0);
}

#[test]
fn test_engine_accessors() {
    let mask = vec![true; 16];
    let engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    assert_eq!(engine.sample_rate, 44100);
    assert_eq!(engine.sample_position(), 0);
    assert!(!engine.playing());
}

#[test]
fn test_engine_handle_command_play_pause_stop() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));

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
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.handle_command(AudioCommand::Seek { sample: 44100 });
    assert_eq!(engine.sample_position(), 44100);
}

#[test]
fn test_engine_handle_command_skip_tracks() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    let skip = vec![false, true, false];
    engine.handle_command(AudioCommand::SkipTracks { skip });
    assert_eq!(engine.skip_track, vec![false, true, false]);
}

#[test]
fn test_engine_render_not_playing() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    let mut output = vec![1.0f32; 100];
    engine.render(&mut output);
    assert!(output.iter().all(|&s| s == 0.0));
}

#[test]
fn test_engine_render_zero_frames() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.handle_command(AudioCommand::Play { from_sample: 0 });
    let mut output: Vec<f32> = Vec::new();
    engine.render(&mut output);
}

fn make_model_with_controls(
    cc: Vec<(u8, u32, u8)>,
    pb: Vec<(u32, i16)>,
    pc: Vec<(u32, u8)>,
    rpn: Vec<(u16, u32, f32)>,
) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 120.0,
                shape: SegmentShape::Step,
            }],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
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
                    value: value as f32,
                    shape: SegmentShape::Step,
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
                value: (value + 8192) as f32,
                shape: SegmentShape::Step,
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
            events: vec![AutomationEvent {
                tick,
                value,
                shape: SegmentShape::Step,
            }],
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
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));

    engine.handle_command(AudioCommand::LoadModel {
        model: model.clone(),
    });
    assert!(!engine.playing());

    engine.handle_command(AudioCommand::ReloadNotes {
        model,
        am_ms: Arc::new(crate::spawn::AmMsMap::new()),
    });
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
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));

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
        vec![(7, 480, 80), (7, 960, 90), (11, 240, 100)],
        vec![(120, 4096), (600, -2048)],
        vec![(0, 1), (480, 2)],
        vec![(0x0000, 240, 0x0200 as f32)],
    ));
    engine.handle_command(AudioCommand::ReloadNotes {
        model: model_b,
        am_ms: Arc::new(crate::spawn::AmMsMap::new()),
    });

    // 3 CC + 2 PB + 2 PC (each with bank_msb=0 + bank_lsb=0 → 2 extra) + 1 RPN (high-level) = 12
    assert_eq!(
        engine.cc_events.len(),
        12,
        "ReloadNotes must rebuild cc_events from the new model (was {} from model A)",
        cc_count_a
    );

    // Assert events are sorted (so the schedule loop's monotonic cursor works).
    for w in engine.cc_events.windows(2) {
        assert!(w[0].tick <= w[1].tick, "cc_events must be sorted by tick");
    }

    // Reload again with an empty model — cc_events must drain to zero.
    let model_c = Arc::new(make_model_with_controls(vec![], vec![], vec![], vec![]));
    engine.handle_command(AudioCommand::ReloadNotes {
        model: model_c,
        am_ms: Arc::new(crate::spawn::AmMsMap::new()),
    });
    assert_eq!(
        engine.cc_events.len(),
        0,
        "ReloadNotes with empty model must clear cc_events"
    );
}

#[test]
fn test_engine_channel_layout_dense_for_smoke() {
    // 烟雾测试：通过 AudioEngine 访问 ChannelLayout 与直接构造结果一致。
    // ChannelLayout 的完整单元测试在 channel_layout.rs。
    let mut mask = vec![false; 16];
    mask[5] = true;
    let engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    assert_eq!(engine.channel_layout.dense_for(5), 0);
    assert_eq!(engine.channel_layout.dense_for(0), u32::MAX);
}

/// 创建一个包含多轨道、多音符的大型模型用于性能基准测试。
fn make_bench_model(tracks: usize, notes_per_track: usize) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 120.0,
                shape: SegmentShape::Step,
            }],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let meta = ProjectMeta {
        ppq: 480,
        ..ProjectMeta::default()
    };

    let mut per_track_notes: Vec<Vec<NoteEvent>> = Vec::with_capacity(tracks);
    let mut track_list = Vec::with_capacity(tracks);

    for t in 0..tracks {
        let ch = (t % 16) as u8;
        track_list.push(Arc::new(TrackData::new(0, ch)));
        let mut notes = Vec::with_capacity(notes_per_track);
        for n in 0..notes_per_track {
            let key = (n % 128) as u8;
            let start_tick = (n * 480) as u32;
            let end_tick = start_tick + 240;
            notes.push(NoteEvent {
                start_tick,
                end_tick,
                key,
                velocity: 100,
                id: 0,
            });
        }
        per_track_notes.push(notes);
    }

    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: track_list,
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

/// 基准测试：对比不同 xsynth 并行配置下的渲染性能。
///
/// 测试三种配置：
/// - `AUTO_PER_CHANNEL`（当前默认）：通道间并行，key 间串行
/// - `AUTO_PER_KEY`：通道间 + key 间都并行
/// - `Sequential`：全串行（baseline）
///
/// 输出渲染 1 秒音频所需的微秒数。
#[test]
fn bench_parallelism_configs() {
    const SAMPLE_RATE: u32 = 44100;
    const RENDER_SECONDS: u64 = 2;
    const RENDER_SAMPLES: usize = RENDER_SECONDS as usize * SAMPLE_RATE as usize * 2;
    const TRACKS: usize = 16;
    const NOTES_PER_TRACK: usize = 500;

    let model = Arc::new(make_bench_model(TRACKS, NOTES_PER_TRACK));
    let active_mask = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();

    let mut output = vec![0.0f32; RENDER_SAMPLES];

    struct Config {
        name: &'static str,
        parallelism: ParallelismOptions,
    }

    let configs = [
        Config {
            name: "AUTO_PER_CHANNEL",
            parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
        },
        Config {
            name: "AUTO_PER_KEY",
            parallelism: ParallelismOptions::AUTO_PER_KEY,
        },
        Config {
            name: "Sequential",
            parallelism: ParallelismOptions {
                channel: xsynth_core::channel_group::ThreadCount::None,
                key: xsynth_core::channel_group::ThreadCount::None,
            },
        },
    ];

    let mut results: Vec<(&str, u128)> = Vec::new();
    for cfg in &configs {
        // 预热：先跑一次不记录时间
        {
            let mut engine = AudioEngine::with_parallelism(
                SAMPLE_RATE,
                ChannelLayout::from_mask(active_mask.clone()),
                cfg.parallelism,
            );
            engine.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            engine.handle_command(AudioCommand::Play { from_sample: 0 });
            engine.render(&mut output);
        }

        // 正式测量
        let mut engine = AudioEngine::with_parallelism(
            SAMPLE_RATE,
            ChannelLayout::from_mask(active_mask.clone()),
            cfg.parallelism,
        );
        engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        engine.handle_command(AudioCommand::Play { from_sample: 0 });

        let start = std::time::Instant::now();
        engine.render(&mut output);
        let elapsed = start.elapsed().as_micros();

        results.push((cfg.name, elapsed));
        eprintln!(
            "  {:<20} → {:>8} µs ({}x real-time)",
            cfg.name,
            elapsed,
            (RENDER_SECONDS as u128 * 1_000_000) / elapsed.max(1)
        );
    }

    // 确保每个配置都跑了（不做具体数值断言，避免 CI 环境波动）
    assert!(
        results.iter().all(|(_, t)| *t > 0),
        "all configs returned 0 time"
    );
    eprintln!();
    eprintln!("Summary:");
    eprintln!("  AUTO_PER_CHANNEL 是当前默认配置，AUTO_PER_KEY 添加了 per-key 并行化开销。");
    eprintln!("  Sequential 是单线程 baseline，用于对比并行化收益。");
}

/// 真实 MIDI 性能测试：用 Night Voyager.mid 对比 AUTO_PER_CHANNEL vs AUTO_PER_KEY。
#[test]
#[ignore = "需要本地 MIDI 和 SoundFont 文件"]
fn prof_night_voyager_parallelism() {
    let midi_path = "/Users/jieneng/Music/MIDIs/Night Voyager.mid";
    let sf_path = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";

    use std::time::Instant;

    let model = std::sync::Arc::new(yinhe_midi::parse_path(midi_path).unwrap());
    let active_mask = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();

    let configs = [
        ("AUTO_PER_CHANNEL", ParallelismOptions::AUTO_PER_CHANNEL),
        ("AUTO_PER_KEY", ParallelismOptions::AUTO_PER_KEY),
    ];

    let render_secs = 30u64;
    let render_samples = render_secs * 44100 * 2;
    let chunk_frames = 512;
    let chunk_samples = chunk_frames * 2;

    for (name, parallelism) in &configs {
        let mut engine = AudioEngine::with_parallelism(
            44100,
            ChannelLayout::from_mask(active_mask.clone()),
            *parallelism,
        );

        engine.handle_command(AudioCommand::LoadModel {
            model: std::sync::Arc::clone(&model),
        });
        engine.handle_command(AudioCommand::LoadSoundFont {
            port: 0,
            paths: vec![sf_path.into()],
        });
        engine.handle_command(AudioCommand::Play { from_sample: 0 });

        let mut buf = vec![0.0f32; chunk_samples];
        let t0 = Instant::now();
        let mut rendered = 0u64;
        while rendered < render_samples {
            let frames = ((render_samples - rendered) as usize / 2).min(chunk_frames);
            let buf_slice = &mut buf[..frames * 2];
            engine.render(buf_slice);
            rendered += (frames * 2) as u64;
        }
        let elapsed = t0.elapsed();
        let elapsed_us = elapsed.as_micros() as u64;
        eprintln!(
            "  {:<20} → {:>8} µs ({}x real-time, max voice count: {})",
            name,
            elapsed_us,
            (render_secs * 1_000_000) / elapsed_us.max(1),
            engine.voice_count(),
        );
    }
}

/// 回归测试：通道激活完全由音轨决定（音轨存在即激活）。
///
/// 1. 真·空 model（无音轨）→ ChannelLayout 全 false → 无通道可 dispatch
/// 2. 有音轨（哪怕没有任何音符）→ 通道立即可用
/// 3. 引擎 spawn 后加第一个音符 → 无需重建即可发声（bug 修复核心）
#[test]
fn test_track_existence_activates_channel() {
    // 1. 无音轨 → 全 false
    let empty = YinModel::default();
    let layout_empty = crate::spawn::channels_for_model(&empty);
    assert!(!layout_empty.is_active(0));

    // 2. 有音轨（ch 0）但没有任何音符 → 通道 0 已激活
    let mut model = YinModel {
        tracks: vec![Arc::new(TrackData::new(0, 0))],
        ..Default::default()
    };
    let layout = crate::spawn::channels_for_model(&model);
    assert!(layout.is_active(0));
    assert_eq!(layout.dense_for(0), 0);
    assert_eq!(layout.compacted_channels(), 1);

    // 3. 加第一个音符（track 0 = ch 0）→ 同一引擎直接 dispatch
    let id = model.alloc_note_id();
    let bucket = Arc::make_mut(&mut model.notes[60]);
    bucket.insert_sorted(yinhe_types::Note {
        id,
        start_tick: 0,
        end_tick: 480,
        velocity: 100,
        track: 0,
    });
    model.rebuild();

    let mut engine = AudioEngine::new(44100, layout);
    engine.load_model(&Arc::new(model));
    engine.playing = true;

    // NoteOff at tick 480 = 1 beat @ 120 BPM @ 44100 Hz = 22050 samples.
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
}

// ---------------------------------------------------------------------------
// 集成测试：用 Document 模拟真实编辑流程
// ---------------------------------------------------------------------------

/// 用当前 model 的 ChannelLayout spawn 引擎，模拟 App 的 rebuild_audio_if_needed。
fn spawn_engine_for_doc(doc: &Document, sample_rate: u32) -> AudioEngine {
    let layout = crate::spawn::channels_for_model(&doc.data.model);
    let mut engine = AudioEngine::new(sample_rate, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::clone(&doc.data.model),
    });
    engine
}

/// 完整 bug 复现 + 修复验证：空 Document（16 条音轨占满 ch 0-15）→
/// 引擎 spawn 时通道已全部激活 → 写第一个音符立即发声，无需 teardown + 重建。
#[test]
fn test_first_note_on_fresh_document_dispatches_without_rebuild() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();

    // 1. 空 Document spawn 引擎：16 条音轨的通道 0-15 全部激活
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    assert!(engine.channel_layout.is_active(0));

    // 2. 加第一个音符（track 1 = channel 0）
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();

    // 3. 模拟 App 的 notify_notes_changed → UpdateNotes：音轨没变 → 激活状态
    //    没变 → 无需 teardown，旧引擎直接更新音符即可 dispatch
    engine.handle_command(AudioCommand::UpdateNotes {
        model: Arc::clone(&doc.data.model),
    });

    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
}

/// 增量 UpdateNotes 回归测试：
/// 1. 编辑只 bump 对应 key 桶的 note_revisions（worker dirty 计算的前提）
/// 2. `prepare_notes_dirty` 只重建 dirty 桶，其余桶 None
/// 3. `apply_notes_only` 增量应用：dirty 桶新音符可 dispatch，干净桶 cursor 保留
#[test]
fn test_notes_delta_incremental_apply_keeps_clean_bucket_cursor() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 64,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();

    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;

    // 播放推进一帧（512 帧 = 1024 samples）：两个音符 NoteOn，cursor 推进
    let mut out = vec![0.0f32; 1024];
    engine.render(&mut out);
    assert_eq!(engine.sample_position, 512);
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.note_cursor[64], 1);

    // 编辑：key 60 桶加一个更晚的音符 → 只有 note_revisions[60] bump
    let revs_before = doc.data.model.note_revisions;
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 960,
            end_tick: 1440,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();
    let revs_after = doc.data.model.note_revisions;
    assert_ne!(revs_before[60], revs_after[60], "dirty 桶 revision bump");
    assert_eq!(revs_before[64], revs_after[64], "干净桶 revision 不变");

    // worker 语义：dirty = revisions 对比 → 只有 key 60
    let dirty: [bool; KEY_COUNT] = core::array::from_fn(|k| revs_before[k] != revs_after[k]);
    assert!(dirty[60]);
    assert!(!dirty[64]);

    let (audio_model, yin_model, delta, _dur) =
        crate::prepare_model::prepare_notes_dirty(&doc.data.model, sample_rate, &dirty);
    assert_eq!(
        delta[60].as_ref().map(|b| b.len()),
        Some(2),
        "dirty 桶含新旧音符"
    );
    for key in 0..KEY_COUNT {
        if key != 60 {
            assert!(delta[key].is_none(), "非 dirty 桶不应重建");
        }
    }

    engine.apply_notes_only(audio_model, yin_model, delta, 0);

    // 干净桶（key 64）cursor 保留 = 1；dirty 桶（key 60）按 sample_position 重算 = 1
    assert_eq!(engine.note_cursor[64], 1, "干净桶 cursor 保留");
    assert_eq!(engine.note_cursor[60], 1, "dirty 桶 cursor 重算");

    // 继续 dispatch：480 = 两个 NoteOff，960 = 新音符（960 tick）的 NoteOn
    let next = engine.dispatch_and_find_next(480, 60000);
    assert_eq!(next, Some(960), "新音符 NoteOn 位置");
    assert_eq!(
        engine.active_notes.len(),
        0,
        "两个 NoteOff 已弹，新音符未到"
    );
    let next = engine.dispatch_and_find_next(960, 70000);
    assert_eq!(next, Some(1440), "新音符 NoteOff = 1440 tick");
    assert_eq!(engine.active_notes.len(), 1, "新音符已 NoteOn");
}
///
/// 空 Document 已用满 0-15 通道，所以先 remove_track(16) 释放 channel 15，
/// 再 add_track 让新 track 分配到 channel 15。
#[test]
fn test_add_track_then_rebuild_activates_new_channel() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();

    // 1. 释放 channel 15：移除 track 16（Track 16）
    doc.remove_track(16);

    // 2. track 1（通道 0）加一个音符
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();

    // 3. 初始 layout：通道 0-14 激活（Track 1..Track 15），channel 15 未激活
    let layout_before = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_before.is_active(0));
    assert!(!layout_before.is_active(15));
    assert_eq!(layout_before.compacted_channels(), 15);

    // 4. add_track(1)：新 track 在 idx 2，channel 15（第一个空闲）
    doc.add_track(1);
    doc.data.bump_revision();

    // 5. 新 layout：channel 15 已激活——即使新音轨还没有任何音符
    let layout_after = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_after.is_active(0), "channel 0 still active");
    assert!(layout_after.is_active(15), "channel 15 now active");
    assert_eq!(layout_after.compacted_channels(), 16);

    // 6. 重建引擎 → track 1 的音符能 dispatch
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
}

/// remove_track 后被移除音轨的通道失活，其音符不再 dispatch。
#[test]
fn test_remove_track_then_rebuild_deactivates_channel() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();

    // 1. track 1（通道 0）和 track 2（通道 1）各加一个音符
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.add_note(
        2,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 64,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();

    // 2. 初始 layout：通道 0-15 全部激活（Track 1..Track 16 占满）
    let layout_before = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_before.is_active(0));
    assert!(layout_before.is_active(1));
    assert_eq!(layout_before.compacted_channels(), 16);

    // 3. remove track 2（通道 1 的音符随之删除）
    doc.remove_track(2);
    doc.data.bump_revision();

    // 4. 新 layout：通道 1 失活，其余 15 个通道仍激活
    let layout_after = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_after.is_active(0));
    assert!(!layout_after.is_active(1));
    assert_eq!(layout_after.compacted_channels(), 15);

    // 5. 重建引擎 → 只有通道 0 的音符 dispatch
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.note_cursor[64], 0);
    assert_eq!(engine.active_notes.len(), 1);
}

/// mute 轨道的自动化事件（CC）在 dispatch 时应被跳过，
/// 不发送到合成器，使同 channel 上其他非 mute 轨道不受影响。
#[test]
fn test_muted_track_cc_skipped_in_dispatch() {
    use crate::audio_model::SortedCC;
    use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};

    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    // track 1 → channel 0
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();

    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;

    // 注入两条 CC 事件：track 0（mute）和 track 1（非 mute），同 channel 0
    engine.cc_events = Arc::new(vec![
        SortedCC {
            tick: 0,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 40)),
        },
        SortedCC {
            tick: 0,
            channel: 0,
            track: 1,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
    ]);
    engine.cc_cursor = 0;
    // mute track 0
    engine.skip_track = vec![true, false];

    // dispatch 应跳过 track 0 的 CC，只发 track 1 的
    engine.dispatch_and_find_next(0, 60000);
    // cc_cursor 推进到末尾（两条都处理了，但只发了一条）
    assert_eq!(engine.cc_cursor, 2);
}

/// 回归测试：mute 期间被 cc_cursor 越过但未 dispatch 的自动化事件，
/// chase_skip 不得标记——否则 unmute 后 chase 跳过这些控制器，
/// 该轨道的自动化状态永远丢失（卡在 mute 前的旧值），直到下次 seek。
#[test]
fn test_unmute_chase_skip_excludes_events_missed_while_muted() {
    use crate::audio_model::SortedCC;
    use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};

    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    doc.add_note(
        0,
        NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;

    // track 0 的两条 CC7：tick 100 = 40，tick 300 = 80
    engine.cc_events = Arc::new(vec![
        SortedCC {
            tick: 100,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 40)),
        },
        SortedCC {
            tick: 300,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
        },
    ]);

    // 从 0 播放，越过 tick 100：CC7=40 已 dispatch
    engine.seek_to(0);
    engine.dispatch_and_find_next(200, 100000);
    assert_eq!(engine.cc_cursor, 1, "tick 100 的事件应已 dispatch");

    // mute 轨道 0，继续播放越过 tick 300：CC7=80 被 cursor 越过但未 dispatch
    engine.skip_track = vec![true];
    engine.dispatch_and_find_next(400, 100000);
    assert_eq!(
        engine.cc_cursor, 2,
        "tick 300 的事件应被越过（未 dispatch）"
    );

    // unmute：chase 要恢复 CC7=80，chase_skip 不得标记 CC7
    engine.skip_track = vec![false];
    let skip = engine.chase_skip();
    assert_eq!(
        skip.cc_mask[0] & (1u128 << 7),
        0,
        "mute 期间越过但未 dispatch 的 CC7 被误标记，unmute 后 chase 会跳过它 → 自动化状态丢失"
    );
}

/// chase 计算时跳过 mute 轨道的 CC：
/// mute 轨道的 CC 不参与 channel state 快照构建。
#[test]
fn test_muted_track_cc_skipped_in_chase() {
    // track 0（将被 mute）的 CC7=40，track 1（非 mute）的 CC7=100，同 channel 0
    let mut t0 = TrackData::new(0, 0);
    t0.automation_lanes = vec![AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 0,
        events: vec![AutomationEvent {
            tick: 100,
            value: 40.0,
            shape: SegmentShape::Step,
        }],
    }];
    let mut t1 = TrackData::new(0, 0);
    t1.automation_lanes = vec![AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 1,
        events: vec![AutomationEvent {
            tick: 200,
            value: 100.0,
            shape: SegmentShape::Step,
        }],
    }];
    let mut model = YinModel {
        tracks: vec![Arc::new(t0), Arc::new(t1)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();

    // chase 到 tick 300，skip track 0
    let states = crate::spawn::compute_chase_states_for_test(&model, 300, &[true, false]);
    // track 0 的 CC7=40 被跳过，只有 track 1 的 CC7=100 生效
    // CC7 映射到 ChannelState.volume
    assert_eq!(
        states[0].volume, 100,
        "muted track's CC7=40 should be skipped; only track 1's CC7=100 should apply"
    );
}

/// 构造 cyber-night 风格的模型：RPN(0) PBS 2→48（tick 768）、PitchBend 滑音、CC7。
/// 第 2 小节 = tick 768（PPQ 480，4/4）。
fn make_chase_model() -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 120.0,
                shape: SegmentShape::Step,
            }],
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let mut t = TrackData::new(0, 0);
    t.name = "Chase".into();
    t.automation_lanes = vec![
        AutomationLane {
            target: AutomationTarget::Rpn { parameter: 0 },
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 0,
                    value: 2.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 768,
                    value: 48.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: AutomationTarget::PitchBend,
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 336,
                    value: 8192.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 1536,
                    value: 10892.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 192,
                    value: 100.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 768,
                    value: 80.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
    ];
    // 至少一个音符：track_audible_count > 0，否则 skip_track 会把该轨道当 mute。
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![vec![NoteEvent {
        start_tick: 0,
        end_tick: 100,
        key: 60,
        velocity: 100,
        id: 0,
    }]];
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

/// 回归测试（cyber-night 根因）：从第 2 小节（tick 768）开始播放时，
/// 渲染器先 dispatch 了 seek 点处的 PBS=48 / CC7=80，异步 chase 结果后到。
/// `apply_chase_result` 必须跳过已 dispatch 的控制器，否则 PBS 会被覆盖回
/// seek 前的 2（弯音幅度全错）、CC7 覆盖回 100。
#[test]
fn test_chase_after_seek_skips_dispatched_controllers() {
    let model = Arc::new(make_chase_model());
    let sr = 48000u32;
    let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);

    let seek_sample = (model.tempo_map.tick_to_seconds(768) * sr as f64) as u64;
    engine.seek_to(seek_sample);
    // seek 后 current_tick = sample_to_tick(seek_sample) = 768
    assert_eq!(engine.current_tick(), 768, "seek 位置应反查回 768 tick");

    // 渲染器第一帧：dispatch seek 点及之后的事件（含 t768 的 PBS=48、CC7=80）
    engine.dispatch_and_find_next(768, 768 + 512);

    // worker 异步算出的 chase 快照：seek 之前的状态
    // worker 异步算出的 chase 快照：seek 之前的状态（查询式：直接查模型 lane）
    let states = crate::spawn::compute_chase_states_for_test(&model, 768, &engine.skip_track);
    assert_eq!(states[0].pitch_bend_sensitivity, 2.0, "seek 前 PBS=2");
    assert_eq!(states[0].volume, 100, "seek 前 CC7=100");

    // 修复核心：已 dispatch 的控制器必须在 chase 中跳过
    let skip = engine.chase_skip();
    assert!(
        skip.pbs[0],
        "t768 的 PBS=48 已 dispatch，chase 必须跳过 PBS"
    );
    assert_ne!(
        skip.cc_mask[0] & (1u128 << 7),
        0,
        "t768 的 CC7=80 已 dispatch，chase 必须跳过 CC7"
    );
    assert!(
        !skip.pitch_bend[0],
        "t1536 的 PitchBend 尚未 dispatch，chase 应恢复 seek 前的 PB 值"
    );

    // 应用 chase 结果（跳过逻辑的行为由 events_to_send 单测覆盖）
    engine.apply_chase_result(&states);
    // seek 后继续渲染不应 panic，且后续事件照常 dispatch
    engine.dispatch_and_find_next(768 + 512, 768 + 2048);
}

#[test]
fn test_chase_channel_states_incremental() {
    use crate::preview_engine::chase_channel_states;

    let cc_events = vec![
        // ch0 的 CC7=100
        SortedCC {
            tick: 10,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        // ch1 的 CC7=50（不应影响 ch0）
        SortedCC {
            tick: 20,
            channel: 1,
            track: 1,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 50)),
        },
        // ch0 的 CC10=80（pan）
        SortedCC {
            tick: 30,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(10, 80)),
        },
        // ch0 的 CC7=90（最新）
        SortedCC {
            tick: 40,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 90)),
        },
        // ch0 的 PBS=48
        SortedCC {
            tick: 45,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(48.0)),
        },
        // 边界：tick == target 参与（预览无 dispatch 兜底，Bug 8 回归）
        SortedCC {
            tick: 50,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 10)),
        },
    ];
    // 同一 channel 三个升序目标：增量 chase 一次扫完
    let states = chase_channel_states(&cc_events, 0, &[25, 40, 50]);
    assert_eq!(states.len(), 3);
    assert_eq!(states[0].volume, 100, "target 25：只有 CC7=100");
    assert_eq!(states[0].pan, 64);
    assert_eq!(
        states[1].volume, 90,
        "target 40：tick==target 的 CC7=90 参与"
    );
    assert_eq!(states[1].pan, 80, "CC10 已累积");
    assert_eq!(
        states[2].volume, 10,
        "target 50：tick==target 的 CC7=10 参与，听到跳变后的值"
    );
    assert_eq!(states[2].pitch_bend_sensitivity, 48.0, "PBS 累积");

    // 其他 channel 不受影响
    let other = chase_channel_states(&cc_events, 1, &[50]);
    assert_eq!(other[0].volume, 50);
}

/// Bug 8 回归：预听恰好在自动化跳变点（tick 1920 从 0 跳到 127）的音符，
/// 必须听到跳变后的值 127，而不是跳变前的旧值。
#[test]
fn test_preview_chase_includes_jump_at_target_tick() {
    use crate::preview_engine::chase_channel_states;

    let jump = vec![
        SortedCC {
            tick: 1000,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 0)),
        },
        SortedCC {
            tick: 1920,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 127)),
        },
    ];
    let before = chase_channel_states(&jump, 0, &[1919]);
    let at = chase_channel_states(&jump, 0, &[1920]);
    assert_eq!(before[0].volume, 0, "跳变前：听到旧值 0");
    assert_eq!(at[0].volume, 127, "跳变点：必须听到跳变后的值 127");
}

// ---------------------------------------------------------------------------
// tick 域化回归测试：sample↔tick 转换、全曲渲染完整性、零长段、seek 去重
// ---------------------------------------------------------------------------

/// 构造带 tempo 变速的模型：`tempo_events` = [(tick, BPM), ...]，`notes` = [(key, start, end)]。
fn make_model_with_tempo(
    tempo_events: Vec<(u32, f32)>,
    notes: Vec<(u8, u32, u32)>,
) -> Arc<YinModel> {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: tempo_events
                .into_iter()
                .map(|(tick, value)| AutomationEvent {
                    tick,
                    value,
                    shape: SegmentShape::Step,
                })
                .collect(),
        },
        time_sig: Vec::new(),
        key_sig: Vec::new(),
        markers: Vec::new(),
        lyrics: Vec::new(),
        chord: Vec::new(),
    };
    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        notes
            .into_iter()
            .map(|(key, start, end)| NoteEvent {
                start_tick: start,
                end_tick: end,
                key,
                velocity: 100,
                id: 0,
            })
            .collect(),
    ];
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(TrackData::new(0, 0))],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    Arc::new(model)
}

/// sample↔tick 往返：变速（120→60 BPM @ tick 1000）下转换精确、浮点误差修正有效。
#[test]
fn test_sample_tick_roundtrip_with_tempo_changes() {
    let model = make_model_with_tempo(vec![(0, 120.0), (1000, 60.0)], vec![]);
    let sr = 44100f64;
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat;

    // 已知值：120BPM 段 1 tick = 45.9375 sample；60BPM 段 = 91.875
    assert_eq!(
        crate::audio_model::tick_to_sample(480, segments, tpb, sr),
        22050
    );
    assert_eq!(
        crate::audio_model::tick_to_sample(1000, segments, tpb, sr),
        45937
    );
    assert_eq!(
        crate::audio_model::tick_to_sample(2000, segments, tpb, sr),
        137812
    );

    // 往返：tick→sample→tick 精确还原（含变速段边界）
    for t in [0u32, 1, 479, 480, 999, 1000, 1001, 1999, 2000, 2500] {
        let s = crate::audio_model::tick_to_sample(t, segments, tpb, sr);
        assert_eq!(
            crate::audio_model::sample_to_tick(s, segments, tpb, sr),
            t,
            "tick {t} 往返失败"
        );
    }

    // 浮点边界：sample 略小于某 tick 的映射时，floor 反查应回到前一 tick
    assert_eq!(
        crate::audio_model::sample_to_tick(22049, segments, tpb, sr),
        479,
        "22049 sample 应反查 479 tick（22050 才是 480）"
    );
}

/// 全曲渲染完整性：所有音符恰好触发一次 NoteOn/NoteOff，无丢无重。
/// 这是 tick 域化后的事件闭环防护（dispatch 基准/块边界转换出错会在此暴露）。
#[test]
fn test_full_render_all_events_exactly_once() {
    let model = make_model_with_tempo(
        vec![(0, 120.0), (1000, 60.0)],
        vec![
            (60, 0, 480),
            (64, 480, 960),
            (67, 0, 2000),
            (72, 1500, 2400),
        ],
    );
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.load_model(&model);
    engine.playing = true;

    let total_frames = engine.duration_samples() as usize;
    assert!(total_frames > 0);
    let chunk = 1024usize;
    let mut out = vec![0.0f32; chunk * 2];
    let mut rendered = 0usize;
    let mut max_active = 0usize;
    // 多渲染一块：曲尾事件（end_tick 映射 sample == duration_samples）按
    // "候选 == 块边界延迟到下一块"的语义在曲内不触发（真实播放由 Stop 兜底），
    // 额外一块让它们最终触发，验证事件不丢。
    while rendered < total_frames + chunk {
        let n = (total_frames + chunk - rendered).min(chunk);
        engine.render(&mut out[..n * 2]);
        rendered += n;
        max_active = max_active.max(engine.active_notes.len());
    }

    // 全部事件已 dispatch：活跃音符清空、所有桶 cursor 到末尾
    assert_eq!(engine.active_notes.len(), 0, "所有音符都应 NoteOff");
    assert!(max_active >= 2, "渲染过程中应存在叠层音符");
    for key in 0..128usize {
        assert_eq!(
            engine.note_cursor[key],
            engine.audible_notes[key].len(),
            "key {key} 的桶应全部 dispatch"
        );
    }
    assert_eq!(
        engine.sample_position as usize,
        total_frames + chunk,
        "渲染到曲尾后一块"
    );
    assert_eq!(
        engine.current_tick(),
        crate::audio_model::sample_to_tick(
            (total_frames + chunk) as u64,
            &model.tempo_map.tempo_segments,
            model.tempo_map.ticks_per_beat,
            44100.0,
        ),
        "current_tick 应同步推进"
    );
}

/// 极快 tempo（1 tick < 1 sample）：多个 tick 映射同一 sample，零长渲染段
/// 不死循环、事件不丢（tick 域化后零长段路径的正确性防护）。
#[test]
fn test_fast_tempo_zero_length_segments_no_hang() {
    // 12000 BPM：mpq = 5000us，1 tick ≈ 0.46 sample @44100 → tick 480/481 同 sample。
    let model = make_model_with_tempo(vec![(0, 12000.0)], vec![(60, 0, 481), (64, 482, 960)]);
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.load_model(&model);
    engine.playing = true;

    // 一帧 512 帧：块内 tick 跨度约 1113，覆盖全部事件
    let mut out = vec![0.0f32; 1024];
    engine.render(&mut out);
    assert_eq!(engine.sample_position, 512, "整块渲染完成，无死循环");
    assert_eq!(engine.active_notes.len(), 0, "两个音符都 NoteOff");
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.note_cursor[64], 1);
}

/// seek 到任意位置后 dispatch：跨 seek 点的音符不重复 NoteOn
/// （seek 已重启一次，dispatch 不能再次触发）。
#[test]
fn test_seek_then_dispatch_no_duplicate_note_on() {
    let model = make_model_with_tempo(vec![(0, 120.0)], vec![(60, 0, 960), (64, 480, 1440)]);
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.load_model(&model);
    engine.playing = true;

    // seek 到 30000 sample ≈ tick 653（两个音符都已开始）
    let seek_sample = 30000u64;
    engine.seek_to(seek_sample);
    assert_eq!(engine.current_tick(), 653, "sample_to_tick(30000) = 653");
    assert_eq!(engine.active_notes.len(), 2, "seek 重启两个跨点音符");

    // dispatch seek 点：不重复 NoteOn（cursor 已跳过），只处理 NoteOff 边界
    let next = engine.dispatch_and_find_next(653, 3000);
    assert_eq!(next, Some(960), "下一个事件是两个音符的 NoteOff 960");
    assert_eq!(engine.active_notes.len(), 2, "dispatch 不重复 NoteOn");

    // NoteOff 960 触发一次，active 减 1
    engine.dispatch_and_find_next(960, 3000);
    assert_eq!(engine.active_notes.len(), 1);
    // 1440 处第二个 NoteOff
    engine.dispatch_and_find_next(1440, 3000);
    assert_eq!(engine.active_notes.len(), 0);
}

/// audible_notes 免 sort 依赖：模型桶乱序插入、rebuild 排序后，
/// 音频桶保持 start_tick 严格升序（dispatch 单调 cursor 的前提）。
#[test]
fn test_audible_buckets_sorted_without_sort() {
    let model = make_model_with_tempo(
        vec![(0, 120.0)],
        vec![(60, 960, 1440), (60, 0, 480), (60, 480, 960)], // 乱序插入
    );
    let audible = crate::prepare_model::build_audible_notes(&model);
    for key in 0..128usize {
        let bucket = &audible[key];
        for w in bucket.windows(2) {
            assert!(
                w[0].start_tick < w[1].start_tick,
                "key {key} 桶必须严格升序（免 sort 依赖模型桶顺序）"
            );
        }
    }
    assert_eq!(audible[60].len(), 3, "三个音符都进桶");
    assert_eq!(audible[60][0].start_tick, 0);
    assert_eq!(audible[60][2].start_tick, 960);
}

// ---------------------------------------------------------------------------
// 查询式 chase 回归测试：模型 lane 二分 + 曲线实时插值
// ---------------------------------------------------------------------------

/// 构造单 track 带一条 lane 的模型。
fn model_with_lane(lane: AutomationLane) -> YinModel {
    let mut t = TrackData::new(0, 0);
    t.automation_lanes = vec![lane];
    let mut model = YinModel {
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

/// 查询式 chase 的核心卖点：曲线段内实时插值真实值（与 flatten density 无关）。
/// 直线 = 退化曲线 `Curve { 0,0,0,0 }`。
#[test]
fn test_chase_query_linear_interpolation() {
    // CC7：tick 0 = 100 → tick 480 = 60，Linear（退化曲线）。
    let model = model_with_lane(AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 0,
        events: vec![
            AutomationEvent {
                tick: 0,
                value: 100.0,
                shape: SegmentShape::Curve {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                },
            },
            AutomationEvent {
                tick: 480,
                value: 60.0,
                shape: SegmentShape::Curve {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                },
            },
        ],
    });

    // 段中点：真实值 80（线性插值），CC7 → volume
    let states = crate::spawn::compute_chase_states_for_test(&model, 240, &[false]);
    assert_eq!(states[0].volume, 80, "曲线段中点应插值到 80");

    // 段外（最后一条之后）：保持终点值
    let states = crate::spawn::compute_chase_states_for_test(&model, 960, &[false]);
    assert_eq!(states[0].volume, 60, "曲线结束后保持终点值");

    // 边界：target == 下一事件 tick → 曲线终点值（连续性）
    let states = crate::spawn::compute_chase_states_for_test(&model, 480, &[false]);
    assert_eq!(states[0].volume, 60, "target == 段末 tick 取曲线终点");

    // target 在第一条事件之前：无事件，默认值 127
    let states = crate::spawn::compute_chase_states_for_test(&model, 0, &[false]);
    assert_eq!(states[0].volume, 127, "target 前无事件保持默认");
}

/// Step 段边界：保持最后一条事件值（与播放事件流 `tick < target` 语义一致）。
#[test]
fn test_chase_query_step_keeps_last_value() {
    // CC10 pan：tick 0 = 100（Step）→ tick 480 = 20（Step）。
    let model = model_with_lane(AutomationLane {
        target: AutomationTarget::CC { controller: 10 },
        track: 0,
        events: vec![
            AutomationEvent {
                tick: 0,
                value: 100.0,
                shape: SegmentShape::Step,
            },
            AutomationEvent {
                tick: 480,
                value: 20.0,
                shape: SegmentShape::Step,
            },
        ],
    });

    // 段内：保持 100
    let states = crate::spawn::compute_chase_states_for_test(&model, 240, &[false]);
    assert_eq!(states[0].pan, 100, "Step 段保持上一值");

    // 边界：target == 下一事件 tick，Step 语义保持 100（t480 的事件由 dispatch 处理）
    let states = crate::spawn::compute_chase_states_for_test(&model, 480, &[false]);
    assert_eq!(states[0].pan, 100, "Step 在事件 tick 处仍保持旧值");

    // 段后：20
    let states = crate::spawn::compute_chase_states_for_test(&model, 960, &[false]);
    assert_eq!(states[0].pan, 20, "Step 段后取新值");
}

/// Program Change：取 target 前最后一条（离散事件，无插值）。
#[test]
fn test_chase_query_program_change_last_before_target() {
    let mut t = TrackData::new(0, 0);
    t.program_change = vec![
        PcEvent {
            tick: 0,
            program: 5,
            bank_msb: 0,
            bank_lsb: 0,
        },
        PcEvent {
            tick: 480,
            program: 20,
            bank_msb: 0,
            bank_lsb: 0,
        },
    ];
    let mut model = YinModel {
        tracks: vec![Arc::new(t)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();

    let states = crate::spawn::compute_chase_states_for_test(&model, 240, &[false]);
    assert_eq!(states[0].program, 5, "target 前最后一条 PC");
    let states = crate::spawn::compute_chase_states_for_test(&model, 960, &[false]);
    assert_eq!(states[0].program, 20);
    // target == PC tick：该 PC 由 dispatch 处理，chase 取更早的
    let states = crate::spawn::compute_chase_states_for_test(&model, 480, &[false]);
    assert_eq!(states[0].program, 5, "t480 的 PC 不参与（== target）");
}

/// 查询式 vs flatten 全扫一致性：Step 段 + 各种控制器下，两种 chase 结果必须一致。
/// 这是 chase 语义没被改坏的防护（任何重写都必须过此测试）。
#[test]
fn test_chase_query_matches_flattened_scan() {
    // cyber-night 风格模型：RPN PBS、PitchBend、CC7、多 track 同 channel。
    let model = Arc::new(make_chase_model());
    let skip = vec![false; model.tracks.len()];

    for target in [200u32, 480, 768, 1000, 1536, 2000] {
        // 旧式：flatten 事件流从曲首累计（density=1 的离散近似）
        let cc = crate::audio_model::flatten_automation_to_cc_events(
            &model,
            1,
            &std::collections::HashMap::new(),
        );
        let mut old = [crate::channel::ChannelState::default(); 256];
        for e in cc.iter() {
            if e.tick >= target {
                break;
            }
            old[e.channel as usize].apply(&e.event);
        }
        // 新式：查询模型 lane
        let new = crate::spawn::compute_chase_states_for_test(&model, target, &skip);

        for ch in 0..256usize {
            assert_eq!(
                new[ch].volume, old[ch].volume,
                "target {target} ch{ch} volume"
            );
            assert_eq!(new[ch].pan, old[ch].pan, "target {target} ch{ch} pan");
            assert_eq!(
                new[ch].pitch_bend_sensitivity, old[ch].pitch_bend_sensitivity,
                "target {target} ch{ch} PBS"
            );
            assert_eq!(
                new[ch].pitch_bend, old[ch].pitch_bend,
                "target {target} ch{ch} pitch_bend"
            );
            assert_eq!(
                new[ch].program, old[ch].program,
                "target {target} ch{ch} program"
            );
            assert_eq!(new[ch].fine_tune, old[ch].fine_tune);
            assert_eq!(new[ch].coarse_tune, old[ch].coarse_tune);
            assert_eq!(
                new[ch].cc_values, old[ch].cc_values,
                "target {target} ch{ch} cc"
            );
        }
    }
}
