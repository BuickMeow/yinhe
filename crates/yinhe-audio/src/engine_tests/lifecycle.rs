use super::*;

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
    engine.synth.cpu_synth = synth;
    engine.handle_command(AudioCommand::Play { from_sample: 0 });

    let mut out = vec![0.0f32; 512 * 2];
    for _ in 0..10 {
        engine.render(&mut out);
    }
    engine.publish_track_levels();
    let levels = engine.meter.track_levels.lock().unwrap().clone();
    assert_eq!(levels.len(), 1, "一轨一个电平表槽");
    let (l, r) = levels[0];
    assert!(l > 0.0 || r > 0.0, "发声轨道应有电平（{l}/{r}）");

    // 停止播放 → 全部 0。
    engine.playback.playing = false;
    engine.publish_track_levels();
    assert!(
        engine
            .meter
            .track_levels
            .lock()
            .unwrap()
            .iter()
            .all(|&(l, r)| l == 0.0 && r == 0.0)
    );
}

/// 空闲通道跳过回归：整颗音符（含 release）落在单个块内时，不能因为
/// "渲染后没有存活 voice"把这块当成静音跳过。
#[test]
#[ignore = "需要 SoundFont"]
fn idle_skip_keeps_note_that_ends_inside_block() {
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = make_model_with_notes(vec![(60, 0, 1, 100, 0)]);
    let layout = crate::spawn::channels_for_model(&model);
    let mut engine = AudioEngine::new(sr, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::new(model),
    });
    let mut synth = yinhe_synth::CpuSynth::new(sr);
    synth
        .load_dense_soundfonts(0, &[std::path::PathBuf::from(sfz)])
        .expect("音色库加载");
    engine.synth.cpu_synth = synth;
    engine.handle_command(AudioCommand::Play { from_sample: 0 });

    // 一块 3 秒：音符与 release 都在块内结束（渲染后无存活 voice）。
    let mut out = vec![0.0f32; sr as usize * 3 * 2];
    engine.render(&mut out);
    assert!(
        out.iter().any(|s| s.abs() > 0.0),
        "块内结束的音符不能被空闲跳过吞掉"
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
    engine.playback.playing = true;

    // Note at key 60, start_tick=960, velocity=100 → should dispatch at tick 960.
    // @48000Hz 1 tick = 50 sample：960 tick = 48000 sample，1200 tick = 60000 sample。
    let next = engine.dispatch_and_find_next(960, 1200);
    // NoteOff at tick 1440 > block_end 1200, so no next event in range.
    assert_eq!(next, None);

    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.active_notes.len(), 1);
    assert_eq!(engine.sample_position(), 0);
}

#[test]
fn transport_loop_wraps_at_exact_sample() {
    let model = Arc::new(make_model_with_notes(vec![]));
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.playback.playing = true;

    // 120 BPM、PPQ 480：1 tick = 50 samples；480 tick = 24000 samples。
    engine.set_loop(true, 0, 480);
    assert!(engine.playback.loop_enabled);

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
    assert_eq!(engine.playback.loop_wrap_offset, 24000);

    // 再跑一圈：偏移继续累加，绝对位置保持单调。
    for _ in 0..47 {
        engine.render(&mut out);
    }
    assert_eq!(engine.sample_position(), 128);
    assert_eq!(engine.playback.loop_wrap_offset, 48000);
}

