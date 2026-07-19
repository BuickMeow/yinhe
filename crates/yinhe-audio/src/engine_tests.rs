use super::*;
use std::collections::BTreeMap;
use xsynth_core::channel::ControlEvent;
use xsynth_core::channel_group::ParallelismOptions;
use yinhe_core::{
    ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel,
};
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

use crate::channel_layout::ChannelLayout;

fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
        },
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
            id: 0,
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
            events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
        },
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
                AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step },
                AutomationEvent { tick: 1000, value: 60.0, shape: SegmentShape::Step },
            ],
        },
        time_sig: Vec::new(),
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
            events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
        },
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
            events: vec![AutomationEvent { tick, value, shape: SegmentShape::Step }],
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
        vec![
            (7, 480, 80),
            (7, 960, 90),
            (11, 240, 100),
        ],
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
            events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
        },
        time_sig: Vec::new(),
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
            engine.handle_command(AudioCommand::LoadModel { model: Arc::clone(&model) });
            engine.handle_command(AudioCommand::Play { from_sample: 0 });
            engine.render(&mut output);
        }

        // 正式测量
        let mut engine = AudioEngine::with_parallelism(
            SAMPLE_RATE,
            ChannelLayout::from_mask(active_mask.clone()),
            cfg.parallelism,
        );
        engine.handle_command(AudioCommand::LoadModel { model: Arc::clone(&model) });
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
    assert!(results.iter().all(|(_, t)| *t > 0), "all configs returned 0 time");
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
    let sf_path =
        "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";

    use std::time::Instant;

    let model = std::sync::Arc::new(
        yinhe_mid2::parse_path(midi_path).unwrap(),
    );
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

/// 回归测试：空 model 创建引擎后，加音符应通过 teardown + 重建恢复发声。
///
/// 这是"新建工程写音符不发声"bug 的核心防护：
/// 1. 空 model → ChannelLayout 全 false → 引擎创建时无激活通道
/// 2. 后续 add_note 无法更新已冻结的 ChannelLayout
/// 3. 方案 A：add_track/remove_track 触发 teardown，下帧重建
/// 4. 重建时 from_model 重新扫描，新音符的通道被激活
#[test]
fn test_teardown_rebuild_reactivates_channels() {
    // 1. 空 model → 全 false
    let empty = YinModel::default();
    let layout_empty = crate::spawn::channels_for_model(&empty);
    assert!(!layout_empty.is_active(0));
    let engine = AudioEngine::new(44100, layout_empty);
    assert_eq!(engine.channel_layout.dense_for(0), u32::MAX);

    // 2. 加音符后重建 → 通道 0 激活
    let with_notes = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
    let layout_with = crate::spawn::channels_for_model(&with_notes);
    assert!(layout_with.is_active(0));
    assert_eq!(layout_with.dense_for(0), 0);
    assert_eq!(layout_with.compacted_channels(), 1);

    // 3. teardown 旧引擎，用新 layout 重建
    let model = Arc::new(with_notes);
    let mut engine = AudioEngine::new(44100, layout_with);
    engine.load_model(&model);
    engine.playing = true;

    // 4. dispatch 应能触发 NoteOn（之前在空 layout 下会被 dense_for=MAX 跳过）
    let next = engine.dispatch_and_find_next(0, 60000);
    // NoteOff at tick 480 = 1 beat @ 120 BPM @ 44100 Hz = 22050 samples.
    assert_eq!(next, Some(22050));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.active_notes.len(), 1);
}

// ---------------------------------------------------------------------------
// 方案 A 集成测试：用 Document 模拟真实编辑流程
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

/// 完整 bug 复现 + 修复验证：空 Document → 加音符 → 旧引擎无声 → teardown + 重建 → 有声。
#[test]
fn test_teardown_rebuild_fixes_silent_note_bug_via_document() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();

    // 1. 空 model → spawn 引擎 A（layout 全 false）
    let mut engine_a = spawn_engine_for_doc(&doc, sample_rate);
    engine_a.playing = true;

    // 2. 加音符（track 1 = channel 0）
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

    // 3. 旧引擎 A dispatch —— 通道 0 未激活 → NoteOn 被跳过（bug 复现）
    let next_a = engine_a.dispatch_and_find_next(0, 60000);
    assert_eq!(next_a, None, "旧引擎 layout 未激活通道 0 → 无声");
    assert_eq!(engine_a.active_notes.len(), 0);

    // 4. teardown 引擎 A，用新 model 重建引擎 B（方案 A）
    drop(engine_a);
    let mut engine_b = spawn_engine_for_doc(&doc, sample_rate);
    engine_b.playing = true;

    // 5. 新引擎 B 的 layout 已激活通道 0 → NoteOn 正常 dispatch
    let next_b = engine_b.dispatch_and_find_next(0, 60000);
    assert_eq!(next_b, Some(22050));
    assert_eq!(engine_b.note_cursor[60], 1);
    assert_eq!(engine_b.active_notes.len(), 1);
}

/// add_track 后新通道在重建的 layout 中被激活。
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

    // 3. 初始 layout：只有通道 0 激活
    let layout_before = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_before.is_active(0));
    assert!(!layout_before.is_active(15));
    assert_eq!(layout_before.compacted_channels(), 1);

    // 4. add_track(1)：新 track 在 idx 2，channel 15（第一个空闲）
    doc.add_track(1);
    doc.data.bump_revision();

    // 5. 在新 track（idx 2）上加音符 → channel 15
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

    // 6. 新 layout：通道 0 和 15 都激活
    let layout_after = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_after.is_active(0), "channel 0 still active");
    assert!(layout_after.is_active(15), "channel 15 now active");
    assert_eq!(layout_after.compacted_channels(), 2);

    // 7. 重建引擎 → 两个音符都能 dispatch
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(22050));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.note_cursor[64], 1);
    assert_eq!(engine.active_notes.len(), 2);
}

/// remove_track 后被移除通道的音符不再 dispatch。
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

    // 2. 初始 layout：通道 0 和 1 都激活
    let layout_before = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_before.is_active(0));
    assert!(layout_before.is_active(1));
    assert_eq!(layout_before.compacted_channels(), 2);

    // 3. remove track 2（通道 1 的音符随之删除）
    doc.remove_track(2);
    doc.data.bump_revision();

    // 4. 新 layout：只有通道 0 激活
    let layout_after = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_after.is_active(0));
    assert!(!layout_after.is_active(1));
    assert_eq!(layout_after.compacted_channels(), 1);

    // 5. 重建引擎 → 只有通道 0 的音符 dispatch
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(22050));
    assert_eq!(engine.note_cursor[60], 1);
    assert_eq!(engine.note_cursor[64], 0);
    assert_eq!(engine.active_notes.len(), 1);
}
