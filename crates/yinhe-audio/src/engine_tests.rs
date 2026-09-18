use super::*;
use std::collections::BTreeMap;
use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};
use xsynth_core::channel_group::ParallelismOptions;
use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_editor_core::document::Document;
use yinhe_types::automation::{ParamDevice, xsynth_param};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, KEY_COUNT, SegmentShape};

use crate::channel_layout::ChannelLayout;

/// GPU 事件流（NoteOn 自带 `end_sample`）→ 展开出独立 NoteOff，供 xsynth 对比
/// 路径使用：xsynth 靠 NoteOff 事件释放，GpuSynth 靠 voice 到期自释，展开后
/// 两条路径的释放时机在同一 sample 对齐。
#[cfg(feature = "gpu")]
fn expand_note_offs(events: &[yinhe_synth::SynthEvent]) -> Vec<yinhe_synth::SynthEvent> {
    let mut out = Vec::with_capacity(events.len() * 2);
    for e in events {
        match e {
            yinhe_synth::SynthEvent::NoteOn {
                sample,
                channel,
                key,
                velocity,
                end_sample,
            } => {
                out.push(yinhe_synth::SynthEvent::NoteOn {
                    sample: *sample,
                    channel: *channel,
                    key: *key,
                    velocity: *velocity,
                    end_sample: *end_sample,
                });
                out.push(yinhe_synth::SynthEvent::NoteOff {
                    sample: *end_sample,
                    channel: *channel,
                    key: *key,
                });
            }
            other => out.push(*other),
        }
    }
    out.sort_by_key(|e| e.sample());
    out
}

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
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 80)),
        },
        SortedCC {
            tick: 50,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        SortedCC {
            tick: 200,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
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

/// track 0（MIDI 通道 0）的内置 XSynth 参数 target。
fn xsynth_target(id: u32) -> AutomationTarget {
    AutomationTarget::Param {
        device: ParamDevice::ChannelInstrument { channel: 0 },
        id,
        name: String::new(),
    }
}

/// CC → target：一律低层 CC（CC 广播方案）。
fn cc_target(controller: u8) -> AutomationTarget {
    AutomationTarget::CC { controller }
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

    // Build automation lanes from CC events（入参为原始整数 CC 值，内部归一化）。
    let mut lanes: Vec<AutomationLane> = Vec::new();
    if !cc.is_empty() {
        let mut cc_by_controller: BTreeMap<u8, Vec<AutomationEvent>> = BTreeMap::new();
        for (controller, tick, value) in cc {
            cc_by_controller
                .entry(controller)
                .or_default()
                .push(AutomationEvent {
                    tick,
                    value: value as f32 / 127.0,
                    shape: SegmentShape::Step,
                });
        }
        for (controller, events) in cc_by_controller {
            lanes.push(AutomationLane {
                target: cc_target(controller),
                track: 0,
                events,
            });
        }
    }

    // Pitch bend lane（入参为相对中心的原始偏移 -8192..8191，内部归一化）。
    if !pb.is_empty() {
        let events: Vec<AutomationEvent> = pb
            .into_iter()
            .map(|(tick, value)| AutomationEvent {
                tick,
                value: (value + 8192) as f32 / 16383.0,
                shape: SegmentShape::Step,
            })
            .collect();
        lanes.push(AutomationLane {
            target: xsynth_target(xsynth_param::PITCH_BEND),
            track: 0,
            events,
        });
    }

    // RPN lanes（入参为原始整数，按绑定上限归一化）。
    for (key, tick, value) in rpn {
        let (target, max) = match key {
            0 => (xsynth_target(xsynth_param::PB_SENSITIVITY), 127.0),
            1 => (xsynth_target(xsynth_param::FINE_TUNE), 16383.0),
            2 => (xsynth_target(xsynth_param::COARSE_TUNE), 127.0),
            _ => (AutomationTarget::Rpn { parameter: key }, 16383.0),
        };
        lanes.push(AutomationLane {
            target,
            track: 0,
            events: vec![AutomationEvent {
                tick,
                value: value / max,
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

    // ReloadNotes 在 renderer 层重建——引擎级等价路径直接 load_model。
    engine.load_model(&model);
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
    // ReloadNotes 在 renderer 层重建——引擎级等价路径直接 load_model。
    engine.load_model(&model_b);

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
    engine.load_model(&model_c);
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
        engine.handle_command(AudioCommand::SetSoundFonts {
            configs: Box::new(vec![(0u8, vec![sf_path.into()])]),
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
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 40)),
        },
        SortedCC {
            tick: 0,
            channel: 0,
            track: 1,
            lane: 0,
            plugin_param: None,
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

    // track 0 的两条 CC（用不同控制器以区分位图）：tick 100 = CC7 40，tick 300 = CC1 80。
    // CC1 用于验证"mute 期间越过未 dispatch"；CC7 的 dispatch 不受影响。
    engine.cc_events = Arc::new(vec![
        SortedCC {
            tick: 100,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 40)),
        },
        SortedCC {
            tick: 300,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(1, 80)),
        },
    ]);

    // 从 0 播放，越过 tick 100：CC7=40 已 dispatch
    engine.seek_to(0);
    engine.dispatch_and_find_next(200, 100000);
    assert_eq!(engine.cc_cursor, 1, "tick 100 的事件应已 dispatch");

    // mute 轨道 0，继续播放越过 tick 300：CC1=80 被 cursor 越过但未 dispatch
    engine.skip_track = vec![true];
    engine.dispatch_and_find_next(400, 100000);
    assert_eq!(
        engine.cc_cursor, 2,
        "tick 300 的事件应被越过（未 dispatch）"
    );

    // unmute：chase 要恢复 CC1=80，chase_skip 不得标记 CC1
    //（CC7=40 在 mute 前已 dispatch，bit7 置位是预期行为）
    engine.skip_track = vec![false];
    let skip = engine.chase_skip();
    assert_eq!(
        skip.cc_mask[0] & (1u128 << 1),
        0,
        "mute 期间越过但未 dispatch 的 CC1 被误标记，unmute 后 chase 会跳过它 → 自动化状态丢失"
    );
    assert_eq!(
        skip.cc_mask[0] & (1u128 << 7),
        1u128 << 7,
        "mute 前已 dispatch 的 CC7 应保持标记"
    );
}

/// 第 4 层回归：即时派语义——mute 立即停掉该轨在响音符，
/// unmute 立即重启该轨跨点音符（CPU/GPU 统一，不再等 gate 结束）。
#[test]
fn test_skip_mask_immediate_kill_and_restart() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    // track 1（track 0 是 conductor，add_note 会拒绝）：start 0，end 1000，播放到 500 时它在响
    doc.add_note(
        1,
        NoteEvent {
            start_tick: 0,
            end_tick: 1000,
            key: 60,
            velocity: 100,
            id: 0,
        },
    );
    doc.data.bump_revision();
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    // Document::empty() 的 track_audible_count 在 add_note 后未重建：显式标记全部可听。
    let all_audible = vec![false; engine.skip_track.len()];
    engine.skip_track = all_audible.clone();

    engine.seek_to(0);
    engine.dispatch_and_find_next(500, 100000);
    // 模拟播放推进：手动 dispatch 不会更新 current_tick（由 render 推进）。
    engine.current_tick = 500;
    assert_eq!(engine.active_notes.len(), 1, "播放中应有一个活跃音符");

    // mute track 1 → 立即从 active_notes 移除（发 NoteOff）
    let mut muted = all_audible.clone();
    muted[1] = true;
    engine.apply_skip_mask(&all_audible, &muted);
    assert_eq!(
        engine.active_notes.len(),
        0,
        "mute 应立即停掉该轨在响音符，而不是等 gate 结束"
    );

    // unmute track 1 → 立即重启跨点音符（回到 active_notes）
    engine.apply_skip_mask(&muted, &all_audible);
    assert_eq!(engine.active_notes.len(), 1, "unmute 应立即重启跨点音符");
}

/// chase 计算时跳过 mute 轨道的 CC：
/// mute 轨道的 CC 不参与 channel state 快照构建。
#[test]
fn test_muted_track_cc_skipped_in_chase() {
    // track 0（将被 mute）的 CC7=40，track 1（非 mute）的 CC7=100，同 channel 0
    let mut t0 = TrackData::new(0, 0);
    t0.automation_lanes = vec![AutomationLane {
        target: cc_target(7),
        track: 0,
        events: vec![AutomationEvent {
            tick: 100,
            value: 40.0 / 127.0,
            shape: SegmentShape::Step,
        }],
    }];
    let mut t1 = TrackData::new(0, 0);
    t1.automation_lanes = vec![AutomationLane {
        target: cc_target(7),
        track: 1,
        events: vec![AutomationEvent {
            tick: 200,
            value: 100.0 / 127.0,
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
        states[0].as_ref().unwrap().volume,
        100,
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
            target: xsynth_target(xsynth_param::PB_SENSITIVITY),
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 0,
                    value: 2.0 / 127.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 768,
                    value: 48.0 / 127.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: xsynth_target(xsynth_param::PITCH_BEND),
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 336,
                    value: 8192.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 1536,
                    value: 10892.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
            ],
        },
        AutomationLane {
            target: cc_target(7),
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 192,
                    value: 100.0 / 127.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 768,
                    value: 80.0 / 127.0,
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
    assert_eq!(
        states[0].as_ref().unwrap().pitch_bend_sensitivity,
        2.0,
        "seek 前 PBS=2"
    );
    assert_eq!(states[0].as_ref().unwrap().volume, 100, "seek 前 CC7=100");

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
    engine.apply_chase_result(&states, &[]);
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
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)),
        },
        // ch1 的 CC7=50（不应影响 ch0）
        SortedCC {
            tick: 20,
            channel: 1,
            track: 1,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 50)),
        },
        // ch0 的 CC10=80（pan）
        SortedCC {
            tick: 30,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(10, 80)),
        },
        // ch0 的 CC7=90（最新）
        SortedCC {
            tick: 40,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 90)),
        },
        // ch0 的 PBS=48（PB 参数，非 DSP CC，走常规 apply）
        SortedCC {
            tick: 45,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(48.0)),
        },
        // 边界：tick == target 参与（预览无 dispatch 兜底，Bug 8 回归）
        SortedCC {
            tick: 50,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
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
            lane: 0,
            plugin_param: None,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(7, 0)),
        },
        SortedCC {
            tick: 1920,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
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
        target: cc_target(7),
        track: 0,
        events: vec![
            AutomationEvent {
                tick: 0,
                value: 100.0 / 127.0,
                shape: SegmentShape::Curve {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                },
            },
            AutomationEvent {
                tick: 480,
                value: 60.0 / 127.0,
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
    assert_eq!(
        states[0].as_ref().unwrap().volume,
        80,
        "曲线段中点应插值到 80"
    );

    // 段外（最后一条之后）：保持终点值
    let states = crate::spawn::compute_chase_states_for_test(&model, 960, &[false]);
    assert_eq!(
        states[0].as_ref().unwrap().volume,
        60,
        "曲线结束后保持终点值"
    );

    // 边界：target == 下一事件 tick → 曲线终点值（连续性）
    let states = crate::spawn::compute_chase_states_for_test(&model, 480, &[false]);
    assert_eq!(
        states[0].as_ref().unwrap().volume,
        60,
        "target == 段末 tick 取曲线终点"
    );

    // target 在第一条事件之前：无事件 → None（chase 应用时不触碰通道）
    let states = crate::spawn::compute_chase_states_for_test(&model, 0, &[false]);
    assert!(
        states[0].is_none(),
        "target 前无事件应返回 None，应用 chase 时保持通道现状"
    );
}

/// Step 段边界：保持最后一条事件值（与播放事件流 `tick < target` 语义一致）。
#[test]
fn test_chase_query_step_keeps_last_value() {
    // CC10 pan：tick 0 = 100（Step）→ tick 480 = 20（Step）。
    let model = model_with_lane(AutomationLane {
        target: cc_target(10),
        track: 0,
        events: vec![
            AutomationEvent {
                tick: 0,
                value: 100.0 / 127.0,
                shape: SegmentShape::Step,
            },
            AutomationEvent {
                tick: 480,
                value: 20.0 / 127.0,
                shape: SegmentShape::Step,
            },
        ],
    });

    // 段内：保持 100
    let states = crate::spawn::compute_chase_states_for_test(&model, 240, &[false]);
    assert_eq!(states[0].as_ref().unwrap().pan, 100, "Step 段保持上一值");

    // 边界：target == 下一事件 tick，Step 语义保持 100（t480 的事件由 dispatch 处理）
    let states = crate::spawn::compute_chase_states_for_test(&model, 480, &[false]);
    assert_eq!(
        states[0].as_ref().unwrap().pan,
        100,
        "Step 在事件 tick 处仍保持旧值"
    );

    // 段后：20
    let states = crate::spawn::compute_chase_states_for_test(&model, 960, &[false]);
    assert_eq!(states[0].as_ref().unwrap().pan, 20, "Step 段后取新值");
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
    assert_eq!(
        states[0].as_ref().unwrap().program,
        5,
        "target 前最后一条 PC"
    );
    let states = crate::spawn::compute_chase_states_for_test(&model, 960, &[false]);
    assert_eq!(states[0].as_ref().unwrap().program, 20);
    // target == PC tick：该 PC 由 dispatch 处理，chase 取更早的
    let states = crate::spawn::compute_chase_states_for_test(&model, 480, &[false]);
    assert_eq!(
        states[0].as_ref().unwrap().program,
        5,
        "t480 的 PC 不参与（== target）"
    );
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
        let cc = crate::audio_model::flatten_automation_to_cc_events(&model, 1);
        let mut old = [crate::channel::ChannelState::default(); 256];
        for e in cc.iter() {
            if e.tick >= target {
                break;
            }
            old[e.channel as usize].apply(&e.event);
        }
        // 新式：查询模型 lane（无事件通道 = None，等价于 default 状态）
        let new = crate::spawn::compute_chase_states_for_test(&model, target, &skip);
        let default_state = crate::channel::ChannelState::default();

        for ch in 0..256usize {
            let ns = new[ch].as_ref().unwrap_or(&default_state);
            assert_eq!(ns.volume, old[ch].volume, "target {target} ch{ch} volume");
            assert_eq!(ns.pan, old[ch].pan, "target {target} ch{ch} pan");
            assert_eq!(
                ns.pitch_bend_sensitivity, old[ch].pitch_bend_sensitivity,
                "target {target} ch{ch} PBS"
            );
            assert_eq!(
                ns.pitch_bend, old[ch].pitch_bend,
                "target {target} ch{ch} pitch_bend"
            );
            assert_eq!(
                ns.program, old[ch].program,
                "target {target} ch{ch} program"
            );
            assert_eq!(ns.fine_tune, old[ch].fine_tune);
            assert_eq!(ns.coarse_tune, old[ch].coarse_tune);
            assert_eq!(ns.cc_values, old[ch].cc_values, "target {target} ch{ch} cc");
        }
    }
}

/// 诊断：cyber-night.mid 的 GPU 事件列表不变量（排序 / on-off 配对 / 同 sample 聚集）。
/// 复现"大量短音符（3 tick 中位数）换算到 sample 域后的配对与时序"。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI 文件"]
fn diag_cyber_night_event_invariants() {
    let path = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let model = std::sync::Arc::new(yinhe_midi::parse_path(path).unwrap());
    let active = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();
    let mut engine = AudioEngine::new(48_000, ChannelLayout::from_mask(active));
    engine.handle_command(AudioCommand::LoadModel { model });

    let events = engine.build_gpu_events(0);
    eprintln!("事件总数={}", events.len());

    let mut sorted = true;
    let mut prev = 0u64;
    for e in &events {
        if e.sample() < prev {
            sorted = false;
        }
        prev = e.sample();
    }
    eprintln!("采样域单调不减: {sorted}");

    // GPU note_off_to_cmd 语义：同 (channel,key) 释放最老的未释放 voice
    use std::collections::HashMap;
    let mut open: HashMap<(u8, u8), Vec<u64>> = HashMap::new();
    let mut on_count = 0u64;
    let mut unmatched_off = 0u64;
    for e in &events {
        match e {
            yinhe_synth::SynthEvent::NoteOn {
                sample,
                channel,
                key,
                ..
            } => {
                open.entry((*channel, *key)).or_default().push(*sample);
                on_count += 1;
            }
            yinhe_synth::SynthEvent::NoteOff {
                sample,
                channel,
                key,
            } => {
                let k = (*channel, *key);
                match open.get_mut(&k) {
                    Some(v) if !v.is_empty() => {
                        // 最老的 on 若晚于该 off 的 sample，说明 off 匹配到了"未来"的 voice
                        let oldest = v.remove(0);
                        if oldest > *sample {
                            eprintln!(
                                "  [配对] off 早于它匹配的 on：ch={channel} key={key} on={oldest} off={sample}"
                            );
                        }
                    }
                    _ => unmatched_off += 1,
                }
            }
            _ => {}
        }
    }
    let leftover: usize = open.values().map(|v| v.len()).sum();
    eprintln!("note_on={on_count} 无匹配 off={unmatched_off} 未关闭 on={leftover}");

    // 同 sample 事件聚集
    let mut max_same = 0usize;
    let mut cur_same = 0usize;
    let mut prev_s: Option<u64> = None;
    for e in &events {
        let s = e.sample();
        if prev_s == Some(s) {
            cur_same += 1;
        } else {
            cur_same = 1;
            prev_s = Some(s);
        }
        max_same = max_same.max(cur_same);
    }
    let last = events.last().map(|e| e.sample()).unwrap_or(0);
    eprintln!(
        "同 sample 最大事件数={max_same} 末事件={last}（≈{:.1}s）",
        last as f64 / 48_000.0
    );
}

/// 诊断：cyber-night 的 GPU vs xsynth 渲染对比（分通道隔离）。
/// 用户反馈：mute 掉 "NOTE 4"（MIDI ch3，16116 音符 / 50806 pitch bend）后正常。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn diag_cyber_night_channel_isolation() {
    use std::sync::Arc;
    use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

    let midi = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let secs = 6u64;
    let frames_per_chunk = 512usize;
    let total_frames = secs * sr as u64;

    let model = Arc::new(yinhe_midi::parse_path(midi).unwrap());
    let active = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();
    let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(active));
    engine.handle_command(AudioCommand::LoadModel { model });
    // cc_events 统计（原始）
    {
        let mut bytype: std::collections::BTreeMap<String, usize> = Default::default();
        let mut bych: std::collections::BTreeMap<u32, usize> = Default::default();
        for cc in engine.cc_events.iter() {
            let k = match &cc.event {
                ChannelAudioEvent::Control(ControlEvent::Raw(c, _)) => format!("CC{c}"),
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)) => "PB".into(),
                ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(_)) => "PBS".into(),
                ChannelAudioEvent::Control(ControlEvent::FineTune(_)) => "FineTune".into(),
                ChannelAudioEvent::Control(ControlEvent::CoarseTune(_)) => "CoarseTune".into(),
                ChannelAudioEvent::ProgramChange(_) => "PC".into(),
                _ => "other".into(),
            };
            *bytype.entry(k).or_insert(0) += 1;
            *bych.entry(cc.channel).or_insert(0) += 1;
        }
        eprintln!("cc_events 总数={} 类型={bytype:?}", engine.cc_events.len());
        eprintln!("cc_events 按 channel={bych:?}");
    }
    // 逐条过滤统计：找出 CC 被丢的具体条件
    {
        let mut stats = [0usize; 6];
        for cc in engine.cc_events.iter() {
            if engine
                .skip_track
                .get(cc.track as usize)
                .copied()
                .unwrap_or(false)
            {
                stats[0] += 1;
                continue;
            }
            let lane_skipped = engine
                .am_lane_skip
                .get(cc.track as usize)
                .and_then(|v| v.get(cc.lane as usize))
                .copied()
                .unwrap_or(false);
            if lane_skipped {
                stats[1] += 1;
                continue;
            }
            if cc.plugin_param.is_some() {
                stats[2] += 1;
                continue;
            }
            if engine.channel_plugin_dense(cc.channel as u8).is_some() {
                stats[3] += 1;
                continue;
            }
            let dense = engine.channel_layout.dense_for(cc.channel as usize);
            if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
                stats[4] += 1;
                continue;
            }
            if crate::engine_gpu::to_backend_control_event(&cc.event).is_none() {
                stats[5] += 1;
                continue;
            }
        }
        eprintln!(
            "过滤统计: track_skip={} lane_skip={} plugin_param={} plugin_ch={} dense={} to_gpu_none={}",
            stats[0], stats[1], stats[2], stats[3], stats[4], stats[5]
        );
    }
    let all_events = engine.build_gpu_events(0);
    {
        let mut bytype: std::collections::BTreeMap<&str, usize> = Default::default();
        for e in &all_events {
            let k = match e {
                yinhe_synth::SynthEvent::NoteOn { .. } => "NoteOn",
                yinhe_synth::SynthEvent::NoteOff { .. } => "NoteOff",
                yinhe_synth::SynthEvent::Control { .. } => "Control",
            };
            *bytype.entry(k).or_insert(0) += 1;
        }
        eprintln!("gpu 事件类型={bytype:?}");
    }

    let sfz_path = std::path::PathBuf::from(sfz);
    // 音色库只加载一次（进程级缓存共享）
    let sf = Arc::new(
        SampleSoundfont::new(
            sfz_path.clone(),
            AudioStreamParams {
                channels: ChannelCount::Stereo,
                sample_rate: sr,
            },
            SoundfontInitOptions::default(),
        )
        .expect("soundfont load"),
    );

    let render_gpu = |events: &[yinhe_synth::SynthEvent]| -> Vec<f32> {
        let mut used: std::collections::BTreeSet<u8> = Default::default();
        for e in events {
            match e {
                yinhe_synth::SynthEvent::NoteOn { channel, .. }
                | yinhe_synth::SynthEvent::NoteOff { channel, .. }
                | yinhe_synth::SynthEvent::Control { channel, .. } => {
                    used.insert(*channel);
                }
            }
        }
        let mut gpu = yinhe_synth::GpuSynth::new_default(sr).unwrap();
        for ch in &used {
            gpu.load_dense_soundfonts(*ch as u32, std::slice::from_ref(&sfz_path))
                .unwrap();
        }
        gpu.finish_soundfont_load();
        gpu.load_events(events.to_vec());
        gpu.seek(0);
        let used_max = (*used.iter().max().unwrap_or(&0) as usize) + 1;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..used_max)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames_per_chunk],
                right: vec![0.0; frames_per_chunk],
            })
            .collect();
        let mut out: Vec<f32> = Vec::with_capacity(total_frames as usize * 2);
        while (out.len() as u64) / 2 < total_frames {
            gpu.render_to_mixer(&mut bufs);
            for i in 0..frames_per_chunk {
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for b in &bufs {
                    l += b.left[i];
                    r += b.right[i];
                }
                out.push(l);
                out.push(r);
            }
        }
        out
    };

    let render_cpu = |events: &[yinhe_synth::SynthEvent]| -> Vec<f32> {
        let stream_params = AudioStreamParams {
            channels: ChannelCount::Stereo,
            sample_rate: sr,
        };
        let config = ChannelGroupConfig {
            channel_init_options: Default::default(),
            format: SynthFormat::Custom { channels: 32 },
            audio_params: stream_params,
            parallelism: ParallelismOptions {
                channel: xsynth_core::channel_group::ThreadCount::None,
                key: xsynth_core::channel_group::ThreadCount::None,
            },
        };
        let mut cg = ChannelGroup::new(config);
        let mut used: std::collections::BTreeSet<u8> = Default::default();
        for e in events {
            match e {
                yinhe_synth::SynthEvent::NoteOn { channel, .. }
                | yinhe_synth::SynthEvent::NoteOff { channel, .. }
                | yinhe_synth::SynthEvent::Control { channel, .. } => {
                    used.insert(*channel);
                }
            }
        }
        for ch in &used {
            cg.send_event(XEvent::Channel(
                *ch as u32,
                ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf.clone()])),
            ));
        }
        let expanded = expand_note_offs(events);
        let xevents: Vec<(u64, XEvent)> = expanded
            .iter()
            .map(|e| match e {
                yinhe_synth::SynthEvent::NoteOn {
                    sample,
                    channel,
                    key,
                    velocity,
                    ..
                } => (
                    *sample,
                    XEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: *key,
                            vel: *velocity,
                        }),
                    ),
                ),
                yinhe_synth::SynthEvent::NoteOff {
                    sample,
                    channel,
                    key,
                } => (
                    *sample,
                    XEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                    ),
                ),
                yinhe_synth::SynthEvent::Control {
                    sample,
                    channel,
                    event,
                } => {
                    let x = match event {
                        yinhe_synth::ControlEvent::ProgramChange(p) => {
                            ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(*p))
                        }
                        other => ChannelEvent::Audio(ChannelAudioEvent::Control(match other {
                            yinhe_synth::ControlEvent::Raw(c, v) => ControlEvent::Raw(*c, *v),
                            yinhe_synth::ControlEvent::PitchBend(v) => {
                                ControlEvent::PitchBendValue(*v)
                            }
                            yinhe_synth::ControlEvent::PitchBendSensitivity(v) => {
                                ControlEvent::PitchBendSensitivity(*v)
                            }
                            yinhe_synth::ControlEvent::FineTune(v) => ControlEvent::FineTune(*v),
                            yinhe_synth::ControlEvent::CoarseTune(v) => {
                                ControlEvent::CoarseTune(*v)
                            }
                            yinhe_synth::ControlEvent::PercussionMode(_) => ControlEvent::Raw(0, 0),
                            yinhe_synth::ControlEvent::ProgramChange(_) => unreachable!(),
                        })),
                    };
                    (*sample, XEvent::Channel(*channel as u32, x))
                }
            })
            .collect();

        let mut out: Vec<f32> = Vec::with_capacity(total_frames as usize * 2);
        let mut chunk = vec![0.0f32; frames_per_chunk * 2];
        let mut cursor = 0usize;
        let mut rendered = 0u64;
        let mut next_report = 0u64;
        while rendered < total_frames {
            while cursor < xevents.len() && xevents[cursor].0 <= rendered {
                cg.send_event(xevents[cursor].1.clone());
                cursor += 1;
            }
            if rendered >= next_report {
                eprintln!(
                    "  [cpu voice] t={:.2}s count={}",
                    rendered as f64 / sr as f64,
                    cg.voice_count()
                );
                next_report += sr as u64 / 4;
            }
            let seg_end = xevents
                .get(cursor)
                .map(|(s, _)| (*s).min(total_frames))
                .unwrap_or(total_frames);
            let seg_frames = seg_end - rendered;
            let mut done = 0u64;
            while done < seg_frames {
                let n = ((seg_frames - done) as usize).min(frames_per_chunk);
                let buf = &mut chunk[..n * 2];
                cg.read_samples(buf);
                out.extend_from_slice(buf);
                done += n as u64;
            }
            rendered = seg_end;
        }
        // 补偿 xsynth 通道默认 pan（中心 = 每声道 1/√2）
        for v in out.iter_mut() {
            *v *= std::f32::consts::SQRT_2;
        }
        out
    };

    let compare = |tag: &str, events: &[yinhe_synth::SynthEvent]| {
        let t = std::time::Instant::now();
        let gpu_out = render_gpu(events);
        let t_gpu = t.elapsed();
        let t = std::time::Instant::now();
        let cpu_out = render_cpu(events);
        let t_cpu = t.elapsed();
        let n = gpu_out.len().min(cpu_out.len());
        let mut sse = 0.0f64;
        let mut s_ref = 0.0f64;
        let mut max_diff = 0.0f32;
        for i in 0..n {
            let d = (gpu_out[i] - cpu_out[i]) as f64;
            sse += d * d;
            s_ref += (cpu_out[i] as f64) * (cpu_out[i] as f64);
            max_diff = max_diff.max((gpu_out[i] - cpu_out[i]).abs());
        }
        let rel = (sse / s_ref.max(1e-12)).sqrt();
        let mut first_big = None;
        for i in 0..n {
            if (gpu_out[i] - cpu_out[i]).abs() > 0.05 {
                first_big = Some(i);
                break;
            }
        }
        let t_big = first_big
            .map(|i| format!("{:.2}s", i as f64 / sr as f64 / 2.0))
            .unwrap_or_else(|| "-".into());
        eprintln!(
            "[{tag}] 事件={} gpu={t_gpu:?} cpu={t_cpu:?} rel_rmse={rel:.4} max_diff={max_diff:.4} 首次差>0.05@{t_big}",
            events.len()
        );
        rel
    };

    let no_ch3: Vec<yinhe_synth::SynthEvent> = all_events
        .iter()
        .filter(|e| match e {
            yinhe_synth::SynthEvent::NoteOn { channel, .. }
            | yinhe_synth::SynthEvent::NoteOff { channel, .. }
            | yinhe_synth::SynthEvent::Control { channel, .. } => *channel != 3,
        })
        .copied()
        .collect();
    let only_ch3: Vec<yinhe_synth::SynthEvent> = all_events
        .iter()
        .filter(|e| match e {
            yinhe_synth::SynthEvent::NoteOn { channel, .. }
            | yinhe_synth::SynthEvent::NoteOff { channel, .. }
            | yinhe_synth::SynthEvent::Control { channel, .. } => *channel == 3,
        })
        .copied()
        .collect();

    let ch3_no_pb: Vec<yinhe_synth::SynthEvent> = only_ch3
        .iter()
        .filter(|e| {
            !matches!(
                e,
                yinhe_synth::SynthEvent::Control {
                    event: yinhe_synth::ControlEvent::PitchBend(_),
                    ..
                }
            )
        })
        .copied()
        .collect();
    let ch3_no_cc64: Vec<yinhe_synth::SynthEvent> = only_ch3
        .iter()
        .filter(|e| {
            !matches!(
                e,
                yinhe_synth::SynthEvent::Control {
                    event: yinhe_synth::ControlEvent::Raw(64, _),
                    ..
                }
            )
        })
        .copied()
        .collect();
    let ch3_notes_only: Vec<yinhe_synth::SynthEvent> = only_ch3
        .iter()
        .filter(|e| !matches!(e, yinhe_synth::SynthEvent::Control { .. }))
        .copied()
        .collect();

    compare("全曲前6s", &all_events);
    compare("去掉 ch3", &no_ch3);
    compare("仅 ch3", &only_ch3);
    compare("ch3 无PB", &ch3_no_pb);
    compare("ch3 无CC64", &ch3_no_cc64);
    compare("ch3 仅音符", &ch3_notes_only);
}

