use super::*;
use std::collections::BTreeMap;
use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};
use xsynth_core::channel_group::ParallelismOptions;
use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_editor_core::document::Document;
use yinhe_types::automation::{ParamDevice, xsynth_param};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, KEY_COUNT, SegmentShape};

use crate::audio_model::AudioEvent;
use crate::channel_layout::ChannelLayout;

fn make_model_with_notes(notes: Vec<(u8, u32, u32, u8, u8)>) -> YinModel {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
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

/// 每轨电平：渲染后发声轨道有峰值；停止播放发布 0。
#[test]
#[ignore = "需要 SoundFont"]
fn track_level_meter_reads_synth_peak_per_track() {
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = make_model_with_notes(vec![(60, 0, 4_800, 100, 0)]);
    let layout = crate::spawn::channels_for_model(&model);
    let mut engine = AudioEngine::new(sr, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::new(model),
    });
    let mut synth = yinhe_synth::CpuSynth::new(sr);
    synth
        .load_dense_soundfonts(0, &[std::path::PathBuf::from(sfz)])
        .expect("音色库加载");
    engine.cpu_synth = Some(synth);
    engine.handle_command(AudioCommand::Play { from_sample: 0 });

    let mut out = vec![0.0f32; 512 * 2];
    for _ in 0..10 {
        engine.render(&mut out);
    }
    engine.publish_track_levels();
    let levels = engine.track_levels.lock().unwrap().clone();
    assert_eq!(levels.len(), 1, "一轨一个电平表槽");
    assert!(levels[0] > 0.0, "发声轨道应有电平（{}）", levels[0]);

    // 停止播放 → 全部 0。
    engine.playing = false;
    engine.publish_track_levels();
    assert!(
        engine
            .track_levels
            .lock()
            .unwrap()
            .iter()
            .all(|&v| v == 0.0)
    );
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
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 80))),
        },
        SortedCC {
            tick: 50,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 100))),
        },
        SortedCC {
            tick: 200,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 60))),
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

    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.active_notes.len(), 1);
    assert_eq!(engine.sample_position(), 0);
}

#[test]
fn transport_loop_wraps_at_exact_sample() {
    let model = Arc::new(make_model_with_notes(vec![]));
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.playing = true;

    // 120 BPM、PPQ 480：1 tick = 50 samples；480 tick = 24000 samples。
    engine.set_loop(true, 0, 480);
    assert!(engine.loop_enabled);

    let mut out = vec![0.0f32; 512 * 2];
    // 46 块 = 23552 samples，尚未到 loop end。
    for _ in 0..46 {
        engine.render(&mut out);
    }
    assert_eq!(engine.sample_position(), 23552);

    // 第 47 块：先渲染 448 帧到 24000，再回绕到 0 渲染剩余 64 帧。
    engine.render(&mut out);
    assert_eq!(engine.sample_position(), 64);
    assert_eq!(engine.current_tick(), engine.sample_to_tick(64));
    // 回绕偏移 = 一圈长度：cpal 回调据此把绝对消费位置换算回逻辑位置。
    assert_eq!(engine.loop_wrap_offset, 24000);

    // 再跑一圈：偏移继续累加，绝对位置保持单调。
    for _ in 0..47 {
        engine.render(&mut out);
    }
    assert_eq!(engine.sample_position(), 128);
    assert_eq!(engine.loop_wrap_offset, 48000);
}

