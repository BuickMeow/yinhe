use super::*;
use std::collections::BTreeMap;
use xsynth_core::channel::ControlEvent;
use xsynth_core::channel_group::ParallelismOptions;
use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

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
            sample: 100,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
        },
        SortedCC {
            sample: 50,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        SortedCC {
            sample: 200,
            channel: 0,
            track: 0,
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
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(mask));
    engine.load_model(&model);
    engine.playing = true;

    // Note at key 60, start_tick=960, velocity=100 → should dispatch at sample 48000.
    let next = engine.dispatch_and_find_next(48000, 60000);
    // NoteOff at tick1440 = 72000 samples > block_end 60000, so no next event in range.
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
    // Note at key 60, start_tick=960, velocity=100 → should dispatch at sample 44100.
    let next = engine.dispatch_and_find_next(44100, 60000);
    // Next note (other track) starts at tick1440 = 132300 > block_end, so no next event.
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

    // Note at key 0, start_tick=2000 → ~150000 samples at 48000 Hz (120→60 BPM at tick 1000).
    // Note at key 60, start_tick=480 → 24000 samples at 48000 Hz.
    let next = engine.dispatch_and_find_next(24000, 200000);
    // NoteOff at end_tick=960 = 48000 samples is the next event (before key 0 at 150000).
    assert_eq!(next, Some(48000));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(48000, 200000);
    // After dispatching NoteOff at 48000, next event is key 0 NoteOn at ~150000.
    assert_eq!(next, Some(150000));
    // key 60 ended, so only key 0 is active.
    assert_eq!(engine.active_notes.len(), 0);

    let next = engine.dispatch_and_find_next(150000, 200000);
    // After dispatching key 0 NoteOn, NoteOff at end_tick=2480 = 198000 samples.
    assert_eq!(next, Some(198000));
    assert_eq!(engine.note_cursor[0], 1);
    // key 0 is active.
    assert_eq!(engine.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(198000, 200000);
    // No more events in [198000, 200000).
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
    engine.handle_command(AudioCommand::ReloadNotes { model: model_b });

    // 3 CC + 2 PB + 2 PC (each with bank_msb=0 + bank_lsb=0 → 2 extra) + 1 RPN (high-level) = 12
    assert_eq!(
        engine.cc_events.len(),
        12,
        "ReloadNotes must rebuild cc_events from the new model (was {} from model A)",
        cc_count_a
    );

    // Assert events are sorted (so the schedule loop's monotonic cursor works).
    for w in engine.cc_events.windows(2) {
        assert!(
            w[0].sample <= w[1].sample,
            "cc_events must be sorted by sample"
        );
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

    let model = std::sync::Arc::new(yinhe_mid2::parse_path(midi_path).unwrap());
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
    bucket.push(yinhe_types::Note {
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
    assert_eq!(next, Some(22050));
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
    assert_eq!(next, Some(22050));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
}

/// add_track 后新音轨的通道在重建的 layout 中立即激活（无需等音符）。
///
/// 空 Document 已用满 0-15 通道，所以先 remove_track(16) 释放 channel 15，
/// 再 add_track 让新 track 分配到 channel 15。
#[test]
fn test_add_track_then_rebuild_activates_new_channel() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();

    // 1. 释放 channel 15：移除 track 16（A16）
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

    // 3. 初始 layout：通道 0-14 激活（A1..A15），channel 15 未激活
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
    assert_eq!(next, Some(22050));
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

    // 2. 初始 layout：通道 0-15 全部激活（A1..A16 占满）
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
    assert_eq!(next, Some(22050));
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
            sample: 0,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 40)),
        },
        SortedCC {
            sample: 0,
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

/// chase 计算时跳过 mute 轨道的 CC：
/// mute 轨道的 CC 不参与 channel state 快照构建。
#[test]
fn test_muted_track_cc_skipped_in_chase() {
    use crate::audio_model::SortedCC;
    use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};

    let cc_events = vec![
        // track 0（将被 mute）的 CC7=40
        SortedCC {
            sample: 100,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 40)),
        },
        // track 1（非 mute）的 CC7=100
        SortedCC {
            sample: 200,
            channel: 0,
            track: 1,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
    ];

    // chase 到 sample 300，skip track 0
    let states = crate::spawn::compute_chase_states_for_test(&cc_events, 300, &[true, false]);
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

    // 渲染器第一帧：dispatch seek 点及之后 512 帧的事件（含 t768 的 PBS=48、CC7=80）
    engine.dispatch_and_find_next(seek_sample, seek_sample + 512);

    // worker 异步算出的 chase 快照：seek 之前的状态
    let states = crate::spawn::compute_chase_states_for_test(
        &engine.cc_events,
        seek_sample,
        &engine.skip_track,
    );
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
    engine.apply_chase_result(states);
    // seek 后继续渲染不应 panic，且后续事件照常 dispatch
    engine.dispatch_and_find_next(seek_sample + 512, seek_sample + 2048);
}

#[test]
fn test_chase_channel_states_incremental() {
    use crate::preview_engine::chase_channel_states;

    let cc_events = vec![
        // ch0 的 CC7=100
        SortedCC {
            sample: 10,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        // ch1 的 CC7=50（不应影响 ch0）
        SortedCC {
            sample: 20,
            channel: 1,
            track: 1,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 50)),
        },
        // ch0 的 CC10=80（pan）
        SortedCC {
            sample: 30,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(10, 80)),
        },
        // ch0 的 CC7=90（最新）
        SortedCC {
            sample: 40,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 90)),
        },
        // ch0 的 PBS=48
        SortedCC {
            sample: 45,
            channel: 0,
            track: 0,
            event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(48.0)),
        },
        // 边界：sample == target 不参与
        SortedCC {
            sample: 50,
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
        states[1].volume, 100,
        "target 40：CC7=90 在 target 处不参与"
    );
    assert_eq!(states[1].pan, 80, "CC10 已累积");
    assert_eq!(
        states[2].volume, 90,
        "target 50：CC7=90 参与，边界事件不参与"
    );
    assert_eq!(states[2].pitch_bend_sensitivity, 48.0, "PBS 累积");

    // 其他 channel 不受影响
    let other = chase_channel_states(&cc_events, 1, &[50]);
    assert_eq!(other[0].volume, 50);
}