/// 诊断：ch3 前 1 秒波形的 GPU vs xsynth 逐样本对比（定位首个差异点）。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn diag_cyber_night_ch3_wave_dump() {
    use std::sync::Arc;
    use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

    let midi = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 44_100u32;
    let secs = 3u64;
    let frames_per_chunk = 512usize;
    let total_frames = secs * sr as u64;

    let model = Arc::new(yinhe_midi::parse_path(midi).unwrap());
    let active = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();
    let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(active));
    engine.handle_command(AudioCommand::LoadModel { model });
    let all_events = engine.build_gpu_events(0);
    let events: Vec<yinhe_synth::SynthEvent> = all_events
        .iter()
        .filter(|e| {
            let ch = match e {
                yinhe_synth::SynthEvent::NoteOn { channel, .. }
                | yinhe_synth::SynthEvent::NoteOff { channel, .. }
                | yinhe_synth::SynthEvent::Control { channel, .. } => *channel,
            };
            ch == 3 && e.sample() < total_frames
        })
        .copied()
        .collect();
    // 全部音符（44.1k 重采样路径验证）
    let events_no_pb: Vec<yinhe_synth::SynthEvent> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                yinhe_synth::SynthEvent::NoteOn { .. } | yinhe_synth::SynthEvent::NoteOff { .. }
            )
        })
        .copied()
        .collect();
    eprintln!(
        "ch3 前1s 事件数={}（去PB={}）",
        events.len(),
        events_no_pb.len()
    );
    for e in events.iter() {
        if let yinhe_synth::SynthEvent::Control { sample, event, .. } = e {
            eprintln!("  CC @{:.4}s: {event:?}", *sample as f64 / sr as f64);
        }
    }

    // 完整事件聚合（时间 + on/off 数 + key 范围）
    {
        let mut agg: std::collections::BTreeMap<u64, (Vec<u8>, Vec<u8>)> = Default::default();
        for e in &events {
            match e {
                yinhe_synth::SynthEvent::NoteOn { sample, key, .. } => {
                    agg.entry(*sample).or_default().0.push(*key);
                }
                yinhe_synth::SynthEvent::NoteOff { sample, key, .. } => {
                    agg.entry(*sample).or_default().1.push(*key);
                }
                yinhe_synth::SynthEvent::Control { .. } => {}
            }
        }
        eprintln!("=== ch3 前3s 事件时间线 ===");
        for (sample, (ons, offs)) in &agg {
            eprintln!(
                "  t={:.4}s on={:?} off={:?}",
                *sample as f64 / sr as f64,
                if ons.is_empty() {
                    "-".to_string()
                } else {
                    format!(
                        "{}个[{}..{}]",
                        ons.len(),
                        ons.iter().min().unwrap(),
                        ons.iter().max().unwrap()
                    )
                },
                if offs.is_empty() {
                    "-".to_string()
                } else {
                    format!(
                        "{}个[{}..{}]",
                        offs.len(),
                        offs.iter().min().unwrap(),
                        offs.iter().max().unwrap()
                    )
                },
            );
        }
    }

    // GPU（与 CPU 侧同一输入 events_no_pb）
    let mut gpu = yinhe_synth::GpuSynth::new_default(sr).unwrap();
    gpu.load_dense_soundfonts(3, &[std::path::PathBuf::from(sfz)])
        .unwrap();
    gpu.finish_soundfont_load();
    gpu.load_events(events_no_pb.clone());
    gpu.seek(0);
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..4)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames_per_chunk],
            right: vec![0.0; frames_per_chunk],
        })
        .collect();
    let mut gpu_out: Vec<f32> = Vec::new();
    let mut next_report = 0u64;
    while (gpu_out.len() as u64) / 2 < total_frames {
        gpu.render_to_mixer(&mut bufs);
        let pos = (gpu_out.len() as u64) / 2;
        if pos >= next_report {
            eprintln!(
                "  [voice] t={:.2}s count={}",
                pos as f64 / sr as f64,
                gpu.voice_count()
            );
            next_report += sr as u64 / 4;
        }
        for i in 0..frames_per_chunk {
            gpu_out.push(bufs[3].left[i]);
            gpu_out.push(bufs[3].right[i]);
        }
    }
    eprintln!("  [voice] peak={}", gpu.peak_voices());

    // CPU
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: sr,
    };
    let sf = Arc::new(
        SampleSoundfont::new(
            std::path::PathBuf::from(sfz),
            stream_params,
            SoundfontInitOptions::default(),
        )
        .expect("soundfont load"),
    );
    let config = ChannelGroupConfig {
        channel_init_options: Default::default(),
        format: SynthFormat::Custom { channels: 32 },
        audio_params: stream_params,
        parallelism: ParallelismOptions {
            channel: xsynth_core::channel_group::ThreadCount::None,
            key: xsynth_core::channel_group::ThreadCount::None,
        },
    };
    let mut cg = ChannelGroup::new(config);
    cg.send_event(XEvent::Channel(
        3,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf])),
    ));
    let xevents: Vec<(u64, XEvent)> = events_no_pb
        .iter()
        .map(|e| match e {
            yinhe_synth::SynthEvent::NoteOn {
                sample,
                key,
                velocity,
                ..
            } => (
                *sample,
                XEvent::Channel(
                    3,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                        key: *key,
                        vel: *velocity,
                    }),
                ),
            ),
            yinhe_synth::SynthEvent::NoteOff { sample, key, .. } => (
                *sample,
                XEvent::Channel(
                    3,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                ),
            ),
            yinhe_synth::SynthEvent::Control { sample, event, .. } => (
                *sample,
                XEvent::Channel(
                    3,
                    ChannelEvent::Audio(ChannelAudioEvent::Control(match event {
                        yinhe_synth::ControlEvent::Raw(c, v) => ControlEvent::Raw(*c, *v),
                        yinhe_synth::ControlEvent::PitchBend(v) => ControlEvent::PitchBendValue(*v),
                        _ => ControlEvent::Raw(0, 0),
                    })),
                ),
            ),
        })
        .collect();
    let mut cpu_out: Vec<f32> = Vec::new();
    let mut chunk = vec![0.0f32; frames_per_chunk * 2];
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < total_frames {
        while cursor < xevents.len() && xevents[cursor].0 <= rendered {
            cg.send_event(xevents[cursor].1.clone());
            cursor += 1;
        }
        let seg_end = xevents
            .get(cursor)
            .map(|(s, _)| (*s).min(total_frames))
            .unwrap_or(total_frames);
        let seg_frames = seg_end - rendered;
        let mut done = 0u64;
        while done < seg_frames {
            let n = ((seg_frames - done) as usize).min(frames_per_chunk);
            let buf = &mut chunk[..n * 2];
            cg.read_samples(buf);
            cpu_out.extend_from_slice(buf);
            done += n as u64;
        }
        rendered = seg_end;
    }
    // 补偿 xsynth 通道默认 pan
    for v in cpu_out.iter_mut() {
        *v *= std::f32::consts::SQRT_2;
    }

    // 每 100ms RMS + 最大差异位置
    let n = gpu_out.len().min(cpu_out.len());
    let win = (sr / 10) as usize * 2;
    let mut k = 0usize;
    while k + win <= n {
        let g: f64 = gpu_out[k..k + win]
            .iter()
            .map(|&x| (x as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        let c: f64 = cpu_out[k..k + win]
            .iter()
            .map(|&x| (x as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        eprintln!(
            "t={:.1}s gpu_rms={:.3} cpu_rms={:.3} ratio={:.3}",
            k as f64 / 2.0 / sr as f64,
            g / (win as f64).sqrt(),
            c / (win as f64).sqrt(),
            g / c.max(1e-12)
        );
        k += win;
    }
    let mut worst = (0usize, 0.0f32);
    for i in 0..n {
        let d = (gpu_out[i] - cpu_out[i]).abs();
        if d > worst.1 {
            worst = (i, d);
        }
    }
    eprintln!(
        "max_diff={:.4} @样本 {}（{:.3}s）",
        worst.1,
        worst.0,
        worst.0 as f64 / 2.0 / sr as f64
    );
    // 打印 max diff 时刻附近 ±0.1s 的事件
    let t0s = worst.0 as u64 / 2;
    let lo = t0s.saturating_sub((sr / 10) as u64);
    let hi = t0s + (sr / 10) as u64;
    eprintln!(
        "=== 事件区间 [{:.3}s, {:.3}s] ===",
        lo as f64 / sr as f64,
        hi as f64 / sr as f64
    );
    for e in events
        .iter()
        .filter(|e| e.sample() >= lo && e.sample() <= hi)
    {
        eprintln!("  {e:?}");
    }
    let a = worst.0.saturating_sub(8);
    let b = (worst.0 + 8).min(n);
    eprintln!(
        "  GPU: {:?}",
        gpu_out[a..b]
            .iter()
            .map(|x| format!("{x:.4}"))
            .collect::<Vec<_>>()
    );
    eprintln!(
        "  CPU: {:?}",
        cpu_out[a..b]
            .iter()
            .map(|x| format!("{x:.4}"))
            .collect::<Vec<_>>()
    );
    let peak = gpu_out.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let cpeak = cpu_out.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    eprintln!("gpu_peak={peak:.4} cpu_peak={cpeak:.4}");
    // 导出 wav 供本地分析
    for (name, data) in [("gpu", &gpu_out), ("cpu", &cpu_out)] {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let path = format!("/tmp/diag_ch3_{name}.wav");
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for &v in data.iter() {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        eprintln!("已导出 {path}");
    }
    // 差值每 5ms RMS
    let dw = (sr / 200) as usize * 2;
    let mut k = 0usize;
    while k + dw <= n {
        let d: f64 = (0..dw)
            .map(|i| ((gpu_out[k + i] - cpu_out[k + i]) as f64).powi(2))
            .sum();
        let r = (d / dw as f64).sqrt();
        if r > 0.02 {
            eprintln!("diff t={:.3}s rms={:.4}", k as f64 / 2.0 / sr as f64, r);
        }
        k += dw;
    }
}

/// 诊断：隔离渲染 ch3 高音刮奏（key 107-127 同时按/放），对比 GPU 与 xsynth。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要 SoundFont"]
fn diag_high_cluster_isolated() {
    use std::sync::Arc;
    use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let frames_per_chunk = 512usize;
    let _total_frames = 0; // 在循环内按批数设置

    for (tag, batches, keys, interval, with_off) in [
        ("8批", 8usize, 1u8, 5_294u64, false),
        ("12批", 12, 1, 5_294, false),
        ("16批", 16, 1, 5_294, false),
        ("20批", 20, 1, 5_294, false),
    ] {
        let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();
        for b in 0..batches {
            let t0 = b as u64 * interval;
            for key in 107u8..107 + keys {
                events.push(yinhe_synth::SynthEvent::NoteOn {
                    sample: t0,
                    channel: 3,
                    key,
                    velocity: 127,
                    end_sample: if with_off { t0 + 4_963 } else { u64::MAX },
                });
                if with_off {
                    events.push(yinhe_synth::SynthEvent::NoteOff {
                        sample: t0 + 4_963,
                        channel: 3,
                        key,
                    });
                }
            }
        }
        events.sort_by_key(|e| e.sample());

        let total_frames = batches as u64 * interval + 5 * sr as u64;
        // GPU
        let mut gpu = yinhe_synth::GpuSynth::new_default(sr).unwrap();
        gpu.load_dense_soundfonts(3, &[std::path::PathBuf::from(sfz)])
            .unwrap();
        gpu.finish_soundfont_load();
        gpu.load_events(events.clone());
        gpu.seek(0);
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..4)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames_per_chunk],
                right: vec![0.0; frames_per_chunk],
            })
            .collect();
        let mut gpu_out: Vec<f32> = Vec::new();
        while (gpu_out.len() as u64) / 2 < total_frames {
            gpu.render_to_mixer(&mut bufs);
            for i in 0..frames_per_chunk {
                gpu_out.push(bufs[3].left[i]);
                gpu_out.push(bufs[3].right[i]);
            }
        }
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        {
            let mut w = hound::WavWriter::create(format!("/tmp/mb_{tag}_gpu.wav"), spec).unwrap();
            for &v in gpu_out.iter() {
                w.write_sample(v).unwrap();
            }
            w.finalize().unwrap();
        }

        // CPU
        let stream_params = AudioStreamParams {
            channels: ChannelCount::Stereo,
            sample_rate: sr,
        };
        let sf = Arc::new(
            SampleSoundfont::new(
                std::path::PathBuf::from(sfz),
                stream_params,
                SoundfontInitOptions::default(),
            )
            .expect("soundfont load"),
        );
        let mut cg = ChannelGroup::new(ChannelGroupConfig {
            channel_init_options: Default::default(),
            format: SynthFormat::Custom { channels: 32 },
            audio_params: stream_params,
            parallelism: ParallelismOptions {
                channel: xsynth_core::channel_group::ThreadCount::None,
                key: xsynth_core::channel_group::ThreadCount::None,
            },
        });
        cg.send_event(XEvent::Channel(
            3,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf])),
        ));
        let xevents: Vec<(u64, XEvent)> = events
            .iter()
            .map(|e| match e {
                yinhe_synth::SynthEvent::NoteOn {
                    sample,
                    key,
                    velocity,
                    ..
                } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: *key,
                            vel: *velocity,
                        }),
                    ),
                ),
                yinhe_synth::SynthEvent::NoteOff { sample, key, .. } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                    ),
                ),
                yinhe_synth::SynthEvent::Control { .. } => unreachable!(),
            })
            .collect();
        let mut cpu_out: Vec<f32> = Vec::new();
        let mut chunk = vec![0.0f32; frames_per_chunk * 2];
        let mut cursor = 0usize;
        let mut rendered = 0u64;
        while rendered < total_frames {
            while cursor < xevents.len() && xevents[cursor].0 <= rendered {
                cg.send_event(xevents[cursor].1.clone());
                cursor += 1;
            }
            let seg_end = xevents
                .get(cursor)
                .map(|(s, _)| (*s).min(total_frames))
                .unwrap_or(total_frames);
            let seg_frames = seg_end - rendered;
            let mut done = 0u64;
            while done < seg_frames {
                let n = ((seg_frames - done) as usize).min(frames_per_chunk);
                let buf = &mut chunk[..n * 2];
                cg.read_samples(buf);
                cpu_out.extend_from_slice(buf);
                done += n as u64;
            }
            rendered = seg_end;
        }
        for v in cpu_out.iter_mut() {
            *v *= std::f32::consts::SQRT_2;
        }
        {
            let mut w = hound::WavWriter::create(format!("/tmp/mb_{tag}_cpu.wav"), spec).unwrap();
            for &v in cpu_out.iter() {
                w.write_sample(v).unwrap();
            }
            w.finalize().unwrap();
        }

        let n = gpu_out.len().min(cpu_out.len());
        let mut max_diff = 0.0f32;
        let mut max_at = 0usize;
        let mut sse = 0.0f64;
        let mut sref = 0.0f64;
        for i in 0..n {
            let d = (gpu_out[i] - cpu_out[i]).abs();
            if d > max_diff {
                max_diff = d;
                max_at = i;
            }
            sse += ((gpu_out[i] - cpu_out[i]) as f64).powi(2);
            sref += (cpu_out[i] as f64).powi(2);
        }
        let rel = (sse / sref.max(1e-12)).sqrt();
        eprintln!(
            "[key {tag}] rel_rmse={rel:.4} max_diff={max_diff:.4} @{:.3}s",
            max_at as f64 / 2.0 / sr as f64
        );
        // 逐块（4096 帧 = 85ms）ratio + max diff 位置的波形
        eprintln!(
            "  [{tag}] max_diff={max_diff:.4} @{:.4}s",
            max_at as f64 / 2.0 / sr as f64
        );
        let a = max_at.saturating_sub(8);
        let b = (max_at + 8).min(n);
        eprintln!(
            "    GPU: {:?}",
            gpu_out[a..b]
                .iter()
                .map(|x| format!("{x:.4}"))
                .collect::<Vec<_>>()
        );
        eprintln!(
            "    CPU: {:?}",
            cpu_out[a..b]
                .iter()
                .map(|x| format!("{x:.4}"))
                .collect::<Vec<_>>()
        );
        let block = 4096usize * 2;
        let mut k = 0usize;
        let mut ratios = Vec::new();
        while k + block <= n {
            let gr: f64 = gpu_out[k..k + block]
                .iter()
                .map(|&x| (x as f64).powi(2))
                .sum::<f64>()
                .sqrt();
            let cr: f64 = cpu_out[k..k + block]
                .iter()
                .map(|&x| (x as f64).powi(2))
                .sum::<f64>()
                .sqrt();
            ratios.push(format!("{:.3}", gr / cr.max(1e-12)));
            k += block;
        }
        eprintln!("    [{tag}] 每块(85ms) ratio: {}", ratios.join(" "));
    }
}