#[test]
fn invalid_loop_range_disables_loop() {
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.set_loop(true, 480, 480);
    assert!(!engine.loop_enabled);
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
                id: 0,
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

    assert_eq!(engine.note_cursor[60].idx, 0);
    // Note at key 60, start_tick=960, velocity=100 → should dispatch at tick 960
    //（44100 Hz：1 tick = 45.94 sample，960 tick = 44100 sample）。
    let next = engine.dispatch_and_find_next(960, 1306);
    // Next note (other track) starts at tick1440 = 66150 sample > block_end, so no next event.
    assert_eq!(next, None);
    // key 60 桶有 3 个音符：vel 0/1 在 dispatch 时被跳过，vel=100 已派发。
    assert_eq!(engine.note_cursor[60].idx, 3);
    assert_eq!(engine.active_notes.len(), 1);
    for key in 0..128usize {
        if key != 60 {
            assert_eq!(engine.note_cursor[key].idx, 0);
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
    // 力度 ≤1 的音符在 dispatch 时被跳过（游标越过它们），不产生事件。
    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.note_cursor[61].idx, 1);
}

#[test]
fn test_audible_index_uses_per_key_tempo_cursor() {
    let conductor = ConductorData {
        tempo: AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
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
    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(960, 2500);
    // 处理 NoteOff 960 后，下一个事件是 key 0 的 NoteOn at 2000。
    assert_eq!(next, Some(2000));
    // key 60 ended, so only key 0 is active.
    assert_eq!(engine.active_notes.len(), 0);

    let next = engine.dispatch_and_find_next(2000, 2500);
    // key 0 NoteOn 后，NoteOff at end_tick=2480。
    assert_eq!(next, Some(2480));
    assert_eq!(engine.note_cursor[0].idx, 1);
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
                id: 0,
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
                    id: 0,
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
                id: 0,
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
                id: 0,
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
                id: 0,
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
    assert_eq!(engine.note_cursor[60].idx, 1);
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
    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.active_notes.len(), 1);
}

/// 增量 UpdateNotes 回归：换模型引用 + 按当前 tick 重定位游标后，
/// 已派发的音符不会被重复触发，新音符正常进入 dispatch。
#[test]
fn test_notes_update_repositions_cursors_without_replaying() {
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

    // 播放推进一块：两个音符 NoteOn，游标越过它们
    let mut out = vec![0.0f32; 1024];
    engine.render(&mut out);
    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.note_cursor[64].idx, 1);

    // 编辑：key 60 桶加一个更晚的音符
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

    let (audio_model, yin_model, duration) =
        crate::prepare_model::prepare_notes(&doc.data.model, sample_rate);
    engine.apply_notes_only(audio_model, yin_model, duration);

    // 两个桶都按当前 tick 重定位：已派发的 0..480 音符不会被重复派发
    assert_eq!(engine.note_cursor[64].idx, 1, "干净桶游标越过已派发音符");
    assert_eq!(engine.note_cursor[60].idx, 1, "新音符排在已派发音符之后");

    // 继续 dispatch：480 = 两个 NoteOff，960 = 新音符的 NoteOn
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
    assert_eq!(engine.note_cursor[60].idx, 1);
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
    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.note_cursor[64].idx, 0);
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
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 40))),
        },
        SortedCC {
            tick: 0,
            channel: 0,
            track: 1,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 100))),
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
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 40))),
        },
        SortedCC {
            tick: 300,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(1, 80))),
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
            id: 0,
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
            id: 0,
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
                id: 0,
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
                    id: 0,
                    tick: 0,
                    value: 2.0 / 127.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
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
                    id: 0,
                    tick: 336,
                    value: 8192.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
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
                    id: 0,
                    tick: 192,
                    value: 100.0 / 127.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
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
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 100))),
        },
        // ch1 的 CC7=50（不应影响 ch0）
        SortedCC {
            tick: 20,
            channel: 1,
            track: 1,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 50))),
        },
        // ch0 的 CC10=80（pan）
        SortedCC {
            tick: 30,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(10, 80))),
        },
        // ch0 的 CC7=90（最新）
        SortedCC {
            tick: 40,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 90))),
        },
        // ch0 的 PBS=48（RPN0，非 DSP CC，走常规 apply）
        SortedCC {
            tick: 45,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Rpn {
                parameter: 0,
                value: 48.0 / 127.0,
            },
        },
        // 边界：tick == target 参与（预览无 dispatch 兜底，Bug 8 回归）
        SortedCC {
            tick: 50,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 10))),
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
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 0))),
        },
        SortedCC {
            tick: 1920,
            channel: 0,
            track: 0,
            lane: 0,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 127))),
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
                    id: 0,
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
            engine.note_cursor[key].idx as usize,
            engine
                .yin_model
                .as_ref()
                .map(|m| m.notes[key].len())
                .unwrap_or(0),
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
    assert_eq!(engine.note_cursor[60].idx, 1);
    assert_eq!(engine.note_cursor[64].idx, 1);
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