#[test]
fn invalid_loop_range_disables_loop() {
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.set_loop(true, 480, 480);
    assert!(!engine.playback.loop_enabled);
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

    assert_eq!(engine.playback.note_cursor[60].idx, 0);
    // Note at key 60, start_tick=960, velocity=100 → should dispatch at tick 960
    //（44100 Hz：1 tick = 45.94 sample，960 tick = 44100 sample）。
    let next = engine.dispatch_and_find_next(960, 1306);
    // Next note (other track) starts at tick1440 = 66150 sample > block_end, so no next event.
    assert_eq!(next, None);
    // key 60 桶有 3 个音符：vel 0/1 在 dispatch 时被跳过，vel=100 已派发。
    assert_eq!(engine.playback.note_cursor[60].idx, 3);
    assert_eq!(engine.playback.active_notes.len(), 1);
    for key in 0..128usize {
        if key != 60 {
            assert_eq!(engine.playback.note_cursor[key].idx, 0);
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
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.note_cursor[61].idx, 1);
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
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(960, 2500);
    // 处理 NoteOff 960 后，下一个事件是 key 0 的 NoteOn at 2000。
    assert_eq!(next, Some(2000));
    // key 60 ended, so only key 0 is active.
    assert_eq!(engine.playback.active_notes.len(), 0);

    let next = engine.dispatch_and_find_next(2000, 2500);
    // key 0 NoteOn 后，NoteOff at end_tick=2480。
    assert_eq!(next, Some(2480));
    assert_eq!(engine.playback.note_cursor[0].idx, 1);
    // key 0 is active.
    assert_eq!(engine.playback.active_notes.len(), 1);

    let next = engine.dispatch_and_find_next(2480, 2500);
    // [2480, 2500) 内无更多事件。
    assert_eq!(next, None);
    assert_eq!(engine.playback.active_notes.len(), 0);
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

/// 用户 seek 必须作废旧位置算出的 chase（否则会把旧位置的 CC7/Program
/// 带回新位置）；loop 回绕不能作废，否则 loop_chase 永远应用不上。
#[test]
fn seek_invalidates_chase_but_loop_wrap_keeps_it() {
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(vec![true; 16]));
    let before = engine.playback.chase_generation;
    engine.handle_command(AudioCommand::Seek { sample: 44100 });
    assert_eq!(
        engine.playback.chase_generation,
        before.wrapping_add(1),
        "seek 应推进 chase 代次"
    );

    engine.playback.loop_enabled = true;
    engine.playback.loop_start_tick = 0;
    engine.playback.loop_end_tick = 480;
    let before = engine.playback.chase_generation;
    engine.rewind_to_loop_start();
    assert_eq!(
        engine.playback.chase_generation, before,
        "loop 回绕不得作废 loop_chase"
    );
}

#[test]
fn test_engine_handle_command_skip_tracks() {
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    let skip = vec![false, true, false];
    engine.handle_command(AudioCommand::SkipTracks { skip });
    assert_eq!(engine.playback.skip_track, vec![false, true, false]);
}

/// 回归：向后 seek 时鼓组声明必须落在新位置，不能是旧位置。
/// 旧位置会作为"未来事件"排在跨点 NoteOn 前面，卡住 CpuSynth 事件游标，
/// 表现为向后跳转后一直无声、等播放头追上旧位置才恢复。
#[test]
fn backward_seek_keeps_synth_event_queue_in_past() {
    let model = Arc::new(make_model_with_notes(vec![(60, 0, 480, 100, 0)]));
    let mut engine = AudioEngine::new(48_000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.synth.cpu_synth = yinhe_synth::CpuSynth::new(48_000);

    engine.seek_to(100_000);
    engine.seek_to(50_000);

    let first = engine.synth.cpu_synth.debug_first_event_sample();
    assert!(first.is_some(), "seek 后应有鼓组声明事件");
    assert!(
        first.unwrap() <= 50_000,
        "向后 seek 后事件样本必须 <= 新位置（实际 {first:?}）"
    );
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
    let cc_count_a = engine.playback.cc_events.len();
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
        engine.playback.cc_events.len(),
        12,
        "ReloadNotes must rebuild cc_events from the new model (was {} from model A)",
        cc_count_a
    );

    // Assert events are sorted (so the schedule loop's monotonic cursor works).
    for w in engine.playback.cc_events.windows(2) {
        assert!(w[0].tick <= w[1].tick, "cc_events must be sorted by tick");
    }

    // Reload again with an empty model — cc_events must drain to zero.
    let model_c = Arc::new(make_model_with_controls(vec![], vec![], vec![], vec![]));
    engine.load_model(&model_c);
    assert_eq!(
        engine.playback.cc_events.len(),
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
    engine.playback.playing = true;

    // NoteOff at tick 480 = 1 beat @ 120 BPM @ 44100 Hz = 22050 samples.
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.active_notes.len(), 1);
}