/// 诊断：单音符 + 密集 pitch bend（每 100 samples 一个）GPU vs xsynth 对比。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要 SoundFont"]
fn diag_pitch_bend_dense() {
    use std::sync::Arc;
    use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let frames_per_chunk = 512usize;

    for (tag, pb_every) in [
        ("无PB", 0u64),
        ("每2400smp", 2400),
        ("每480smp", 480),
        ("每100smp", 100),
    ] {
        let total_frames = 2 * sr as u64;
        // 事件构造顺序与生产 build_gpu_synth_events 一致：CC/PB 先于音符
        //（stable sort 后同 sample 时先处理 CC，与 CPU dispatch 同序）。
        let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();
        if pb_every > 0 {
            let mut i = 0u64;
            while i * pb_every < 2 * sr as u64 {
                let v = if i.is_multiple_of(2) { 0.5 } else { -0.5 };
                events.push(yinhe_synth::SynthEvent::Control {
                    sample: i * pb_every,
                    channel: 3,
                    event: yinhe_synth::ControlEvent::PitchBend(v),
                });
                i += 1;
            }
        }
        events.push(yinhe_synth::SynthEvent::NoteOn {
            sample: 0,
            channel: 3,
            key: 60,
            velocity: 127,
            end_sample: 2 * sr as u64 - 1,
        });
        events.sort_by_key(|e| e.sample());

        // GPU
        let mut gpu = yinhe_synth::GpuSynth::new_default(sr).unwrap();
        gpu.load_dense_soundfonts(3, &[std::path::PathBuf::from(sfz)])
            .unwrap();
        gpu.finish_soundfont_load();
        gpu.load_events(events.clone());
        gpu.seek(0);
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..4)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames_per_chunk],
                right: vec![0.0; frames_per_chunk],
            })
            .collect();
        let mut gpu_out: Vec<f32> = Vec::new();
        while (gpu_out.len() as u64) / 2 < total_frames {
            gpu.render_to_mixer(&mut bufs);
            for i in 0..frames_per_chunk {
                gpu_out.push(bufs[3].left[i]);
                gpu_out.push(bufs[3].right[i]);
            }
        }

        // CPU
        let stream_params = AudioStreamParams {
            channels: ChannelCount::Stereo,
            sample_rate: sr,
        };
        let sf = Arc::new(
            SampleSoundfont::new(
                std::path::PathBuf::from(sfz),
                stream_params,
                SoundfontInitOptions::default(),
            )
            .expect("soundfont load"),
        );
        let mut cg = ChannelGroup::new(ChannelGroupConfig {
            channel_init_options: Default::default(),
            format: SynthFormat::Custom { channels: 32 },
            audio_params: stream_params,
            parallelism: ParallelismOptions {
                channel: xsynth_core::channel_group::ThreadCount::None,
                key: xsynth_core::channel_group::ThreadCount::None,
            },
        });
        cg.send_event(XEvent::Channel(
            3,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf])),
        ));
        let xevents: Vec<(u64, XEvent)> = events
            .iter()
            .map(|e| match e {
                yinhe_synth::SynthEvent::NoteOn {
                    sample,
                    key,
                    velocity,
                    ..
                } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: *key,
                            vel: *velocity,
                        }),
                    ),
                ),
                yinhe_synth::SynthEvent::NoteOff { sample, key, .. } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                    ),
                ),
                yinhe_synth::SynthEvent::Control { sample, event, .. } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::Control(match event {
                            yinhe_synth::ControlEvent::PitchBend(v) => {
                                ControlEvent::PitchBendValue(*v)
                            }
                            _ => ControlEvent::Raw(0, 0),
                        })),
                    ),
                ),
            })
            .collect();
        let mut cpu_out: Vec<f32> = Vec::new();
        let mut chunk = vec![0.0f32; frames_per_chunk * 2];
        let mut cursor = 0usize;
        let mut rendered = 0u64;
        while rendered < total_frames {
            while cursor < xevents.len() && xevents[cursor].0 <= rendered {
                cg.send_event(xevents[cursor].1.clone());
                cursor += 1;
            }
            let seg_end = xevents
                .get(cursor)
                .map(|(s, _)| (*s).min(total_frames))
                .unwrap_or(total_frames);
            let seg_frames = seg_end - rendered;
            let mut done = 0u64;
            while done < seg_frames {
                let n = ((seg_frames - done) as usize).min(frames_per_chunk);
                let buf = &mut chunk[..n * 2];
                cg.read_samples(buf);
                cpu_out.extend_from_slice(buf);
                done += n as u64;
            }
            rendered = seg_end;
        }
        for v in cpu_out.iter_mut() {
            *v *= std::f32::consts::SQRT_2;
        }

        let n = gpu_out.len().min(cpu_out.len());
        let mut max_diff = 0.0f32;
        let mut max_at = 0usize;
        let mut sse = 0.0f64;
        let mut sref = 0.0f64;
        for i in 0..n {
            let d = (gpu_out[i] - cpu_out[i]).abs();
            if d > max_diff {
                max_diff = d;
                max_at = i;
            }
            sse += ((gpu_out[i] - cpu_out[i]) as f64).powi(2);
            sref += (cpu_out[i] as f64).powi(2);
        }
        let rel = (sse / sref.max(1e-12)).sqrt();
        eprintln!(
            "[{tag}] rel_rmse={rel:.4} max_diff={max_diff:.4} @{:.3}s",
            max_at as f64 / 2.0 / sr as f64
        );
    }
}