/// dispatch 直接读模型桶，依赖桶内 start_tick 升序：乱序插入 + rebuild 后
/// 模型桶必须已排序（游标单调前进的前提）。
#[test]
fn test_model_buckets_sorted_for_dispatch() {
    let model = make_model_with_tempo(
        vec![(0, 120.0)],
        vec![(60, 960, 1440), (60, 0, 480), (60, 480, 960)], // 乱序插入
    );
    for key in 0..128usize {
        let bucket = &model.notes[key];
        let mut prev: Option<u32> = None;
        for n in bucket.iter() {
            if let Some(p) = prev {
                assert!(p <= n.start_tick, "key {key} 桶必须按 start_tick 升序");
            }
            prev = Some(n.start_tick);
        }
    }
    let starts: Vec<u32> = model.notes[60].iter().map(|n| n.start_tick).collect();
    assert_eq!(starts, vec![0, 480, 960]);
}

/// 力度阈值在 dispatch 时生效：阈值 1 只派发 vel>1 的音符。
#[test]
fn test_dispatch_filters_velocity_threshold() {
    let model = Arc::new(make_model_with_notes(vec![
        (60, 0, 480, 1, 0),
        (60, 480, 960, 2, 0),
        (60, 960, 1440, 3, 0),
    ]));
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.playing = true;
    engine.ignore_velocity = 1;

    // tick 0：vel=1 被过滤 → 没有 NoteOn，下一个边界是 480
    let next = engine.dispatch_and_find_next(0, 2000);
    assert_eq!(next, Some(480), "vel=1 不派发，边界跳到下一个音符");
    assert_eq!(engine.active_notes.len(), 0);

    // tick 480：vel=2 派发
    let next = engine.dispatch_and_find_next(480, 2000);
    assert_eq!(next, Some(960));
    assert_eq!(engine.active_notes.len(), 1);

    // 阈值改为 0 → vel=1 的音符重新可听（不影响已经过去的 0 tick，
    // 但说明阈值只是 dispatch 期的判断，无缓存可失效）
    engine.ignore_velocity = 0;
    let next = engine.dispatch_and_find_next(960, 2000);
    assert_eq!(next, Some(1440));
    // vel=2 在 960 结束（NoteOff），vel=3 同时 NoteOn → 活跃数仍为 1。
    assert_eq!(engine.active_notes.len(), 1, "vel=3 已派发、vel=2 已结束");
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
                id: 0,
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
                id: 0,
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
                id: 0,
                tick: 0,
                value: 100.0 / 127.0,
                shape: SegmentShape::Step,
            },
            AutomationEvent {
                id: 0,
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
/// 诊断：cyber-night 的 GPU vs xsynth 渲染对比（分通道隔离）。
/// 用户反馈：mute 掉 "NOTE 4"（MIDI ch3，16116 音符 / 50806 pitch bend）后正常。
/// 诊断：ch3 前 1 秒波形的 GPU vs xsynth 逐样本对比（定位首个差异点）。
/// 诊断：隔离渲染 ch3 高音刮奏（key 107-127 同时按/放），对比 GPU 与 xsynth。
/// 诊断：单音符 + 密集 pitch bend（每 100 samples 一个）GPU vs xsynth 对比。
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
                id: 0,
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
                id: 0,
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

/// 画像（本地 MIDI + SoundFont，ignored）：经典黑乐谱成本分解。
///
/// - `tau2.5.9.mid`：628 万音符，经典布局（音符为主）
/// - `cyber-night.mid`：CC 密集（控制器多的黑乐谱）
///
/// 每首曲子：三后端 baseline + CpuSynth 降级分解（全功能/无滤波/无采样/
/// 仅包络）——差值即各阶段成本占比，用于决定块级重写的收益点。
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn prof_blackmidi_cost_breakdown() {
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    let sfz = std::env::var("YINHE_PROF_SF").unwrap_or_else(|_| {
        "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz".into()
    });
    let sfz = sfz.as_str();
    let sr = 48_000u32;
    // 渲染秒数可配（`YINHE_PROF_SECS`，默认 3）：峰值段实时倍率低时
    // 用 1 秒快速迭代（AGENTS 五-7 单项测试 ≤120s）。
    let prof_secs: f64 = std::env::var("YINHE_PROF_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);

    let midis: Vec<String> = std::env::var("YINHE_PROF_MIDIS")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_else(|_| {
            vec![
                "/Users/jieneng/Music/MIDIs/tau2.5.9.mid".into(),
                "/Users/jieneng/Music/MIDIs/cyber-night.mid".into(),
            ]
        });
    for midi in &midis {
        let t_parse = Instant::now();
        let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse"));
        let total: usize = (0..256).map(|k| model.notes[k].len()).sum();
        let ppq = model.meta.ppq;
        // 密度峰值（每拍音符数）
        let mut per_beat: std::collections::BTreeMap<u32, usize> = Default::default();
        for k in 0..256 {
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
        // 多 port 工程：源通道最多 256 个，dense 索引与源通道不是一回事。
        let active: Vec<u8> = (0..256u16)
            .filter(|c| layout.is_active(*c as usize))
            .map(|c| c as u8)
            .collect();
        let denses: Vec<u32> = (0..layout.num_channels()).collect();
        let peak_tick = peak_beat.saturating_mul(ppq).saturating_sub(ppq * 2);
        {
            // 峰值拍的门长：黑乐谱大量极短音符，决定"同时发声数"的上限。
            let lo = peak_beat * ppq;
            let hi = lo + ppq;
            let (mut n, mut sum) = (0u64, 0u64);
            for k in 0..256 {
                for note in model.notes[k].iter() {
                    if note.start_tick >= lo && note.start_tick < hi {
                        n += 1;
                        sum += note.end_tick.saturating_sub(note.start_tick) as u64;
                    }
                }
            }
            eprintln!(
                "  峰值拍门长：note_on={n} 平均 gate={:.1} tick",
                sum as f64 / n.max(1) as f64
            );
        }
        // tick→sample 必须用**加载模型后**的引擎（tempo map 来自模型；
        // 空引擎会退化成默认 120BPM，seek 到错误位置）。
        let start_sample = {
            let mut probe = AudioEngine::new(sr, layout.clone());
            probe.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            probe.tick_to_sample(peak_tick)
        };

        {
            let beat_secs = model
                .tempo_map
                .tick_to_seconds((peak_beat + 1) as u64 * ppq as u64)
                - model
                    .tempo_map
                    .tick_to_seconds(peak_beat as u64 * ppq as u64);
            let probe = {
                let mut p = AudioEngine::new(sr, layout.clone());
                p.handle_command(AudioCommand::LoadModel {
                    model: Arc::clone(&model),
                });
                p
            };
            let start_tick = probe.sample_to_tick(start_sample);
            let end_tick = probe.sample_to_tick(start_sample + (prof_secs * sr as f64) as u64);
            let mut count = 0usize;
            for key in 0..256 {
                count += model.notes[key].range(start_tick, end_tick).count();
            }
            eprintln!(
                "  峰值拍时长={beat_secs:.3}s bpm≈{:.0} 渲染窗口=[{start_tick},{end_tick}) 音符数={count}",
                60.0 / beat_secs.max(1e-9)
            );
        }

        // 渲染 prof_secs 秒（调用方负责 Play/重置）
        let render_secs = |engine: &mut AudioEngine, secs: f64| -> (f64, u64, f64, f32) {
            let frames = 512usize;
            let mut buf = vec![0.0f32; frames * 2];
            let target: u64 = (secs * sr as f64) as u64;
            let mut rendered = 0u64;
            let mut peak_voice = 0u64;
            let mut peak_sample = 0.0f32;
            let mut max_chunk = 0.0f64;
            let t0 = Instant::now();
            while rendered < target {
                let tc = Instant::now();
                engine.render(&mut buf);
                let dt = tc.elapsed().as_secs_f64() * 1000.0;
                rendered += frames as u64;
                peak_voice = peak_voice.max(engine.voice_count());
                peak_sample = buf.iter().fold(peak_sample, |m, s| m.max(s.abs()));
                if rendered > sr as u64 {
                    max_chunk = max_chunk.max(dt);
                }
            }
            (
                t0.elapsed().as_secs_f64(),
                peak_voice,
                max_chunk,
                peak_sample,
            )
        };

        // ── baseline：两个 CPU 后端 ──
        for (name, backend) in [("xsynth-cpu", 0u8), ("yinhe-cpu", 1)] {
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
                    cs.load_dense_soundfonts_many(&denses, &[std::path::PathBuf::from(sfz)])
                        .expect("cpu sf");
                    engine.cpu_synth = Some(cs);
                }
                _ => {}
            }
            engine.handle_command(AudioCommand::Play {
                from_sample: start_sample,
            });
            let _ = backend;
            let (el, pv, mc, ps) = render_secs(&mut engine, prof_secs);
            eprintln!(
                "  {name:<12} {prof_secs}s音频={el:>6.2}s ({:.2}x) peak_voice={pv:>6} max_chunk={mc:>7.1}ms peak_sample={ps:.3}",
                prof_secs / el.max(1e-9)
            );
        }

        // ── CpuSynth 降级分解（同一引擎，模式间 Play 重置）──
        // 超大工程可跳过（`YINHE_PROF_SKIP_BREAKDOWN=1`）：解析与渲染本身就够慢。
        if std::env::var("YINHE_PROF_SKIP_BREAKDOWN").is_ok_and(|v| v == "1") {
            continue;
        }
        let mut cs_engine = AudioEngine::new(sr, layout.clone());
        cs_engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        let mut cs = yinhe_synth::CpuSynth::new(sr);
        cs.load_dense_soundfonts_many(&denses, &[std::path::PathBuf::from(sfz)])
            .expect("cpu sf");
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
            let (el, pv, mc, _ps) = render_secs(&mut cs_engine, prof_secs);
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
}

// ---------------------------------------------------------------------------
// 同 MIDI 通道、同 tick、同一语义目标的自动化取 max（播放聚合）
// ---------------------------------------------------------------------------

/// 两轨同 MIDI 通道（ch0）各一条 CC 自动化 lane，同 tick 各一个 Step 事件。
fn make_two_track_cc_model(controller: u8, tick: u32, v0: u8, v1: u8) -> YinModel {
    let lane = |track: u16, value: u8| AutomationLane {
        target: cc_target(controller),
        track,
        events: vec![AutomationEvent {
            id: 0,
            tick,
            value: value as f32 / 127.0,
            shape: SegmentShape::Step,
        }],
    };
    let mut t0 = TrackData::new(0, 0);
    t0.automation_lanes = vec![lane(0, v0)];
    let mut t1 = TrackData::new(0, 0);
    t1.automation_lanes = vec![lane(1, v1)];
    let mut model = YinModel {
        tracks: vec![Arc::new(t0), Arc::new(t1)],
        meta: ProjectMeta {
            ppq: 480,
            ..ProjectMeta::default()
        },
        ..Default::default()
    };
    model.rebuild();
    model
}

/// 本块 dispatch 实际会发送的 CC 代表事件（与 dispatch 同一套过滤 + 取 max）。
/// 返回 `(controller, value)`，按事件表顺序。
fn cc_reps(engine: &AudioEngine) -> Vec<(u8, f32)> {
    let events = &engine.cc_events;
    let reps = crate::audio_model::aggregate_max_reps(events, |cc| {
        let track_skipped = engine
            .skip_track
            .get(cc.track as usize)
            .copied()
            .unwrap_or(false);
        let lane_skipped = engine
            .am_lane_skip
            .get(cc.track as usize)
            .and_then(|v| v.get(cc.lane as usize))
            .copied()
            .unwrap_or(false);
        !track_skipped && !lane_skipped
    });
    reps.iter()
        .filter_map(|&i| match &events[i].event {
            AudioEvent::Cc { controller, value } => Some((*controller, *value)),
            _ => None,
        })
        .collect()
}

/// 需求：同 MIDI 通道、同 tick、同一语义目标（CC1）的多轨自动化播放时取 max
/// （旧行为是后写覆盖 = track 索引大者胜出），事件流本身不合并（导出不变）。
/// 两个写入顺序都生效。
#[test]
fn test_same_tick_same_channel_cc_takes_max() {
    let sr = 44100u32;
    for (v0, v1) in [(40u8, 100u8), (100, 40)] {
        let model = Arc::new(make_two_track_cc_model(1, 100, v0, v1));
        let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(vec![true; 16]));
        engine.load_model(&model);
        engine.playing = true;

        // 模型数据不合并：事件流仍保留两条（导出/编辑不受影响）。
        let cc1_count = engine
            .cc_events
            .iter()
            .filter(|c| matches!(c.event, AudioEvent::Cc { controller: 1, .. }))
            .count();
        assert_eq!(cc1_count, 2, "事件流不合并数据（仍两条）");

        // CPU dispatch：两条都被消费（cursor 越过），实际只发一条代表。
        engine.dispatch_and_find_next(100, 100_000);
        assert_eq!(engine.cc_cursor, 2, "两条都被消费");
        assert_ne!(
            engine.dispatched_skip.cc_mask[0] & (1u128 << 1),
            0,
            "CC1 应已发送（代表事件）"
        );

        // 聚合结果：同键只发一条代表，值 = max。
        let reps = cc_reps(&engine);
        assert_eq!(reps.len(), 1, "同键只发一条代表事件");
        assert_eq!(reps[0], (1, 100.0), "同 tick 同通道同 CC 取 max");
    }
}

/// 需求：被 AM M/S 旁通（am_lane_skip）的 lane 不参与 max，取未旁通者的值；
/// 轨道 mute（skip_track）同理；全部被过滤则整组不发。
#[test]
fn test_same_tick_cc_max_respects_track_mute_and_am_ms() {
    let model = Arc::new(make_two_track_cc_model(1, 100, 40, 100));
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);

    // 无旁通：取 max = 100。
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);

    // 轨道 mute（skip_track）：取未被 mute 轨道那条的值。
    engine.skip_track = vec![true, false];
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);
    engine.skip_track = vec![false, true];
    assert_eq!(cc_reps(&engine), vec![(1, 40.0)]);
    // 两条都被 mute → 整组不发。
    engine.skip_track = vec![true, true];
    assert!(cc_reps(&engine).is_empty());

    // CPU dispatch：全旁通时不发送（不打点 chase_skip），但 cursor 仍越过。
    engine.cc_cursor = 0;
    engine.dispatched_skip = crate::channel::ChaseSkip::default();
    engine.dispatch_and_find_next(100, 100_000);
    assert_eq!(
        engine.dispatched_skip.cc_mask[0] & (1u128 << 1),
        0,
        "全被过滤则不发"
    );
    assert_eq!(engine.cc_cursor, 2, "事件仍被消费越过");

    // AM M/S（am_lane_skip，由模型 + am_ms map 构建）：mute track0 的 lane
    // → 取 track1 = 100。
    engine.skip_track = vec![false, false];
    let mut am_ms = crate::spawn::AmMsMap::new();
    am_ms.insert(
        (0u16, cc_target(1)),
        yinhe_types::AmMsState {
            mute: true,
            solo: false,
        },
    );
    engine.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &am_ms);
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);

    // solo track1 的 lane：同轨只有它 solo，track0 未 solo 被旁通 → 取 100。
    let mut am_ms = crate::spawn::AmMsMap::new();
    am_ms.insert(
        (1u16, cc_target(1)),
        yinhe_types::AmMsState {
            mute: false,
            solo: true,
        },
    );
    engine.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &am_ms);
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);

    // 两条都 mute → 整组不发。
    let mut am_ms = crate::spawn::AmMsMap::new();
    for track in 0u16..2 {
        am_ms.insert(
            (track, cc_target(1)),
            yinhe_types::AmMsState {
                mute: true,
                solo: false,
            },
        );
    }
    engine.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &am_ms);
    assert!(cc_reps(&engine).is_empty());
}

/// 需求：chase（seek 恢复）与播放一致——同通道同 tick 同目标取 max；
/// 轨道 mute / AM M/S 旁通的 lane 不参与；全被过滤则无状态（应用时不触碰通道）。
#[test]
fn test_chase_same_tick_same_target_takes_max() {
    // 两轨同通道 CC7（volume）：同 tick 100，track0=40、track1=100。
    let make = |v0: u8, v1: u8| {
        let lane = |track: u16, value: u8| AutomationLane {
            target: cc_target(7),
            track,
            events: vec![AutomationEvent {
                id: 0,
                tick: 100,
                value: value as f32 / 127.0,
                shape: SegmentShape::Step,
            }],
        };
        let mut t0 = TrackData::new(0, 0);
        t0.automation_lanes = vec![lane(0, v0)];
        let mut t1 = TrackData::new(0, 0);
        t1.automation_lanes = vec![lane(1, v1)];
        let mut m = YinModel {
            tracks: vec![Arc::new(t0), Arc::new(t1)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        m.rebuild();
        m
    };

    let model = make(40, 100);
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[false, false]);
    assert_eq!(
        states[0].as_ref().map(|s| s.volume),
        Some(100),
        "同 tick 同目标取 max"
    );

    // 写入顺序颠倒（track0=100、track1=40）仍取 max。
    let reversed = make(100, 40);
    let states = crate::spawn::compute_chase_states_for_test(&reversed, 200, &[false, false]);
    assert_eq!(
        states[0].as_ref().map(|s| s.volume),
        Some(100),
        "顺序颠倒仍取 max"
    );

    // 轨道 mute：只取未被 mute 那条。
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[true, false]);
    assert_eq!(states[0].as_ref().map(|s| s.volume), Some(100));
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[false, true]);
    assert_eq!(states[0].as_ref().map(|s| s.volume), Some(40));
    // 两条都 mute → 无状态（chase 应用时不触碰通道）。
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[true, true]);
    assert!(states[0].is_none());

    // AM M/S 旁通：mute track0 的 CC7 lane → 取 track1 = 100。
    let mut am_ms = crate::spawn::AmMsMap::new();
    am_ms.insert(
        (0u16, cc_target(7)),
        yinhe_types::AmMsState {
            mute: true,
            solo: false,
        },
    );
    let states = crate::spawn::compute_chase_states_with_am_ms_for_test(
        &model,
        200,
        &[false, false],
        &am_ms,
    );
    assert_eq!(states[0].as_ref().map(|s| s.volume), Some(100));

    // 两条 lane 都 mute → 无状态。
    am_ms.insert(
        (1u16, cc_target(7)),
        yinhe_types::AmMsState {
            mute: true,
            solo: false,
        },
    );
    let states = crate::spawn::compute_chase_states_with_am_ms_for_test(
        &model,
        200,
        &[false, false],
        &am_ms,
    );
    assert!(states[0].is_none(), "全部旁通则整组不发（无状态）");
}