/// 诊断：单个 PB 的最小复现（导出 wav 供频率分析）。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要 SoundFont"]
fn diag_pitch_bend_single() {
    use std::sync::Arc;
    use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let frames_per_chunk = 512usize;
    let total_frames = 5 * sr as u64;

    // 5 秒长音（检验 GPU shader 的 f32 time 累积漂移）
    let mut events: Vec<yinhe_synth::SynthEvent> = vec![yinhe_synth::SynthEvent::NoteOn {
        sample: 0,
        channel: 3,
        key: 60,
        velocity: 127,
        end_sample: 5 * sr as u64,
    }];
    events.sort_by_key(|e| e.sample());

    // GPU
    let mut gpu = yinhe_synth::GpuSynth::new_default(sr).unwrap();
    gpu.load_dense_soundfonts(3, &[std::path::PathBuf::from(sfz)])
        .unwrap();
    gpu.finish_soundfont_load();
    gpu.load_events(events.clone());
    gpu.seek(0);
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..4)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames_per_chunk],
            right: vec![0.0; frames_per_chunk],
        })
        .collect();
    let mut gpu_out: Vec<f32> = Vec::new();
    while (gpu_out.len() as u64) / 2 < total_frames {
        gpu.render_to_mixer(&mut bufs);
        for i in 0..frames_per_chunk {
            gpu_out.push(bufs[3].left[i]);
            gpu_out.push(bufs[3].right[i]);
        }
    }

    // CPU
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: sr,
    };
    let sf = Arc::new(
        SampleSoundfont::new(
            std::path::PathBuf::from(sfz),
            stream_params,
            SoundfontInitOptions::default(),
        )
        .expect("sf"),
    );
    let mut cg = ChannelGroup::new(ChannelGroupConfig {
        channel_init_options: Default::default(),
        format: SynthFormat::Custom { channels: 32 },
        audio_params: stream_params,
        parallelism: ParallelismOptions {
            channel: xsynth_core::channel_group::ThreadCount::None,
            key: xsynth_core::channel_group::ThreadCount::None,
        },
    });
    cg.send_event(XEvent::Channel(
        3,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf])),
    ));
    let xevents: Vec<(u64, XEvent)> = events
        .iter()
        .map(|e| match e {
            yinhe_synth::SynthEvent::NoteOn {
                sample,
                key,
                velocity,
                ..
            } => (
                *sample,
                XEvent::Channel(
                    3,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                        key: *key,
                        vel: *velocity,
                    }),
                ),
            ),
            yinhe_synth::SynthEvent::NoteOff { sample, key, .. } => (
                *sample,
                XEvent::Channel(
                    3,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                ),
            ),
            yinhe_synth::SynthEvent::Control { sample, event, .. } => (
                *sample,
                XEvent::Channel(
                    3,
                    ChannelEvent::Audio(ChannelAudioEvent::Control(match event {
                        yinhe_synth::ControlEvent::PitchBend(v) => ControlEvent::PitchBendValue(*v),
                        _ => ControlEvent::Raw(0, 0),
                    })),
                ),
            ),
        })
        .collect();
    let mut cpu_out: Vec<f32> = Vec::new();
    let mut chunk = vec![0.0f32; frames_per_chunk * 2];
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < total_frames {
        while cursor < xevents.len() && xevents[cursor].0 <= rendered {
            cg.send_event(xevents[cursor].1.clone());
            cursor += 1;
        }
        let seg_end = xevents
            .get(cursor)
            .map(|(s, _)| (*s).min(total_frames))
            .unwrap_or(total_frames);
        let seg_frames = seg_end - rendered;
        let mut done = 0u64;
        while done < seg_frames {
            let n = ((seg_frames - done) as usize).min(frames_per_chunk);
            cg.read_samples(&mut chunk[..n * 2]);
            cpu_out.extend_from_slice(&chunk[..n * 2]);
            done += n as u64;
        }
        rendered = seg_end;
    }
    for v in cpu_out.iter_mut() {
        *v *= std::f32::consts::SQRT_2;
    }

    for (name, data) in [("gpu", &gpu_out), ("cpu", &cpu_out)] {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let path = format!("/tmp/pb_single_{name}.wav");
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for &v in data.iter() {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        eprintln!("导出 {path}");
    }
}

/// 诊断：纯音符 + 单个 CC，找出让 GPU 崩坏的 CC。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn diag_ch3_cc_bisect() {
    use std::sync::Arc;
    use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

    let midi = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let frames_per_chunk = 512usize;
    let total_frames = sr as u64;

    let model = Arc::new(yinhe_midi::parse_path(midi).unwrap());
    let active = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();
    let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(active));
    engine.handle_command(AudioCommand::LoadModel { model });
    let all = engine.build_gpu_events(0);
    // ch3 前 1 秒的纯音符
    let notes: Vec<yinhe_synth::SynthEvent> = all
        .iter()
        .filter(|e| {
            matches!(
                e,
                yinhe_synth::SynthEvent::NoteOn { channel: 3, .. }
                    | yinhe_synth::SynthEvent::NoteOff { channel: 3, .. }
            ) && e.sample() < total_frames
        })
        .copied()
        .collect();
    // ch3 前 1 秒的 CC（非 PB）
    let ccs: Vec<yinhe_synth::SynthEvent> = all
        .iter()
        .filter(|e| {
            matches!(e, yinhe_synth::SynthEvent::Control { channel: 3, .. })
                && e.sample() < total_frames
                && !matches!(
                    e,
                    yinhe_synth::SynthEvent::Control {
                        event: yinhe_synth::ControlEvent::PitchBend(_),
                        ..
                    }
                )
        })
        .copied()
        .collect();

    let sf = Arc::new(
        SampleSoundfont::new(
            std::path::PathBuf::from(sfz),
            AudioStreamParams {
                channels: ChannelCount::Stereo,
                sample_rate: sr,
            },
            SoundfontInitOptions::default(),
        )
        .expect("sf"),
    );

    let render_gpu = |events: &[yinhe_synth::SynthEvent]| -> (f32, f32) {
        let mut gpu = yinhe_synth::GpuSynth::new_default(sr).unwrap();
        gpu.load_dense_soundfonts(3, &[std::path::PathBuf::from(sfz)])
            .unwrap();
        gpu.finish_soundfont_load();
        gpu.load_events(events.to_vec());
        gpu.seek(0);
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..4)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames_per_chunk],
                right: vec![0.0; frames_per_chunk],
            })
            .collect();
        let mut peak = 0.0f32;
        let mut n = 0u64;
        while n < total_frames {
            gpu.render_to_mixer(&mut bufs);
            for i in 0..frames_per_chunk {
                peak = peak.max(bufs[3].left[i].abs()).max(bufs[3].right[i].abs());
            }
            n += frames_per_chunk as u64;
        }
        (peak, 0.0)
    };

    let render_cpu = |events: &[yinhe_synth::SynthEvent]| -> f32 {
        let mut cg = ChannelGroup::new(ChannelGroupConfig {
            channel_init_options: Default::default(),
            format: SynthFormat::Custom { channels: 32 },
            audio_params: AudioStreamParams {
                channels: ChannelCount::Stereo,
                sample_rate: sr,
            },
            parallelism: ParallelismOptions {
                channel: xsynth_core::channel_group::ThreadCount::None,
                key: xsynth_core::channel_group::ThreadCount::None,
            },
        });
        cg.send_event(XEvent::Channel(
            3,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf.clone()])),
        ));
        let xevents: Vec<(u64, XEvent)> = events
            .iter()
            .map(|e| match e {
                yinhe_synth::SynthEvent::NoteOn {
                    sample,
                    key,
                    velocity,
                    ..
                } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: *key,
                            vel: *velocity,
                        }),
                    ),
                ),
                yinhe_synth::SynthEvent::NoteOff { sample, key, .. } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                    ),
                ),
                yinhe_synth::SynthEvent::Control { sample, event, .. } => (
                    *sample,
                    XEvent::Channel(
                        3,
                        ChannelEvent::Audio(ChannelAudioEvent::Control(match event {
                            yinhe_synth::ControlEvent::Raw(c, v) => ControlEvent::Raw(*c, *v),
                            yinhe_synth::ControlEvent::PitchBend(v) => {
                                ControlEvent::PitchBendValue(*v)
                            }
                            yinhe_synth::ControlEvent::PitchBendSensitivity(v) => {
                                ControlEvent::PitchBendSensitivity(*v)
                            }
                            yinhe_synth::ControlEvent::FineTune(v) => ControlEvent::FineTune(*v),
                            yinhe_synth::ControlEvent::CoarseTune(v) => {
                                ControlEvent::CoarseTune(*v)
                            }
                            yinhe_synth::ControlEvent::ProgramChange(p) => {
                                return (
                                    *sample,
                                    XEvent::Channel(
                                        3,
                                        ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(*p)),
                                    ),
                                );
                            }
                            _ => ControlEvent::Raw(0, 0),
                        })),
                    ),
                ),
            })
            .collect();
        let mut out: Vec<f32> = Vec::new();
        let mut chunk = vec![0.0f32; frames_per_chunk * 2];
        let mut cursor = 0usize;
        let mut rendered = 0u64;
        while rendered < total_frames {
            while cursor < xevents.len() && xevents[cursor].0 <= rendered {
                cg.send_event(xevents[cursor].1.clone());
                cursor += 1;
            }
            let seg_end = xevents
                .get(cursor)
                .map(|(s, _)| (*s).min(total_frames))
                .unwrap_or(total_frames);
            let seg_frames = seg_end - rendered;
            let mut done = 0u64;
            while done < seg_frames {
                let n = ((seg_frames - done) as usize).min(frames_per_chunk);
                cg.read_samples(&mut chunk[..n * 2]);
                out.extend_from_slice(&chunk[..n * 2]);
                done += n as u64;
            }
            rendered = seg_end;
        }
        out.iter().fold(0.0f32, |m, &v| m.max(v.abs())) * std::f32::consts::SQRT_2
    };

    // 基线：纯音符（各 CC 单独加入）
    let cpu_peak = render_cpu(&notes);
    let (gpu_peak, _) = render_gpu(&notes);
    eprintln!("[baseline 纯音符] gpu_peak={gpu_peak:.3} cpu_peak={cpu_peak:.3}");

    // 全部 16 个 CC 一起
    {
        let mut ev = notes.clone();
        ev.extend(ccs.iter().copied());
        ev.sort_by_key(|e| e.sample());
        let tag = format!("全部{}CC", ccs.len());
        let (gp, _) = render_gpu(&ev);
        let cp = render_cpu(&ev);
        let ratio = gp / cp.max(1e-9);
        let flag = if !(0.5..=1.5).contains(&ratio) {
            " <<< 崩坏"
        } else {
            ""
        };
        eprintln!("[{tag}] gpu={gp:.3} cpu={cp:.3} ratio={ratio:.3}{flag}");
    }

    for cc in ccs.iter() {
        let mut ev = notes.clone();
        ev.push(*cc);
        ev.sort_by_key(|e| e.sample());
        let (gp, _) = render_gpu(&ev);
        let cp = render_cpu(&ev);
        let tag = match cc {
            yinhe_synth::SynthEvent::Control { sample, event, .. } => {
                format!("{event:?} @{:.3}s", *sample as f64 / sr as f64)
            }
            _ => "?".into(),
        };
        let ratio = gp / cp.max(1e-9);
        let flag = if !(0.5..=1.5).contains(&ratio) {
            " <<< 崩坏"
        } else {
            ""
        };
        eprintln!("[+{tag}] gpu={gp:.3} cpu={cp:.3} ratio={ratio:.3}{flag}");
    }
}

/// 诊断：走 AudioEngine::render（应用真实入口）与裸 GpuSynth 对比。
/// 复现"测试找不到、应用出问题"——定位差异在 render 路径还是合成器。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn diag_app_render_path() {
    use std::sync::Arc;

    let midi = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let secs = 3u64;
    let frames_per_chunk = 512usize;
    let total = secs * sr as u64;

    let model = Arc::new(yinhe_midi::parse_path(midi).unwrap());
    let active = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();
    let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(active));
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::clone(&model),
    });
    let events = engine.build_gpu_events(0);
    eprintln!("事件数={}", events.len());

    // 裸 GpuSynth 参考（与既有测试相同）
    let mut bare = yinhe_synth::GpuSynth::new_default(sr).unwrap();
    let sfz_path = std::path::PathBuf::from(sfz);
    for ch in 0..32u32 {
        bare.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
            .unwrap();
    }
    bare.finish_soundfont_load();
    bare.load_events(events.clone());
    bare.seek(0);

    // 应用路径：engine + gpu_synth + AudioEngine::render
    let mut synth = yinhe_synth::GpuSynth::new_default(sr).unwrap();
    for ch in 0..32u32 {
        synth
            .load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
            .unwrap();
    }
    synth.finish_soundfont_load();
    synth.load_events(events.clone());
    synth.seek(0);
    engine.gpu_synth = Some(synth);
    engine.playing = true;

    let mut out_app = vec![0.0f32; frames_per_chunk * 2];
    let mut app_all: Vec<f32> = Vec::new();
    let mut n = 0u64;
    while n < total {
        engine.render(&mut out_app);
        app_all.extend_from_slice(&out_app);
        n += frames_per_chunk as u64;
    }

    // 裸路径（GpuSynth 直接渲染，通道求和）
    let mut bare_all: Vec<f32> = Vec::new();
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames_per_chunk],
            right: vec![0.0; frames_per_chunk],
        })
        .collect();
    let mut n = 0u64;
    while n < total {
        bare.render_to_mixer(&mut bufs);
        for i in 0..frames_per_chunk {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for b in &bufs {
                l += b.left[i];
                r += b.right[i];
            }
            bare_all.push(l);
            bare_all.push(r);
        }
        n += frames_per_chunk as u64;
    }

    let m = app_all.len().min(bare_all.len());
    let mut max_diff = 0.0f32;
    let mut max_at = 0usize;
    let mut sse = 0.0f64;
    let mut sref = 0.0f64;
    for i in 0..m {
        let d = (app_all[i] - bare_all[i]).abs();
        if d > max_diff {
            max_diff = d;
            max_at = i;
        }
        sse += ((app_all[i] - bare_all[i]) as f64).powi(2);
        sref += (app_all[i] as f64).powi(2);
    }
    eprintln!(
        "app_peak={:.4} bare_peak={:.4} max_diff={max_diff:.4} @{:.3}s rel_rmse={:.4}",
        app_all.iter().fold(0.0f32, |a, &v| a.max(v.abs())),
        bare_all.iter().fold(0.0f32, |a, &v| a.max(v.abs())),
        max_at as f64 / 2.0 / sr as f64,
        (sse / sref.max(1e-12)).sqrt()
    );
    // 补偿 mixer 声像（中心 0.707）后再对比：app × √2 vs bare
    let s2 = std::f32::consts::SQRT_2;
    let mut sse2 = 0.0f64;
    let mut sref2 = 0.0f64;
    let mut max2 = 0.0f32;
    let mut at2 = 0usize;
    for i in 0..m {
        let a = app_all[i] * s2;
        let d = (a - bare_all[i]).abs();
        if d > max2 {
            max2 = d;
            at2 = i;
        }
        sse2 += ((a - bare_all[i]) as f64).powi(2);
        sref2 += (bare_all[i] as f64).powi(2);
    }
    eprintln!(
        "补偿√2后: max_diff={max2:.4} @{:.3}s rel_rmse={:.4}",
        at2 as f64 / 2.0 / sr as f64,
        (sse2 / sref2.max(1e-12)).sqrt()
    );
    // 每 0.25s 的差异分布
    let w = (sr / 4) as usize * 2;
    let mut i = 0usize;
    let mut line = String::new();
    while i + w <= m {
        let d: f64 = (0..w)
            .map(|k| ((app_all[i + k] * s2 - bare_all[i + k]) as f64).powi(2))
            .sum();
        line.push_str(&format!("{:.3} ", (d / w as f64).sqrt()));
        i += w;
    }
    eprintln!("每0.25s差异(补偿后): {line}");
}