/// 需求：预览 chase 与播放一致——同 tick 同通道同目标取 max；不同语义键
/// （CC7 与 CC10）互不干扰；更晚 tick 的同键事件照常后写覆盖（不跨 tick 聚合）。
#[test]
fn test_preview_chase_same_tick_takes_max() {
    use crate::audio_model::SortedCC;
    use crate::preview_engine::chase_channel_states;

    let ev = |tick: u32, cc: u8, value: u8| SortedCC {
        tick,
        channel: 0,
        track: 0,
        lane: 0,
        plugin_param: None,
        event: AudioEvent::Cc {
            controller: cc,
            value: f32::from(value),
        },
    };
    let cc_events = vec![
        ev(10, 7, 40),
        ev(10, 7, 100),
        ev(10, 10, 20),
        ev(10, 10, 80),
        ev(20, 7, 10), // 更晚的同键事件：后写覆盖
    ];
    let states = chase_channel_states(&cc_events, 0, &[10, 20]);
    assert_eq!(states[0].volume, 100, "同 tick 同键取 max");
    assert_eq!(states[0].pan, 80, "不同语义键互不干扰");
    assert_eq!(states[1].volume, 10, "更晚的同键事件照常生效");
}

/// 画像（本地 tau + SoundFont，ignored）：复音上限 / layer 对实时倍率的影响。
///
/// 移动端默认 `max_voices=1024`、`layer=4`；tau 峰值段的实际 voice 数远超
/// 这个值。这里用固定 512 帧块测“1 秒音频需要多少墙钟”，定位默认值是否
/// 足以留出调度余量（实时倍率 > 1 才可能稳定）。
#[test]
#[ignore = "需要本地 tau + SoundFont"]
fn prof_tau_voice_caps() {
    use std::sync::Arc;
    use std::time::Instant;
    use yinhe_types::NoteSource;

    let midi = "/Users/jieneng/Music/MIDIs/tau2.5.9.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse tau"));
    let ppq = model.meta.ppq;
    let mut per_beat: BTreeMap<u32, usize> = Default::default();
    for key in 0..KEY_COUNT {
        for n in model.key_notes(key as u8).iter() {
            *per_beat.entry(n.start_tick / ppq).or_default() += 1;
        }
    }
    let (peak_beat, peak_count) = per_beat
        .iter()
        .max_by_key(|(_, v)| **v)
        .map(|(b, c)| (*b, *c))
        .expect("tau has notes");
    let layout = crate::spawn::channels_for_model(&model);
    let active: Vec<u8> = (0..16u8)
        .filter(|c| layout.is_active(*c as usize))
        .collect();
    let start_sample = {
        let mut probe = AudioEngine::new(sr, layout.clone());
        probe.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        probe.tick_to_sample(peak_beat.saturating_mul(ppq).saturating_sub(ppq * 2))
    };
    eprintln!(
        "=== tau voice caps: peak_beat={peak_beat} count={peak_count} start_sample={start_sample}"
    );

    for (cap, layer) in [
        (Some(1024usize), Some(4usize)),
        (Some(512), Some(4)),
        (Some(256), Some(4)),
        (Some(1024), Some(2)),
        (Some(1024), Some(1)),
    ] {
        let mut engine = AudioEngine::new(sr, layout.clone());
        engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        let mut cs = yinhe_synth::CpuSynth::new(sr);
        for c in &active {
            cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                .expect("cpu sf");
        }
        if let Some(cap) = cap {
            cs.set_max_voices(cap);
        }
        cs.set_layer_count(layer);
        engine.cpu_synth = Some(cs);
        engine.set_max_voices(cap);
        engine.set_layer_count(layer);
        engine.handle_command(AudioCommand::Play {
            from_sample: start_sample,
        });

        let frames = 512usize;
        let mut buf = vec![0.0f32; frames * 2];
        let target = sr as u64 / 2; // 0.5s
        let mut rendered = 0u64;
        let mut peak_voice = 0u64;
        let mut max_chunk = 0.0f64;
        let t0 = Instant::now();
        while rendered < target {
            let tc = Instant::now();
            engine.render(&mut buf);
            let dt = tc.elapsed().as_secs_f64() * 1000.0;
            rendered += frames as u64;
            peak_voice = peak_voice.max(engine.voice_count());
            if rendered > sr as u64 / 8 {
                max_chunk = max_chunk.max(dt);
            }
        }
        let elapsed = t0.elapsed().as_secs_f64();
        eprintln!(
            "  cap={cap:?} layer={layer:?} 0.5s音频={elapsed:.2}s ({:.2}x) peak_voice={peak_voice} max_chunk={max_chunk:.1}ms",
            0.5 / elapsed.max(1e-9)
        );
    }
}