/// 诊断：同一模型下 CPU 路径（xsynth）vs GPU 路径（GpuSynth）经完整 render 链路的输出对比。
/// 这是"只有 GPU 出问题"的直接验证。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn diag_engine_cpu_vs_gpu() {
    use std::sync::Arc;

    let midi = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let secs = 3u64;
    let frames_per_chunk = 512usize;
    let total = secs * sr as u64;

    let render = |use_gpu: bool| -> Vec<f32> {
        let model = Arc::new(yinhe_midi::parse_path(midi).unwrap());
        let active = crate::spawn::channels_for_model(&model)
            .active_mask()
            .to_vec();
        let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(active));
        engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });

        if use_gpu {
            let events = engine.build_gpu_events(0);
            let sfz_path = std::path::PathBuf::from(sfz);
            let mut synth = yinhe_synth::GpuSynth::new_default(sr).unwrap();
            let mut used = std::collections::BTreeSet::new();
            for e in &events {
                match e {
                    yinhe_synth::SynthEvent::NoteOn { channel, .. }
                    | yinhe_synth::SynthEvent::NoteOff { channel, .. }
                    | yinhe_synth::SynthEvent::Control { channel, .. } => {
                        used.insert(*channel);
                    }
                }
            }
            for ch in &used {
                synth
                    .load_dense_soundfonts(*ch as u32, std::slice::from_ref(&sfz_path))
                    .unwrap();
            }
            synth.finish_soundfont_load();
            synth.load_events(events);
            synth.seek(0);
            engine.gpu_synth = Some(synth);
        } else {
            // CPU 路径：xsynth 需要音色库。测试里通过 same-channel soundfont 注入。
            // engine 的 channel_set 由 worker 异步配置，这里直接逐通道加载。
            let sfz_path = std::path::PathBuf::from(sfz);
            let stream_params = xsynth_core::AudioStreamParams {
                channels: xsynth_core::ChannelCount::Stereo,
                sample_rate: sr,
            };
            let sf = std::sync::Arc::new(
                xsynth_core::soundfont::SampleSoundfont::new(
                    sfz_path,
                    stream_params,
                    xsynth_core::soundfont::SoundfontInitOptions {
                        use_effects: false,
                        ..Default::default()
                    },
                )
                .expect("sf"),
            );
            let base: std::sync::Arc<dyn xsynth_core::soundfont::SoundfontBase> = sf;
            for ch in 0..16u8 {
                let dense = engine.channel_layout.dense_for(ch as usize);
                engine.apply_loaded_soundfont_for_channel(ch, dense, vec![Arc::clone(&base)]);
            }
            engine.playing = true;
        }

        engine.playing = true;
        let mut out = vec![0.0f32; frames_per_chunk * 2];
        let mut all: Vec<f32> = Vec::new();
        let mut n = 0u64;
        while n < total {
            engine.render(&mut out);
            all.extend_from_slice(&out);
            n += frames_per_chunk as u64;
        }
        all
    };

    let cpu = render(false);
    let gpu = render(true);
    let m = cpu.len().min(gpu.len());
    let mut max_diff = 0.0f32;
    let mut at = 0usize;
    let mut sse = 0.0f64;
    let mut sref = 0.0f64;
    for i in 0..m {
        let d = (cpu[i] - gpu[i]).abs();
        if d > max_diff {
            max_diff = d;
            at = i;
        }
        sse += ((cpu[i] - gpu[i]) as f64).powi(2);
        sref += (cpu[i] as f64).powi(2);
    }
    eprintln!(
        "cpu_peak={:.4} gpu_peak={:.4} max_diff={max_diff:.4} @{:.3}s rel_rmse={:.4}",
        cpu.iter().fold(0.0f32, |a, &v| a.max(v.abs())),
        gpu.iter().fold(0.0f32, |a, &v| a.max(v.abs())),
        at as f64 / 2.0 / sr as f64,
        (sse / sref.max(1e-12)).sqrt()
    );
    // 每 0.25s 差异
    let w = (sr / 4) as usize * 2;
    let mut i = 0usize;
    let mut line = String::new();
    while i + w <= m {
        let d: f64 = (0..w)
            .map(|k| ((cpu[i + k] - gpu[i + k]) as f64).powi(2))
            .sum();
        line.push_str(&format!("{:.3} ", (d / w as f64).sqrt()));
        i += w;
    }
    eprintln!("每0.25s差异: {line}");
    // 补偿 xsynth 通道 pan（中心 0.707）后对比
    let s2 = std::f32::consts::SQRT_2;
    let mut sse2 = 0.0f64;
    let mut sref2 = 0.0f64;
    let mut max2 = 0.0f32;
    let mut at2 = 0usize;
    for i in 0..m {
        let cc = cpu[i] * s2;
        let d = (cc - gpu[i]).abs();
        if d > max2 {
            max2 = d;
            at2 = i;
        }
        sse2 += ((cc - gpu[i]) as f64).powi(2);
        sref2 += (cc as f64).powi(2);
    }
    eprintln!(
        "补偿 pan(√2)后: max_diff={max2:.4} @{:.3}s rel_rmse={:.4}",
        at2 as f64 / 2.0 / sr as f64,
        (sse2 / sref2.max(1e-12)).sqrt()
    );
    let w2 = (sr / 4) as usize * 2;
    let mut i2 = 0usize;
    let mut line2 = String::new();
    while i2 + w2 <= m {
        let d: f64 = (0..w2)
            .map(|k| ((cpu[i2 + k] * s2 - gpu[i2 + k]) as f64).powi(2))
            .sum();
        line2.push_str(&format!("{:.3} ", (d / w2 as f64).sqrt()));
        i2 += w2;
    }
    eprintln!("补偿后每0.25s差异: {line2}");

    // 导出 wav
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    for (name, data) in [("cpu", &cpu), ("gpu", &gpu)] {
        let mut wr = hound::WavWriter::create(format!("/tmp/eng_{name}.wav"), spec).unwrap();
        for &v in data.iter() {
            wr.write_sample(v).unwrap();
        }
        wr.finalize().unwrap();
    }
    eprintln!("已导出 /tmp/eng_cpu.wav /tmp/eng_gpu.wav");
}

// ---------------------------------------------------------------------------
// 通道处理回归音源：CC7/10/11/71/74 → 内置音源通道处理段
// ---------------------------------------------------------------------------

/// 内置音源通道：CC7 由通道处理段消费（dispatch 打点），不再有外挂广播。
#[test]
fn low_level_cc_routes_to_channel_dsp() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 1,
            events: vec![AutomationEvent {
                tick: 0,
                value: 100.0 / 127.0,
                shape: SegmentShape::Step,
            }],
        });
    }
    doc.data.bump_revision();

    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    engine.dispatch_and_find_next(0, 60_000);

    // 打点 = 该 CC 已被实际消费（内置音源通道处理段）；外挂 insert 不再收 CC。
    let skip = engine.chase_skip();
    assert!(
        skip.cc_mask[0] & (1u128 << 7) != 0,
        "CC7 应由内置音源通道处理段消费"
    );
}

/// 音源层 CC（如 Sustain=CC64）不进通道处理段（由合成器消费）。
#[test]
fn source_level_cc_not_routed_to_channel_dsp() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 64 },
            track: 1,
            events: vec![AutomationEvent {
                tick: 0,
                value: 1.0,
                shape: SegmentShape::Step,
            }],
        });
    }
    doc.data.bump_revision();

    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playing = true;
    engine.dispatch_and_find_next(0, 60_000);

    // CC64 走常规路径（xsynth 处理），不被通道处理段接管（处理段只管 DSP_CHANNEL_CCS）。
    let skip = engine.chase_skip();
    assert!(
        skip.cc_mask[0] & (1u128 << 64) != 0,
        "CC64 应走常规路径并打点（xsynth 消费）"
    );
}

/// yinhe CPU 后端（CpuSynth）经完整引擎链路的冒烟：加载音色库 → Play →
/// dispatch 投递事件 → 渲染出非零输出。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 SoundFont"]
fn yinhe_cpu_engine_render_smoke() {
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = make_model_with_notes(vec![(60, 0, 4_800, 100, 0)]);
    let layout = crate::spawn::channels_for_model(&model);
    let mut engine = AudioEngine::new(sr, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: std::sync::Arc::new(model),
    });

    let mut synth = yinhe_synth::CpuSynth::new(sr);
    synth
        .load_dense_soundfonts(0, &[std::path::PathBuf::from(sfz)])
        .expect("音色库加载");
    engine.cpu_synth = Some(synth);

    engine.handle_command(AudioCommand::Play { from_sample: 0 });
    let mut out = vec![0.0f32; 512 * 2];
    let mut peak = 0.0f32;
    for _ in 0..10 {
        engine.render(&mut out);
        peak = peak.max(out.iter().fold(0.0f32, |m, v| m.max(v.abs())));
    }
    assert!(peak > 0.0, "yinhe CPU 后端应渲染出输出（peak={peak}）");
}

/// 画像（本地 MIDI + SoundFont，ignored）：cyber-night 第 92 小节密集段，
/// 三合成后端（xsynth CPU / yinhe CPU / yinhe GPU）的实时倍数与峰值 voice。
///
/// 输出可直接用于选择黑乐谱的优化方向（并行/分页/流水线）。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn prof_cyber_night_dense_section() {
    use std::time::Instant;

    let midi = "/Users/jieneng/Music/MIDIs/cyber-night.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse cyber-night"));
    let ppq = model.meta.ppq;
    let bar92_tick = 91 * 4 * ppq; // 4/4：第 92 小节起点
    let total_notes: usize = (0..128).map(|k| model.notes[k].len()).sum();
    eprintln!("=== cyber-night: ppq={ppq} notes={total_notes} bar92_tick={bar92_tick}");

    // 密度画像：bar92 前后各 8 小节的每拍音符数（找该段峰值）
    let mut per_beat: std::collections::BTreeMap<u32, usize> = Default::default();
    for k in 0..128 {
        for n in model.notes[k].iter() {
            let beat = n.start_tick / ppq;
            if n.start_tick < bar92_tick + 16 * 4 * ppq && n.start_tick + 16 * 4 * ppq >= bar92_tick
            {
                *per_beat.entry(beat).or_default() += 1;
            }
        }
    }
    let peak = per_beat.iter().max_by_key(|(_, v)| **v);
    if let Some((beat, count)) = peak {
        eprintln!(
            "  bar92 周围每拍音符峰值: beat={beat}（约第 {} 小节）count={count}",
            beat / 4 + 1
        );
    }

    let layout = crate::spawn::channels_for_model(&model);
    let active: Vec<u8> = (0..16u8)
        .filter(|c| layout.is_active(*c as usize))
        .collect();
    eprintln!("  active channels: {active:?}");

    for (name, backend) in [("xsynth-cpu", 0u8), ("yinhe-cpu", 1), ("yinhe-gpu", 2)] {
        let mut engine = AudioEngine::new(sr, layout.clone());
        engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        // 音色库（三后端各自的加载路径）
        let t_sf = Instant::now();
        match backend {
            0 => {
                let configs: Vec<(u8, Vec<String>)> =
                    active.iter().map(|c| (*c, vec![sfz.to_string()])).collect();
                engine.handle_command(AudioCommand::SetSoundFonts {
                    configs: Box::new(configs),
                });
            }
            1 => {
                let mut cs = yinhe_synth::CpuSynth::new(sr);
                for c in &active {
                    cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                        .expect("cpu sf load");
                }
                engine.cpu_synth = Some(cs);
            }
            _ => {
                let mut gs = yinhe_synth::GpuSynth::new_default(sr).expect("gpu init");
                for c in &active {
                    gs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                        .expect("gpu sf load");
                }
                gs.finish_soundfont_load();
                engine.gpu_synth = Some(gs);
            }
        }
        let sf_ms = t_sf.elapsed().as_secs_f64() * 1000.0;

        // 从 92 小节前 1 秒开始（跳过 seek 后 chase 的静默期）
        let target_tick = bar92_tick.saturating_sub(ppq * 2);
        let start_sample = engine.tick_to_sample(target_tick);
        engine.handle_command(AudioCommand::Play {
            from_sample: start_sample,
        });

        // GPU 模式用 4096 块（引擎实际块长），CPU 用 512
        let frames = if backend == 2 { 4096 } else { 512 };
        let mut buf = vec![0.0f32; frames * 2];
        let target: u64 = 3 * sr as u64;
        let mut rendered = 0u64;
        let mut peak_voice = 0u64;
        let mut max_chunk_ms = 0.0f64;
        let t0 = Instant::now();
        while rendered < target {
            // GPU：事件表同步（renderer 的 run 循环职责；测试路径手动调用）
            #[cfg(feature = "gpu")]
            if backend == 2 {
                engine.sync_gpu_backend();
            }
            let tc = Instant::now();
            engine.render(&mut buf);
            let dt = tc.elapsed().as_secs_f64() * 1000.0;
            rendered += frames as u64;
            peak_voice = peak_voice.max(engine.voice_count());
            if rendered > sr as u64 {
                max_chunk_ms = max_chunk_ms.max(dt);
            }
        }
        let el = t0.elapsed().as_secs_f64();
        eprintln!(
            "  {name:<10} 音色库={sf_ms:>6.0}ms 3s音频={el:>6.2}s ({:.2}x realtime) peak_voice={peak_voice} max_chunk={max_chunk_ms:.1}ms",
            3.0 / el.max(1e-9)
        );
    }
}

/// 画像（本地 MIDI + SoundFont，ignored）：经典黑乐谱成本分解。
///
/// - `tau2.5.9.mid`：628 万音符，经典布局（音符为主）
/// - `cyber-night.mid`：CC 密集（控制器多的黑乐谱）
///
/// 每首曲子：三后端 baseline + CpuSynth 降级分解（全功能/无滤波/无采样/
/// 仅包络）——差值即各阶段成本占比，用于决定块级重写的收益点。
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn prof_blackmidi_cost_breakdown() {
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    // 渲染秒数可配（`YINHE_PROF_SECS`，默认 3）：峰值段实时倍率低时
    // 用 1 秒快速迭代（AGENTS 五-7 单项测试 ≤120s）。
    let prof_secs: f64 = std::env::var("YINHE_PROF_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);

    for midi in [
        "/Users/jieneng/Music/MIDIs/tau2.5.9.mid",
        "/Users/jieneng/Music/MIDIs/cyber-night.mid",
    ] {
        let t_parse = Instant::now();
        let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse"));
        let total: usize = (0..128).map(|k| model.notes[k].len()).sum();
        let ppq = model.meta.ppq;
        // 密度峰值（每拍音符数）
        let mut per_beat: std::collections::BTreeMap<u32, usize> = Default::default();
        for k in 0..128 {
            for n in model.notes[k].iter() {
                *per_beat.entry(n.start_tick / ppq).or_default() += 1;
            }
        }
        let (peak_beat, peak_count) = per_beat
            .iter()
            .max_by_key(|(_, v)| **v)
            .map(|(b, c)| (*b, *c))
            .expect("non-empty");
        eprintln!(
            "\n=== {}\n  parse={:.1}s notes={total} ppq={ppq} 峰值拍={peak_beat}（约第 {} 小节）count={peak_count}",
            midi.rsplit('/').next().unwrap_or(midi),
            t_parse.elapsed().as_secs_f64(),
            peak_beat / 4 + 1
        );
        let layout = crate::spawn::channels_for_model(&model);
        let active: Vec<u8> = (0..16u8)
            .filter(|c| layout.is_active(*c as usize))
            .collect();
        let peak_tick = peak_beat.saturating_mul(ppq).saturating_sub(ppq * 2);
        // tick→sample 必须用**加载模型后**的引擎（tempo map 来自模型；
        // 空引擎会退化成默认 120BPM，seek 到错误位置）。
        let start_sample = {
            let mut probe = AudioEngine::new(sr, layout.clone());
            probe.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            probe.tick_to_sample(peak_tick)
        };

        // 渲染 prof_secs 秒（调用方负责 Play/重置）
        let render_secs = |engine: &mut AudioEngine, backend: u8, secs: f64| -> (f64, u64, f64) {
            let frames = if backend == 2 { 4096 } else { 512 };
            let mut buf = vec![0.0f32; frames * 2];
            let target: u64 = (secs * sr as f64) as u64;
            let mut rendered = 0u64;
            let mut peak_voice = 0u64;
            let mut max_chunk = 0.0f64;
            #[cfg(feature = "gpu")]
            if backend == 2 {
                engine.sync_gpu_backend(); // 事件表构建（计时外）
            }
            let t0 = Instant::now();
            while rendered < target {
                #[cfg(feature = "gpu")]
                if backend == 2 {
                    engine.sync_gpu_backend();
                }
                let tc = Instant::now();
                engine.render(&mut buf);
                let dt = tc.elapsed().as_secs_f64() * 1000.0;
                rendered += frames as u64;
                peak_voice = peak_voice.max(engine.voice_count());
                if rendered > sr as u64 {
                    max_chunk = max_chunk.max(dt);
                }
            }
            (t0.elapsed().as_secs_f64(), peak_voice, max_chunk)
        };

        // ── baseline：三后端 ──
        for (name, backend) in [("xsynth-cpu", 0u8), ("yinhe-cpu", 1), ("yinhe-gpu", 2)] {
            let mut engine = AudioEngine::new(sr, layout.clone());
            engine.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            match backend {
                0 => {
                    let configs: Vec<(u8, Vec<String>)> =
                        active.iter().map(|c| (*c, vec![sfz.to_string()])).collect();
                    engine.handle_command(AudioCommand::SetSoundFonts {
                        configs: Box::new(configs),
                    });
                }
                1 => {
                    let mut cs = yinhe_synth::CpuSynth::new(sr);
                    for c in &active {
                        cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                            .expect("cpu sf");
                    }
                    engine.cpu_synth = Some(cs);
                }
                _ => {
                    let mut gs = yinhe_synth::GpuSynth::new_default(sr).expect("gpu");
                    for c in &active {
                        gs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                            .expect("gpu sf");
                    }
                    gs.finish_soundfont_load();
                    engine.gpu_synth = Some(gs);
                }
            }
            engine.handle_command(AudioCommand::Play {
                from_sample: start_sample,
            });
            let (el, pv, mc) = render_secs(&mut engine, backend, prof_secs);
            eprintln!(
                "  {name:<12} {prof_secs}s音频={el:>6.2}s ({:.2}x) peak_voice={pv:>6} max_chunk={mc:>7.1}ms",
                prof_secs / el.max(1e-9)
            );
        }

        // ── CpuSynth 降级分解（同一引擎，模式间 Play 重置）──
        let mut cs_engine = AudioEngine::new(sr, layout.clone());
        cs_engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        let mut cs = yinhe_synth::CpuSynth::new(sr);
        for c in &active {
            cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                .expect("cpu sf");
        }
        cs_engine.cpu_synth = Some(cs);
        for (name, mode) in [("全功能", 0u8), ("无滤波", 1), ("无采样", 2), ("仅包络", 3)]
        {
            yinhe_synth::cpu_synth::CPU_PROFILE_MODE.store(mode, Ordering::Relaxed);
            use std::sync::atomic::AtomicU64;
            let base = |a: &AtomicU64| a.load(Ordering::Relaxed);
            let b_on = base(&yinhe_synth::cpu_synth::PROF_NOTE_ON_NS);
            let b_off = base(&yinhe_synth::cpu_synth::PROF_NOTE_OFF_NS);
            let b_render = base(&yinhe_synth::cpu_synth::PROF_RENDER_NS);
            let b_par = base(&yinhe_synth::cpu_synth::PROF_PAR_NS);
            let b_red = base(&yinhe_synth::cpu_synth::PROF_REDUCE_NS);
            let b_end = base(&yinhe_synth::cpu_synth::PROF_BLOCK_END_NS);
            let b_adv = base(&yinhe_synth::cpu_synth::PROF_ADVANCE_NS);
            let b_ret = base(&yinhe_synth::cpu_synth::PROF_RETAIN_NS);
            let b_reb = base(&yinhe_synth::cpu_synth::PROF_REBUILD_NS);
            cs_engine.handle_command(AudioCommand::Play {
                from_sample: start_sample,
            });
            let (el, pv, mc) = render_secs(&mut cs_engine, 1, prof_secs);
            let d_on = base(&yinhe_synth::cpu_synth::PROF_NOTE_ON_NS) - b_on;
            let d_off = base(&yinhe_synth::cpu_synth::PROF_NOTE_OFF_NS) - b_off;
            let d_render = base(&yinhe_synth::cpu_synth::PROF_RENDER_NS) - b_render;
            let d_par = base(&yinhe_synth::cpu_synth::PROF_PAR_NS) - b_par;
            let d_red = base(&yinhe_synth::cpu_synth::PROF_REDUCE_NS) - b_red;
            let d_end = base(&yinhe_synth::cpu_synth::PROF_BLOCK_END_NS) - b_end;
            let d_adv = base(&yinhe_synth::cpu_synth::PROF_ADVANCE_NS) - b_adv;
            let d_ret = base(&yinhe_synth::cpu_synth::PROF_RETAIN_NS) - b_ret;
            let d_reb = base(&yinhe_synth::cpu_synth::PROF_REBUILD_NS) - b_reb;
            let total_ns = (el * 1e9) as u64;
            let r = d_render.max(1) as f64;
            eprintln!(
                "  CpuSynth-{name:<6} {prof_secs}s音频={el:>6.2}s ({:.2}x) peak_voice={pv:>6} max_chunk={mc:>7.1}ms | note_on={:.0}% note_off={:.0}% render={:.0}%",
                prof_secs / el.max(1e-9),
                100.0 * d_on as f64 / total_ns as f64,
                100.0 * d_off as f64 / total_ns as f64,
                100.0 * d_render as f64 / total_ns as f64,
            );
            eprintln!(
                "    └ render 细分: par={:.0}% reduce={:.0}% block_end={:.0}%（占总渲染）| block_end 内: advance={:.0}% retain={:.0}% rebuild={:.0}%",
                100.0 * d_par as f64 / r,
                100.0 * d_red as f64 / r,
                100.0 * d_end as f64 / r,
                100.0 * d_adv as f64 / d_end.max(1) as f64,
                100.0 * d_ret as f64 / d_end.max(1) as f64,
                100.0 * d_reb as f64 / d_end.max(1) as f64,
            );
        }
        yinhe_synth::cpu_synth::CPU_PROFILE_MODE.store(0, Ordering::Relaxed);
    }
}

/// 分析（本地 MIDI，ignored）：黑乐谱重复音符分布 —— 评估"同参数音符合批"
/// 的收益上限。
///
/// 统计峰值拍 ±8 拍窗口内的三档可合并比例：
/// - 完全重复（key/vel/start/end 全同）：可无损合批（note_off refcount）；
/// - 同起音（key/vel/start 同、长度可能不同）：可共享采样读取；
/// - 同参数（key/vel 同、起音不同）：仅缓存友好（相位不同不可合并）。
#[test]
#[ignore = "需要本地 MIDI"]
fn analyze_blackmidi_duplicates() {
    use std::collections::HashMap;

    let midis = [
        "/Users/jieneng/Music/MIDIs/tau2.5.9.mid",
        "/Users/jieneng/Music/MIDIs/cyber-night.mid",
        "/Users/jieneng/Music/MIDIs/5K 5,555,555 notes by The Atom Bomb.mid",
    ];
    for midi in midis {
        let Ok(model) = yinhe_midi::parse_path(midi) else {
            eprintln!("跳过（解析失败）：{midi}");
            continue;
        };
        let ppq = model.meta.ppq;
        // 峰值拍（每拍音符数）
        let mut per_beat: HashMap<u32, usize> = HashMap::new();
        for k in 0..128 {
            for n in model.notes[k].iter() {
                *per_beat.entry(n.start_tick / ppq.max(1)).or_default() += 1;
            }
        }
        let Some((&peak_beat, &peak_count)) = per_beat.iter().max_by_key(|(_, v)| **v) else {
            continue;
        };
        // 窗口：峰值拍 ±8 拍
        let win_start = peak_beat.saturating_sub(8).saturating_mul(ppq);
        let win_end = (peak_beat + 8).saturating_mul(ppq);

        let mut identical: HashMap<(u8, u8, u32, u32), u32> = HashMap::new();
        let mut identical_ch: HashMap<(u8, u8, u8, u32, u32), u32> = HashMap::new();
        let mut identical_peak: HashMap<(u8, u8, u8, u32, u32), u32> = HashMap::new();
        let track_channels: Vec<u8> = model.tracks.iter().map(|t| t.global_channel()).collect();
        let peak_start = peak_beat.saturating_mul(ppq);
        let peak_end = peak_start.saturating_add(ppq);
        let mut total = 0usize;
        let mut total_peak = 0usize;
        for k in 0..128u8 {
            for n in model.notes[k as usize].iter() {
                if n.start_tick < win_start || n.start_tick >= win_end || n.velocity <= 1 {
                    continue;
                }
                let ch = track_channels.get(n.track as usize).copied().unwrap_or(0);
                total += 1;
                *identical
                    .entry((k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
                *identical_ch
                    .entry((ch, k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
                if n.start_tick >= peak_start && n.start_tick < peak_end {
                    total_peak += 1;
                    *identical_peak
                        .entry((ch, k, n.velocity, n.start_tick, n.end_tick))
                        .or_default() += 1;
                }
            }
        }
        // 冗余率 = 1 - 组数/总数（可省掉的 voice 比例）
        let rate = |groups: usize| 1.0 - groups as f64 / total.max(1) as f64;
        // 峰值时刻"活跃"音符（start <= t < end）：直接对应 voice 峰的可减量
        let t_peak = peak_start.saturating_add(ppq / 2);
        let mut act_all: HashMap<(u8, u8, u32, u32), u32> = HashMap::new();
        let mut act_ch: HashMap<(u8, u8, u8, u32, u32), u32> = HashMap::new();
        let mut act_total = 0usize;
        for k in 0..128u8 {
            for n in model.notes[k as usize].iter() {
                if n.velocity <= 1 || n.start_tick > t_peak || n.end_tick <= t_peak {
                    continue;
                }
                let ch = track_channels.get(n.track as usize).copied().unwrap_or(0);
                act_total += 1;
                *act_all
                    .entry((k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
                *act_ch
                    .entry((ch, k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
            }
        }
        eprintln!(
            "\n=== {}\n  峰值第 {} 拍 count={} 窗口(±8拍)音符={total}\n  完全重复(跨通道)组={} 冗余率={:.1}% 最大组={}\n  完全重复(同通道)组={} 冗余率={:.1}% 最大组={}\n  峰值拍同通道 音符={total_peak} 冗余率={:.1}% 最大组={}",
            midi.rsplit('/').next().unwrap_or(midi),
            peak_beat / 4 + 1,
            peak_count,
            identical.len(),
            100.0 * rate(identical.len()),
            identical.values().copied().max().unwrap_or(0),
            identical_ch.len(),
            100.0 * rate(identical_ch.len()),
            identical_ch.values().copied().max().unwrap_or(0),
            100.0 * (1.0 - identical_peak.len() as f64 / total_peak.max(1) as f64),
            identical_peak.values().copied().max().unwrap_or(0),
        );
        eprintln!(
            "  峰值时刻活跃音符={act_total} 跨通道冗余率={:.1}%（最大组={}）同通道冗余率={:.1}%（最大组={}）",
            100.0 * (1.0 - act_all.len() as f64 / act_total.max(1) as f64),
            act_all.values().copied().max().unwrap_or(0),
            100.0 * (1.0 - act_ch.len() as f64 / act_total.max(1) as f64),
            act_ch.values().copied().max().unwrap_or(0),
        );
    }
}

/// 诊断（本地 MIDI，ignored）：Ouranos Track 13 第 33 小节起的音符模式
/// （同 key 间隔/同帧重复/力度分布）——定位 GPU 丢音与 CPU 的差异来源。
#[test]
#[ignore = "需要本地 MIDI"]
fn analyze_ouranos_track13_dense() {
    let path = "/Users/jieneng/Music/MIDIs/Ouranos - HDSQ & The Romanticist [v1.6.6].mid";
    let model = yinhe_midi::parse_path(path).expect("parse");
    let ppq = model.meta.ppq;
    eprintln!("tracks={} ppq={ppq}", model.tracks.len());
    for track_idx in [12usize, 13] {
        let mut notes: Vec<(u8, u8, u32, u32)> = Vec::new();
        for k in 0..128 {
            for n in model.notes[k].iter() {
                if n.track as usize == track_idx {
                    notes.push((k as u8, n.velocity, n.start_tick, n.end_tick));
                }
            }
        }
        if notes.is_empty() {
            continue;
        }
        notes.sort_by_key(|n| n.2);
        let bar33 = 32 * 4 * ppq;
        let dense: Vec<_> = notes.iter().filter(|n| n.2 >= bar33).take(200).collect();
        if dense.len() < 2 {
            continue;
        }
        let mut gap_count: std::collections::BTreeMap<i64, usize> = Default::default();
        for w in dense.windows(2) {
            *gap_count.entry(w[1].2 as i64 - w[0].2 as i64).or_default() += 1;
        }
        let keys: std::collections::BTreeSet<u8> = dense.iter().map(|n| n.0).collect();
        let vels: std::collections::BTreeSet<u8> = dense.iter().map(|n| n.1).collect();
        let lens: std::collections::BTreeSet<u32> = dense.iter().map(|n| n.3 - n.2).collect();
        eprintln!(
            "track[{track_idx}] 总={} 第33小节起取样={} keys={:?} vels={:?} 长度={:?}",
            notes.len(),
            dense.len(),
            keys.iter().take(6).collect::<Vec<_>>(),
            vels.iter().take(6).collect::<Vec<_>>(),
            lens.iter().take(6).collect::<Vec<_>>()
        );
        eprintln!(
            "  间隔分布(tick→次数，前12)：{:?}",
            gap_count.iter().take(12).collect::<Vec<_>>()
        );
        eprintln!("  前12个(start,end)：{:?}", &dense[..dense.len().min(12)]);
    }
}

/// 诊断（本地 MIDI，ignored）：Ouranos Track 13 第 33 小节 GPU vs CPU 回放
/// 对比（逐 512 帧块能量）——定位 GPU 丢音/断断续续的机制。
#[test]
#[ignore = "需要本地 MIDI + SFZ"]
fn diag_ouranos_gpu_dropout() {
    let sfz = std::env::var("YINHE_TEST_SFZ")
        .unwrap_or_else(|_| "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz".into());
    let midi = std::env::var("YINHE_DIAG_MIDI")
        .unwrap_or_else(|_| {
            "/Users/jieneng/Music/MIDIs/Ouranos - HDSQ & The Romanticist [v1.6.6].mid".into()
        })
        .leak();
    let model = yinhe_midi::parse_path(midi).expect("parse");
    let sr = 48_000u32;
    let ppq = model.meta.ppq as u64;
    let bar33 = 32 * 4 * ppq;
    let end_tick = bar33 + 4 * ppq; // 1 小节，全轨负载（模拟真实播放）
    let to_sample =
        |tick: u32| -> u64 { (model.tempo_map.tick_to_seconds(tick as u64) * sr as f64) as u64 };
    let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();
    for k in 0..128u8 {
        for n in model.notes[k as usize].iter() {
            if n.start_tick < bar33 as u32 || n.start_tick >= end_tick as u32 || n.velocity <= 1 {
                continue;
            }
            let ch = model.tracks[n.track as usize].global_channel();
            events.push(yinhe_synth::SynthEvent::NoteOn {
                sample: to_sample(n.start_tick),
                channel: ch,
                key: k,
                velocity: n.velocity,
                end_sample: to_sample(n.end_tick),
            });
        }
    }
    events.sort_by_key(|e| e.sample());
    eprintln!(
        "事件数={} 首事件 sample={:?}",
        events.len(),
        events.first().map(|e| e.sample())
    );

    let frames = 512usize;
    let blocks = 4700usize + 10 * 4 * (ppq as usize) * 48_000 / 1920 / 512 / 2; // 事件窗口全程（近似）
    // 事件在第 33 小节（sample 2.3M 级）：seek 到首事件前一块开始渲染
    let seek_to = events
        .first()
        .map(|e| e.sample())
        .unwrap_or(0)
        .saturating_sub(frames as u64);
    // 事件在 channel 5：必须提供 32 个通道的 buffer（否则该通道输出被丢弃）
    let mk = || {
        (0..32)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect::<Vec<_>>()
    };
    let energy = |bufs: &[yinhe_mixer::ChannelBuffers]| -> f64 {
        bufs.iter()
            .map(|b| {
                b.left
                    .iter()
                    .chain(b.right.iter())
                    .map(|v| (*v as f64).abs())
                    .sum::<f64>()
            })
            .sum()
    };

    // 事件实际用到的通道（global_channel）→ 这些 dense 槽都要加载音色库
    let chans: Vec<u32> = {
        let mut set = std::collections::BTreeSet::new();
        for e in &events {
            if let yinhe_synth::SynthEvent::NoteOn { channel, .. } = e {
                set.insert(*channel as u32);
            }
        }
        set.into_iter().collect()
    };
    eprintln!("事件通道={chans:?}");

    yinhe_synth::cpu_synth::LAYER_KILLS.store(0, std::sync::atomic::Ordering::Relaxed);
    yinhe_synth::gpu_synth::LAYER_KILLS.store(0, std::sync::atomic::Ordering::Relaxed);

    // CPU
    let mut cpu = yinhe_synth::CpuSynth::new(sr);
    cpu.load_dense_soundfonts_many(&chans, &[std::path::PathBuf::from(&sfz)])
        .expect("cpu sf");
    cpu.load_events(events.clone());
    let _ = seek_to;
    let mut cb = mk();
    let mut cpu_e = Vec::new();
    let mut cpu_voices = Vec::new();
    let mut cpu_ch5: Vec<f32> = Vec::new();
    for _ in 0..blocks {
        cpu.render_to_mixer(&mut cb);
        cpu_e.push(energy(&cb));
        cpu_voices.push(cpu.voice_count());
        cpu_ch5.extend_from_slice(&cb[5].left);
    }
    eprintln!(
        "CPU voices 峰值={:?} 末值={:?}",
        cpu_voices.iter().max(),
        cpu_voices.last()
    );
    eprintln!(
        "CPU 能量峰值={:.1} @块{:?}",
        cpu_e.iter().cloned().fold(0.0f64, f64::max),
        cpu_e
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
    );

    // GPU
    let Ok(mut gpu) = yinhe_synth::GpuSynth::new_default(sr) else {
        eprintln!("无 GPU");
        return;
    };
    gpu.load_dense_soundfonts_many(&chans, &[std::path::PathBuf::from(&sfz)])
        .expect("gpu sf");
    gpu.finish_soundfont_load();
    gpu.prewarm(frames as u32);
    gpu.load_events(events.clone());
    let mut gb = mk();
    let mut gpu_e = Vec::new();
    let mut voices = Vec::new();
    let mut max_ms = 0.0f64;
    let mut gpu_ch5: Vec<f32> = Vec::new();
    for _ in 0..blocks {
        let t = std::time::Instant::now();
        gpu.render_to_mixer(&mut gb);
        max_ms = max_ms.max(t.elapsed().as_secs_f64() * 1000.0);
        gpu_e.push(energy(&gb));
        voices.push(gpu.voice_count());
        gpu_ch5.extend_from_slice(&gb[5].left);
    }
    // 逐样本对比（通道 5 = Track 13）：块能量会被平均掉，截断/尾巴差异
    // 必须看逐样本
    let n = cpu_ch5.len().min(gpu_ch5.len());
    let mut diffs: Vec<(usize, f32, f32)> = Vec::new();
    let mut diff_count = 0usize;
    let mut first_diff = usize::MAX;
    for i in 0..n {
        let d = (cpu_ch5[i] - gpu_ch5[i]).abs();
        if d > 1e-3 {
            diff_count += 1;
            if first_diff == usize::MAX {
                first_diff = i;
            }
            if diffs.len() < 10 {
                diffs.push((i, cpu_ch5[i], gpu_ch5[i]));
            }
        }
    }
    eprintln!(
        "ch5 逐样本显著差异(|d|>1e-3)：{diff_count}/{n}（{:.3}%），首个@样本 {first_diff}（帧 {}）",
        diff_count as f64 / n.max(1) as f64 * 100.0,
        first_diff / 2
    );
    if first_diff != usize::MAX {
        let lo = first_diff.saturating_sub(6);
        let hi = (first_diff + 14).min(n);
        eprintln!("  cpu[{lo}..{hi}]={:?}", &cpu_ch5[lo..hi]);
        eprintln!("  gpu[{lo}..{hi}]={:?}", &gpu_ch5[lo..hi]);
    }
    eprintln!("GPU 单块最大耗时={max_ms:.2}ms（预算 10.67ms，块长 512/48k）");
    eprintln!(
        "layer 杀音计数：cpu={} gpu={}",
        yinhe_synth::cpu_synth::LAYER_KILLS.load(std::sync::atomic::Ordering::Relaxed),
        yinhe_synth::gpu_synth::LAYER_KILLS.load(std::sync::atomic::Ordering::Relaxed)
    );
    let low: Vec<(usize, f64, f64)> = (0..blocks)
        .filter(|&i| cpu_e[i] > 1.0 && gpu_e[i] < cpu_e[i] * 0.5)
        .map(|i| (i, cpu_e[i], gpu_e[i]))
        .take(12)
        .collect();
    eprintln!("GPU 低能量块（cpu>1 且 gpu<50%）数={}", low.len());
    eprintln!("  前 12：{low:?}");
    eprintln!(
        "voices 峰值={:?} 末值={:?}",
        voices.iter().max(),
        voices.last()
    );
    let ce: f64 = cpu_e.iter().sum();
    let ge: f64 = gpu_e.iter().sum();
    eprintln!(
        "总能量 cpu={ce:.1} gpu={ge:.1} 比值={:.3}",
        ge / ce.max(1e-9)
    );
}

mod compare_tests {
    use super::*;

    /// EnchantedLove 对比渲染（CpuSynth vs xsynth）：输出 raw f32（交错立体声）
    /// 供波形/频谱分析。用户报告连续同 key 高频音符处有 click，xsynth 无。
    #[test]
    #[ignore]
    fn render_compare_enchanted_love() {
        let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
        let midi = "/Users/jieneng/Music/MIDIs/EnchantedLove.mid";
        let sr = 48_000u32;
        let seconds = 300u64;
        let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse"));
        let layout = crate::spawn::channels_for_model(&model);
        let active: Vec<u8> = (0..16u8)
            .filter(|c| layout.is_active(*c as usize))
            .collect();
        eprintln!("active channels: {active:?}");
        // 定位"同 key 连续音符"密集段（用户报告的问题场景）
        let ppq = model.meta.ppq.max(1);
        for key in 0..128usize {
            let notes = &model.notes[key];
            if notes.len() < 10 {
                continue;
            }
            let mut best = (0usize, 0u32);
            let mut cur = 1usize;
            let mut cur_start = notes[0].start_tick;
            for w in 1..notes.len() {
                if notes[w].start_tick.saturating_sub(notes[w - 1].start_tick) < ppq / 8 {
                    cur += 1;
                    if cur > best.0 {
                        best = (cur, cur_start);
                    }
                } else {
                    cur = 1;
                    cur_start = notes[w].start_tick;
                }
            }
            if best.0 >= 20 {
                eprintln!(
                    "  key={key} 最长同键密集段={} 个音符 起于 tick={}（约 {:.2}s @120bpm）",
                    best.0,
                    best.1,
                    best.1 as f64 / ppq as f64 * 0.5
                );
            }
        }
        for (name, backend) in [("cpu", 1u8), ("xsynth", 0), ("cpu_l4", 2u8)] {
            let mut e = AudioEngine::new(sr, layout.clone());
            e.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            match backend {
                0 => {
                    let configs: Vec<(u8, Vec<String>)> =
                        active.iter().map(|c| (*c, vec![sfz.to_string()])).collect();
                    e.handle_command(AudioCommand::SetSoundFonts {
                        configs: Box::new(configs),
                    });
                }
                _ => {
                    let mut cs = yinhe_synth::CpuSynth::new(sr);
                    cs.set_layer_count(if backend == 2 { Some(4) } else { None });
                    for c in &active {
                        cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                            .expect("sf");
                    }
                    e.cpu_synth = Some(cs);
                }
            }
            e.handle_command(AudioCommand::Play { from_sample: 0 });
            let frames = 512usize;
            let mut buf = vec![0.0f32; frames * 2];
            let target = seconds * sr as u64;
            let mut out: Vec<f32> = Vec::new();
            let t0 = std::time::Instant::now();
            while (out.len() as u64 / 2) < target {
                e.render(&mut buf);
                out.extend_from_slice(&buf);
            }
            let mut bytes = Vec::with_capacity(out.len() * 4);
            for f in &out {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            let path = format!("/tmp/compare_{name}.raw");
            std::fs::write(&path, &bytes).expect("write");
            eprintln!(
                "{name}: {} frames 耗时 {:?} -> {path}",
                out.len() / 2,
                t0.elapsed()
            );
        }
    }

    /// 单音符增益对比：同 key/vel 的一个音符，CpuSynth vs xsynth 的峰值/RMS。
    /// 用于定位整体响度差异（多 voice 对比已排除 layer 因素）。
    #[test]
    #[ignore]
    fn render_single_note_gain_compare() {
        let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
        let sr = 48_000u32;
        let mut mask = vec![false; 16];
        mask[0] = true;
        let layout = crate::channel_layout::ChannelLayout::from_mask(mask);
        for (name, backend) in [("cpu", 1u8), ("xsynth", 0), ("gpu", 2u8)] {
            let mut e = AudioEngine::new(sr, layout.clone());
            match backend {
                0 => {
                    let configs: Vec<(u8, Vec<String>)> = vec![(0, vec![sfz.to_string()])];
                    e.handle_command(AudioCommand::SetSoundFonts {
                        configs: Box::new(configs),
                    });
                }
                1 => {
                    let mut cs = yinhe_synth::CpuSynth::new(sr);
                    cs.load_dense_soundfonts(0, &[std::path::PathBuf::from(sfz)])
                        .expect("sf");
                    e.cpu_synth = Some(cs);
                }
                _ => {
                    let mut gs = yinhe_synth::GpuSynth::new_default(sr).expect("gpu");
                    gs.load_dense_soundfonts(0, &[std::path::PathBuf::from(sfz)])
                        .expect("sf");
                    gs.finish_soundfont_load();
                    e.gpu_synth = Some(gs);
                    e.invalidate_gpu_events();
                }
            }
            // 直接投一个 note（不依赖模型）：CPU 用 send_event，xsynth 用 channel_set
            if backend == 2 {
                e.gpu_synth.as_mut().expect("gpu").load_events(vec![
                    yinhe_synth::SynthEvent::NoteOn {
                        sample: 0,
                        channel: 0,
                        key: 60,
                        velocity: 100,
                        end_sample: sr as u64,
                    },
                ]);
            } else if let Some(cs) = e.cpu_synth.as_mut() {
                cs.send_event(yinhe_synth::SynthEvent::NoteOn {
                    sample: 0,
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    end_sample: sr as u64,
                });
            } else {
                e.channel_set
                    .send_event(xsynth_core::channel_group::SynthEvent::Channel(
                        0,
                        xsynth_core::channel::ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: 60,
                            vel: 100,
                        }),
                    ));
            }
            e.playing = true;
            let frames = 512usize;
            let mut buf = vec![0.0f32; frames * 2];
            let mut out: Vec<f32> = Vec::new();
            for _ in 0..(sr / frames as u32) {
                e.render(&mut buf);
                out.extend_from_slice(&buf);
            }
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let rms = (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt();
            eprintln!("single_note {name}: peak={peak:.4} rms={rms:.4}");
            let mut bytes = Vec::with_capacity(out.len() * 4);
            for f in &out {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            std::fs::write(format!("/tmp/single_{name}.raw"), &bytes).expect("write");
        }
    }
}

/// 诊断：Ouranos bar157 起的**引擎路径**断续检测（64 帧窗口能量 + 骤降）。
/// 裸 GPU 基准复现不了（无静音窗/无骤降），差异疑在引擎路径（seek/chase/调度）。
/// cargo test --release -p yinhe-audio --features gpu diag_ouranos_bar157_dropout -- --ignored --nocapture
#[cfg(feature = "gpu")]
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn diag_ouranos_bar157_dropout() {
    use std::sync::Arc;

    let midi = std::env::var("YINHE_DIAG_MIDI").unwrap_or_else(|_| {
        "/Users/jieneng/Music/MIDIs/Ouranos - HDSQ & The Romanticist [v1.6.6].mid".to_string()
    });
    let sfz = std::env::var("YINHE_TEST_SFZ").unwrap_or_else(|_| {
        "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz".into()
    });
    let sr = 48_000u32;
    let frames = std::env::var("YINHE_DIAG_FRAMES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4096);
    let bar = std::env::var("YINHE_BENCH_BAR")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(157);
    let model = Arc::new(yinhe_midi::parse_path(&midi).unwrap());
    let ppq = model.meta.ppq as u64;
    let seek_tick = bar.saturating_sub(1) * 4 * ppq;
    let seek_sample = (model.tempo_map.tick_to_seconds(seek_tick) * sr as f64) as u64;

    let active = crate::spawn::channels_for_model(&model)
        .active_mask()
        .to_vec();
    let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(active));
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::clone(&model),
    });
    let events = engine.build_gpu_events(seek_sample);
    eprintln!("MIDI={midi}");
    eprintln!("bar{bar} 事件数={} seek_sample={seek_sample}", events.len());
    {
        let (mut n_on, mut n_cc, mut n_bend, mut n_other) = (0u64, 0u64, 0u64, 0u64);
        for ev in &events {
            match ev {
                yinhe_synth::SynthEvent::NoteOn { .. } => n_on += 1,
                yinhe_synth::SynthEvent::NoteOff { .. } => {}
                yinhe_synth::SynthEvent::Control { event, .. } => match event {
                    yinhe_synth::ControlEvent::Raw(..) => n_cc += 1,
                    yinhe_synth::ControlEvent::PitchBend(..) => n_bend += 1,
                    _ => n_other += 1,
                },
            }
        }
        eprintln!("事件类型：NoteOn={n_on} Raw(CC)={n_cc} PitchBend={n_bend} 其他={n_other}");
        {
            use std::collections::HashMap;
            let mut cc_hist: HashMap<u8, u64> = HashMap::new();
            for ev in &events {
                if let yinhe_synth::SynthEvent::Control {
                    event: yinhe_synth::ControlEvent::Raw(cc, _),
                    ..
                } = ev
                {
                    *cc_hist.entry(*cc).or_insert(0) += 1;
                }
            }
            let mut hist: Vec<(u8, u64)> = cc_hist.into_iter().collect();
            hist.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
            eprintln!("CC 分布（top8）：{hist:?}");
        }
    }
    {
        // 完全重复 NoteOn 统计（同 sample/channel/key/velocity/end_sample 分组）：
        // 合批上限的先验测量——若这里冗余很少，说明数据本身无重复可省。
        use std::collections::HashMap;
        let mut seen: HashMap<(u64, u8, u8, u8, u64), u32> = HashMap::new();
        let mut note_ons = 0usize;
        for ev in &events {
            if let yinhe_synth::SynthEvent::NoteOn {
                sample,
                channel,
                key,
                velocity,
                end_sample,
            } = ev
            {
                note_ons += 1;
                *seen
                    .entry((*sample, *channel, *key, *velocity, *end_sample))
                    .or_insert(0) += 1;
            }
        }
        let groups = seen.values().filter(|&&c| c > 1).count();
        let redundant: usize = seen
            .values()
            .filter(|&&c| c > 1)
            .map(|&c| (c - 1) as usize)
            .sum();
        eprintln!(
            "可合批统计：NoteOn={note_ons} 完全重复组={groups} 冗余事件={redundant}（{:.1}%）",
            redundant as f64 / note_ons.max(1) as f64 * 100.0
        );
    }

    let mut synth = yinhe_synth::GpuSynth::new_default(sr).unwrap();
    let sfz_path = std::path::PathBuf::from(&sfz);
    for ch in 0..32u32 {
        synth
            .load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
            .unwrap();
    }
    synth.finish_soundfont_load();
    // 与 CPU 对照对齐：不限 per-key layer（默认 Some(4) 会杀弱音，导致
    // GPU voice 数远小于 CPU、对照不公平）
    synth.set_layer_count(None);
    synth.load_events(events);
    synth.seek(seek_sample);
    engine.gpu_synth = Some(synth);
    engine.playing = true;
    engine.sample_position = seek_sample;

    let mut out = vec![0.0f32; frames * 2];
    let mut energy: Vec<f32> = Vec::new();
    let mut wav_all: Vec<f32> = Vec::new();
    // 与实时输出一致：前瞻限幅（audio_renderer 同款），否则诊断 WAV 有削波假象。
    let mut limiter = yinhe_dsp::dsp::limiter::VolumeLimiter::new(sr);
    let mut raw_clip: usize = 0;
    let mut raw_all: Vec<f32> = Vec::new();
    let blocks = std::env::var("YINHE_DIAG_BLOCKS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(60);
    let w = 64usize;
    let mut ring_short_sum: u64 = 0;
    let mut ring_short_blocks: u64 = 0;
    for b in 0..blocks {
        engine.render(&mut out);
        if let Some(g) = engine.gpu_synth.as_ref() {
            let rs = g.diag_ring_short as u64;
            if rs > 0 {
                ring_short_sum += rs;
                ring_short_blocks += 1;
            }
        }
        if b % 30 == 0 {
            let vc = engine
                .gpu_synth
                .as_ref()
                .map(|g| g.voice_count())
                .unwrap_or(0);
            eprintln!("voice 数 第{b}块：GPU={vc}");
        }
        raw_clip += out.iter().filter(|v| v.abs() > 1.0).count();
        raw_all.extend_from_slice(&out);
        limiter.limit(&mut out);
        wav_all.extend_from_slice(&out);
        let be = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let mix_peak = engine
            .mixer
            .buffers_mut()
            .iter()
            .map(|b| {
                b.left
                    .iter()
                    .chain(b.right.iter())
                    .fold(0.0f32, |m, v| m.max(v.abs()))
            })
            .fold(0.0f32, f32::max);
        if let Some(gs) = engine.gpu_synth.as_ref() {
            eprintln!(
                "  块{b}: mixer={mix_peak:.4} out={be:.4} voices={} gpu_mix={:.4} 缺帧={} ms(采/提/收/环/出)={:?}",
                gs.voice_count(),
                gs.diag_gpu_mix_peak,
                gs.diag_ring_short,
                gs.diag_ms[..5]
                    .iter()
                    .map(|v| (v * 10.0).round() / 10.0)
                    .collect::<Vec<_>>()
            );
        }
        let n = out.len() / 2 / w;
        for i in 0..n {
            let mut e = 0.0f32;
            for j in (i * w)..((i + 1) * w) {
                e += out[j * 2].abs() + out[j * 2 + 1].abs();
            }
            energy.push(e);
        }
    }
    let peak = energy.iter().fold(0.0f32, |m, v| m.max(*v));
    let floor = peak * 0.01;
    let quiet: Vec<usize> = energy
        .iter()
        .enumerate()
        .filter(|(_, e)| **e < floor)
        .map(|(i, _)| i)
        .collect();
    let mut drops: Vec<(usize, f32, f32)> = Vec::new();
    for i in 1..energy.len() {
        if energy[i - 1] > peak * 0.08 && energy[i] < energy[i - 1] * 0.15 {
            drops.push((i, energy[i - 1], energy[i]));
        }
    }
    eprintln!(
        "引擎路径 bar{bar}：峰值={peak:.3} 静音窗={} 骤降={}",
        quiet.len(),
        drops.len()
    );
    eprintln!("  静音窗前20={:?}", &quiet[..quiet.len().min(20)]);
    eprintln!("  骤降前12={:?}", drops.iter().take(12).collect::<Vec<_>>());

    // —— 单音符幅度对比（排除叠加：同事件、同 seek，各渲染 8 块）——
    {
        let ev1 = engine.build_gpu_events(seek_sample);
        if let Some(n) = ev1
            .iter()
            .find(|e| matches!(e, yinhe_synth::SynthEvent::NoteOn { .. }))
        {
            let note_sample = n.sample();
            let single = vec![*n];
            let mut g1 = yinhe_synth::GpuSynth::new_default(sr).unwrap();
            for ch in 0..32u32 {
                g1.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                    .unwrap();
            }
            g1.finish_soundfont_load();
            g1.set_layer_count(None);
            g1.load_events(single.clone());
            g1.seek(note_sample);
            let mut gb: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
                .map(|_| yinhe_mixer::ChannelBuffers {
                    left: vec![0.0; frames],
                    right: vec![0.0; frames],
                })
                .collect();
            let mut c1 = yinhe_synth::CpuSynth::new(sr);
            for ch in 0..32u32 {
                c1.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                    .unwrap();
            }
            c1.finish_soundfont_load();
            c1.set_layer_count(None);
            c1.seek(note_sample);
            c1.load_events(single);
            c1.set_sample_position(note_sample);
            let mut cb: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
                .map(|_| yinhe_mixer::ChannelBuffers {
                    left: vec![0.0; frames],
                    right: vec![0.0; frames],
                })
                .collect();
            let (mut gp, mut cp) = (0.0f32, 0.0f32);
            for _ in 0..8 {
                g1.render_to_mixer(&mut gb);
                c1.render_to_mixer(&mut cb);
                for i in 0..frames {
                    for b in &gb {
                        gp = gp.max(b.left[i].abs().max(b.right[i].abs()));
                    }
                    for b in &cb {
                        cp = cp.max(b.left[i].abs().max(b.right[i].abs()));
                    }
                }
            }
            let kv = match n {
                yinhe_synth::SynthEvent::NoteOn { key, velocity, .. } => {
                    format!("{key}/{velocity}")
                }
                _ => "?".into(),
            };
            eprintln!(
                "单音符对比（{kv}）：GPU 峰值={gp:.5} voices={}  CPU 峰值={cp:.5} voices={}  比值={:.2}",
                g1.voice_count(),
                c1.voice_count(),
                gp / cp.max(1e-9)
            );
        }
    }

    // —— 单音符长漂移：人工长音（gate 11s）连续渲染 120 块 ——
    // 隔离"时间/速度累积"与"音符/CC 交互"：导出 WAV 供离线测相位漂移。
    {
        // 多音符复现：1200 个 gate 0.5s 的音符（每 100ms 一个，跨 16 通道），
        // 触发大量 voice 创建/结束/槽位回收复用——校准累积差异源（单音符无差异）。
        // 单音符定位实验：sample=4000 落在块 0 的段 7（段 512 帧）内。
        // 若"段内帧→块内帧"换算有误，GPU 起音会延迟整段数（>=512 帧）。
        let single_long: Vec<yinhe_synth::SynthEvent> = vec![yinhe_synth::SynthEvent::NoteOn {
            sample: 4000,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4000 + (sr as u64 * 30) / 1000,
        }];
        let mut g3 = yinhe_synth::GpuSynth::new_default(sr).unwrap();
        for ch in 0..32u32 {
            g3.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                .unwrap();
        }
        g3.finish_soundfont_load();
        g3.set_layer_count(None);
        g3.load_events(single_long.clone());
        g3.seek(0);
        let mut c3 = yinhe_synth::CpuSynth::new(sr);
        for ch in 0..32u32 {
            c3.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                .unwrap();
        }
        c3.finish_soundfont_load();
        c3.set_layer_count(None);
        c3.seek(0);
        c3.load_events(single_long);
        g3.seek(0);
        c3.set_sample_position(0);
        let f3 = 4096usize;
        let b3 = 30usize;
        let mut gb3: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; f3],
                right: vec![0.0; f3],
            })
            .collect();
        let mut cb3: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; f3],
                right: vec![0.0; f3],
            })
            .collect();
        let mut g3_all: Vec<f32> = Vec::new();
        let mut c3_all: Vec<f32> = Vec::new();
        for _ in 0..b3 {
            g3.render_to_mixer(&mut gb3);
            c3.render_to_mixer(&mut cb3);
            for i in 0..f3 {
                g3_all.push(gb3[0].left[i]);
                c3_all.push(cb3[0].left[i]);
            }
        }
        let write_single = |path: &std::path::Path, data: &[f32]| {
            if let Ok(mut w) = hound::WavWriter::create(
                path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: sr,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            ) {
                for &v in data {
                    let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
                let _ = w.finalize();
            }
        };
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        write_single(
            std::path::Path::new(&format!("{home}/Desktop/single_gpu.wav")),
            &g3_all,
        );
        write_single(
            std::path::Path::new(&format!("{home}/Desktop/single_cpu.wav")),
            &c3_all,
        );
        eprintln!(
            "单音符长渲染 WAV 已导出（120 块，{:.1}s）",
            g3_all.len() as f64 / sr as f64
        );
    }

    // —— 子窗口隔离：1.5-2.0s（大 WAV 中 GPU/CPU 波形去相关区）单独重渲染 ——
    // 判据：若子窗口相关也低 -> 内容差异（好定位）；若高 -> 长时间渲染累积问题。
    {
        let w_start = seek_sample + (1.5 * sr as f64) as u64;
        let w_end = w_start + sr as u64 / 2;
        let sub: Vec<yinhe_synth::SynthEvent> = engine
            .build_gpu_events(w_start)
            .into_iter()
            .filter(|e| e.sample() >= w_start && e.sample() < w_end)
            .collect();
        let n_sub = sub.len();
        let mut g2 = yinhe_synth::GpuSynth::new_default(sr).unwrap();
        for ch in 0..32u32 {
            g2.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                .unwrap();
        }
        g2.finish_soundfont_load();
        g2.set_layer_count(None);
        g2.load_events(sub.clone());
        g2.seek(w_start);
        let mut c2 = yinhe_synth::CpuSynth::new(sr);
        for ch in 0..32u32 {
            c2.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                .unwrap();
        }
        c2.finish_soundfont_load();
        c2.set_layer_count(None);
        c2.seek(w_start);
        c2.load_events(sub);
        c2.set_sample_position(w_start);
        let mut gb2: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        let mut cb2: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        let blocks2 = 6usize;
        let mut ga: Vec<f32> = Vec::new();
        let mut ca: Vec<f32> = Vec::new();
        let mut gv_peak = 0usize;
        let mut cv_peak = 0usize;
        for _ in 0..blocks2 {
            g2.render_to_mixer(&mut gb2);
            c2.render_to_mixer(&mut cb2);
            gv_peak = gv_peak.max(g2.voice_count());
            cv_peak = cv_peak.max(c2.voice_count());
            for i in 0..frames {
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for b in &gb2 {
                    l += b.left[i];
                    r += b.right[i];
                }
                ga.push(l);
                ga.push(r);
                let (mut l2, mut r2) = (0.0f32, 0.0f32);
                for b in &cb2 {
                    l2 += b.left[i];
                    r2 += b.right[i];
                }
                ca.push(l2);
                ca.push(r2);
            }
        }
        let m = ga.len().min(ca.len());
        let ag: Vec<f32> = ga[..m].iter().step_by(2).copied().collect();
        let ac: Vec<f32> = ca[..m].iter().step_by(2).copied().collect();
        let (mg, mc) = (
            ag.iter().sum::<f32>() / ag.len() as f32,
            ac.iter().sum::<f32>() / ac.len() as f32,
        );
        let mut num = 0.0f64;
        let mut dg = 0.0f64;
        let mut dc = 0.0f64;
        for (a, b) in ag.iter().zip(ac.iter()) {
            let (a, b) = ((a - mg) as f64, (b - mc) as f64);
            num += a * b;
            dg += a * a;
            dc += b * b;
        }
        let corr = num / (dg.sqrt() * dc.sqrt()).max(1e-12);
        let egr = ag.iter().map(|v| v * v).sum::<f32>().sqrt();
        let ecr = ac.iter().map(|v| v * v).sum::<f32>().sqrt();
        eprintln!(
            "子窗口 1.5-2.0s（事件 {n_sub}）：相关={corr:.3} 能量比 G/C={:.3} 峰值voice GPU={gv_peak} CPU={cv_peak}",
            egr / ecr.max(1e-9)
        );
    }

    // —— CPU 对照（yinhe CpuSynth，同一事件表/seek）：对比"音符是否被削短" ——
    {
        let events3 = engine.build_gpu_events(seek_sample);
        let mut cpu = yinhe_synth::CpuSynth::new(sr);
        // 与 GPU 对齐：16 个 dense 通道全部加载（此前只加载通道 0，
        // 导致 CPU 只渲染 1/16 内容，对照不公平且听感认不出）
        for ch in 0..32u32 {
            cpu.load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
                .unwrap();
        }
        cpu.finish_soundfont_load();
        cpu.set_layer_count(None);
        // 注意顺序：seek 清空事件表（与 GPU 的 seek 语义不同），load_events 又把
        // 位置重置为 0；故 seek（清状态）→ load_events（灌事件）→ set 位置。
        cpu.seek(seek_sample);
        cpu.load_events(events3);
        cpu.set_sample_position(seek_sample);
        let mut cbufs: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        let mut cpu_all: Vec<f32> = Vec::new();
        // 与实时一致：CPU 对照也经前瞻限幅（否则 WAV 不可听、且对照口径不同）
        let mut cpu_limiter = yinhe_dsp::dsp::limiter::VolumeLimiter::new(sr);
        let mut cpu_raw_clip: usize = 0;
        let mut cpu_frame = vec![0.0f32; frames * 2];
        let mut cpu_raw_all: Vec<f32> = Vec::new();
        for bi in 0..blocks.min(120) {
            cpu.render_to_mixer(&mut cbufs);
            if bi % 30 == 0 {
                eprintln!(
                    "voice 数 第{bi}块：CPU未结束={} CPU槽位={} 位置={}",
                    cpu.voice_count(),
                    cpu.debug_voices_len(),
                    cpu.sample_position()
                );
            }
            for i in 0..frames {
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for b in &cbufs {
                    l += b.left[i];
                    r += b.right[i];
                }
                cpu_frame[i * 2] = l;
                cpu_frame[i * 2 + 1] = r;
            }
            cpu_raw_clip += cpu_frame.iter().filter(|v| v.abs() > 1.0).count();
            cpu_raw_all.extend_from_slice(&cpu_frame);
            cpu_limiter.limit(&mut cpu_frame);
            cpu_all.extend_from_slice(&cpu_frame);
        }
        eprintln!(
            "CPU 限幅前削波={cpu_raw_clip}/{}（{:.1}%）",
            cpu_raw_all.len(),
            cpu_raw_clip as f64 / cpu_raw_all.len().max(1) as f64 * 100.0
        );
        // 逐窗口能量对比（GPU 用 wav_all 前 120 块的同口径）
        let w = 64usize;
        let gpu_take = (blocks.min(120) * frames * 2).min(wav_all.len());
        let cpu_take = cpu_all.len();
        let n_win = (gpu_take / 2 / w).min(cpu_take / 2 / w);
        let mut g_peak = 0.0f32;
        let mut c_peak = 0.0f32;
        let mut g_eq = 0usize;
        let mut c_eq = 0usize;
        let mut ratio_sum = 0.0f64;
        let mut ratio_n = 0usize;
        for i in 0..n_win {
            let mut ge = 0.0f32;
            let mut ce = 0.0f32;
            for j in (i * w)..((i + 1) * w) {
                ge += wav_all[j * 2].abs() + wav_all[j * 2 + 1].abs();
                ce += cpu_all[j * 2].abs() + cpu_all[j * 2 + 1].abs();
            }
            g_peak = g_peak.max(ge);
            c_peak = c_peak.max(ce);
            if ge > g_peak * 0.0 && ge > 1e-6 {
                g_eq += 1;
            }
            if ce > 1e-6 {
                c_eq += 1;
            }
            if ce > 1e-6 {
                ratio_sum += (ge as f64) / (ce as f64);
                ratio_n += 1;
            }
        }
        // 削波统计（|v|>1.0 的比例；引擎最终输出应 <=1.0 基本不削）
        let g_clip = wav_all.iter().filter(|v| v.abs() > 1.0).count();
        let c_clip = cpu_all.iter().filter(|v| v.abs() > 1.0).count();
        eprintln!("ring 不足：块数={ring_short_blocks}/{blocks} 总缺帧={ring_short_sum}");
        {
            // 限幅前 GPU/CPU 直接相关（区分"渲染层累积差异"与"限幅器差异"）
            let m = raw_all.len().min(cpu_raw_all.len());
            let ag: Vec<f32> = raw_all[..m].iter().step_by(2).copied().collect();
            let ac: Vec<f32> = cpu_raw_all[..m].iter().step_by(2).copied().collect();
            let (mg, mc) = (
                ag.iter().sum::<f32>() / ag.len() as f32,
                ac.iter().sum::<f32>() / ac.len() as f32,
            );
            let (mut num, mut dg, mut dc) = (0.0f64, 0.0f64, 0.0f64);
            for (a, b) in ag.iter().zip(ac.iter()) {
                let (a, b) = ((a - mg) as f64, (b - mc) as f64);
                num += a * b;
                dg += a * a;
                dc += b * b;
            }
            let corr = num / (dg.sqrt() * dc.sqrt()).max(1e-12);
            eprintln!("限幅前相关 GPU/CPU（全段）={corr:.4}");
        }
        eprintln!(
            "限幅前削波 GPU={raw_clip}/{}（{:.2}%）",
            wav_all.len(),
            raw_clip as f64 / wav_all.len().max(1) as f64 * 100.0
        );
        eprintln!(
            "CPU 对照：窗口数={n_win} 非静音窗 GPU={g_eq} CPU={c_eq} 峰值 GPU={g_peak:.1} CPU={c_peak:.1} 能量比均值={:.3}\n  削波样本 GPU={g_clip}/{}（{:.1}%） CPU={c_clip}/{}（{:.1}%）",
            ratio_sum / ratio_n.max(1) as f64,
            wav_all.len(),
            g_clip as f64 / wav_all.len().max(1) as f64 * 100.0,
            cpu_all.len(),
            c_clip as f64 / cpu_all.len().max(1) as f64 * 100.0
        );
        let cpu_wav = format!("/Users/jieneng/Desktop/yinhe_bar{bar}_cpu.wav");
        if let Ok(mut wtr) = hound::WavWriter::create(
            &cpu_wav,
            hound::WavSpec {
                channels: 2,
                sample_rate: sr,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        ) {
            for &v in &cpu_all {
                let _ = wtr.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
            let _ = wtr.finalize();
            eprintln!("CPU WAV：{cpu_wav}");
        }
    }

    // 导出本段为 WAV（供人耳确认"断续"是否存在于引擎路径输出）
    let wav_path = std::env::var("YINHE_DIAG_WAV")
        .unwrap_or_else(|_| format!("/Users/jieneng/Desktop/yinhe_bar{bar}.wav"));
    match hound::WavWriter::create(
        &wav_path,
        hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    ) {
        Ok(mut w) => {
            for &v in &wav_all {
                let s = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                let _ = w.write_sample(s);
            }
            let _ = w.finalize();
            eprintln!(
                "WAV 已导出：{wav_path}（{:.1}s）合批命中={}",
                wav_all.len() as f64 / 2.0 / sr as f64,
                yinhe_synth::gpu_synth::BATCH_HITS.load(std::sync::atomic::Ordering::Relaxed)
            );
        }
        Err(e) => eprintln!("WAV 导出失败：{e}"),
    }

    // —— 对照：同一事件/seek 的裸 GpuSynth 路径，逐样本找差异起点 ——
    let mut synth2 = yinhe_synth::GpuSynth::new_default(sr).unwrap();
    for ch in 0..32u32 {
        synth2
            .load_dense_soundfonts(ch, std::slice::from_ref(&sfz_path))
            .unwrap();
    }
    synth2.finish_soundfont_load();
    // 注：事件已在前面 move 给引擎 synth，这里重取一份
    let events2 = engine.build_gpu_events(seek_sample);
    synth2.load_events(events2);
    synth2.seek(seek_sample);
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..16)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();
    let mut bare_all: Vec<f32> = Vec::new();
    for _ in 0..blocks {
        synth2.render_to_mixer(&mut bufs);
        for i in 0..frames {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for b in &bufs {
                l += b.left[i];
                r += b.right[i];
            }
            bare_all.push(l);
            bare_all.push(r);
        }
    }
    let m = bare_all.len().min(energy.len() * w);
    let _ = m;
    // energy 是每窗口绝对值和；裸路径同口径
    let mut bare_energy: Vec<f32> = Vec::new();
    let nw = bare_all.len() / 2 / w;
    for i in 0..nw {
        let mut e = 0.0f32;
        for j in (i * w)..((i + 1) * w) {
            e += bare_all[j * 2].abs() + bare_all[j * 2 + 1].abs();
        }
        bare_energy.push(e);
    }
    let bpeak = bare_energy.iter().fold(0.0f32, |m, v| m.max(*v));
    let bquiet: Vec<usize> = bare_energy
        .iter()
        .enumerate()
        .filter(|(_, e)| **e < bpeak * 0.01)
        .map(|(i, _)| i)
        .collect();
    eprintln!(
        "裸路径 bar{bar}：峰值={bpeak:.3} 静音窗={}（窗口数={}）",
        bquiet.len(),
        bare_energy.len()
    );
    // 首差异窗口（引擎 vs 裸，能量口径）
    let mut first = None;
    for i in 0..bare_energy.len().min(energy.len()) {
        if (energy[i] - bare_energy[i]).abs() > bpeak * 0.05 {
            first = Some(i);
            break;
        }
    }
    eprintln!(
        "  首差异窗={:?}（帧≈{}）",
        first,
        first.map(|i| i * w).unwrap_or(0)
    );
}