/// 循环区间映射到零 sample（极快 tempo 下 1 tick < 1 sample）时，回绕不能
/// 让播放位置前进；必须按普通播放继续，而不是在音频线程里死循环。
#[test]
fn zero_sample_loop_does_not_spin() {
    let model = make_model_with_tempo(vec![(0, 12000.0)], vec![]);
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.playing = true;

    // tick 0..1 在 12000 BPM 下都落在 sample 0。
    engine.set_loop(true, 0, 1);
    let mut out = vec![0.0f32; 512 * 2];
    engine.render(&mut out);
    assert_eq!(engine.sample_position, 512, "零 sample 循环必须照常前进");
}

/// 测试用 insert：持续输出一个按样本衰减的"尾音"，忽略输入。
struct TailInsert {
    value: f32,
}

impl yinhe_mixer::InsertProcessor for TailInsert {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for i in 0..left.len().min(right.len()) {
            left[i] += self.value;
            right[i] += self.value;
            self.value *= 0.999;
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

fn tail_peak(engine: &mut AudioEngine) -> f32 {
    let mut buf = vec![0.0f32; 512 * 2];
    engine.render_tail(&mut buf);
    buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

/// 暂停后 insert 尾音仍由 render_tail 输出，且不推进走带。
#[test]
fn pause_keeps_insert_tail_and_transport_frozen() {
    let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
    let model = Arc::new(model);
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.insert_add(
        crate::spawn::InsertTarget::Channel(0),
        0,
        Box::new(TailInsert { value: 0.5 }),
    );

    let pos = engine.sample_position();
    engine.handle_command(crate::spawn::AudioCommand::Pause);
    let peak = tail_peak(&mut engine);

    assert!(peak > 0.01, "暂停后应继续输出 insert 尾音，peak={peak}");
    assert_eq!(engine.sample_position(), pos, "尾音渲染不得推进走带");
}

/// Stop 复位走带到 0，但不 reset insert，尾音继续。
#[test]
fn stop_keeps_insert_tail() {
    let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
    let model = Arc::new(model);
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.insert_add(
        crate::spawn::InsertTarget::Channel(0),
        0,
        Box::new(TailInsert { value: 0.5 }),
    );

    engine.handle_command(crate::spawn::AudioCommand::StopTransport);
    assert_eq!(engine.sample_position(), 0, "Stop 应回到 0");
    let peak = tail_peak(&mut engine);
    assert!(peak > 0.01, "Stop 后应继续输出 insert 尾音，peak={peak}");
}
