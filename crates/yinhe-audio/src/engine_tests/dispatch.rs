use super::*;

/// 完整 bug 复现 + 修复验证：空 Document（16 条音轨占满 ch 0-15）→
/// 引擎 spawn 时通道已全部激活 → 写第一个音符立即发声，无需 teardown + 重建。
#[test]
fn test_first_note_on_fresh_document_dispatches_without_rebuild() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();

    // 1. 空 Document spawn 引擎：16 条音轨的通道 0-15 全部激活
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playback.playing = true;
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
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.active_notes.len(), 1);
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
    engine.playback.playing = true;

    // 播放推进一块：两个音符 NoteOn，游标越过它们
    let mut out = vec![0.0f32; 1024];
    engine.render(&mut out);
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.note_cursor[64].idx, 1);

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
    assert_eq!(
        engine.playback.note_cursor[64].idx, 1,
        "干净桶游标越过已派发音符"
    );
    assert_eq!(
        engine.playback.note_cursor[60].idx, 1,
        "新音符排在已派发音符之后"
    );

    // 继续 dispatch：480 = 两个 NoteOff，960 = 新音符的 NoteOn
    let next = engine.dispatch_and_find_next(480, 60000);
    assert_eq!(next, Some(960), "新音符 NoteOn 位置");
    assert_eq!(
        engine.playback.active_notes.len(),
        0,
        "两个 NoteOff 已弹，新音符未到"
    );
    let next = engine.dispatch_and_find_next(960, 70000);
    assert_eq!(next, Some(1440), "新音符 NoteOff = 1440 tick");
    assert_eq!(engine.playback.active_notes.len(), 1, "新音符已 NoteOn");
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

    // 4. add_track(1)：新 track 在 idx 2，channel 15（第一个空闲）
    doc.add_track(1);
    doc.data.bump_revision();

    // 5. 新 layout：channel 15 已激活——即使新音轨还没有任何音符
    let layout_after = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_after.is_active(0), "channel 0 still active");
    assert!(layout_after.is_active(15), "channel 15 now active");

    // 6. 重建引擎 → track 1 的音符能 dispatch
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playback.playing = true;
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.active_notes.len(), 1);
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

    // 3. remove track 2（通道 1 的音符随之删除）
    doc.remove_track(2);
    doc.data.bump_revision();

    // 4. 新 layout：通道 1 失活，其余 15 个通道仍激活
    let layout_after = crate::spawn::channels_for_model(&doc.data.model);
    assert!(layout_after.is_active(0));
    assert!(!layout_after.is_active(1));

    // 5. 重建引擎 → 只有通道 0 的音符 dispatch
    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playback.playing = true;
    let next = engine.dispatch_and_find_next(0, 60000);
    assert_eq!(next, Some(480));
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.note_cursor[64].idx, 0);
    assert_eq!(engine.playback.active_notes.len(), 1);
}

/// mute 轨道的自动化事件（CC）在 dispatch 时应被跳过，
/// 不发送到合成器，使同 channel 上其他非 mute 轨道不受影响。
#[test]
fn test_muted_track_cc_skipped_in_dispatch() {
    use crate::audio_model::SortedCC;
    use crate::audio_model::{ChannelAudioEvent, ControlEvent};

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
    engine.playback.playing = true;

    // 注入两条 CC 事件：track 0（mute）和 track 1（非 mute），同 channel 0
    engine.playback.cc_events = Arc::new(vec![
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
    engine.playback.cc_cursor = 0;
    // mute track 0
    engine.playback.skip_track = vec![true, false];

    // dispatch 应跳过 track 0 的 CC，只发 track 1 的
    engine.dispatch_and_find_next(0, 60000);
    // cc_cursor 推进到末尾（两条都处理了，但只发了一条）
    assert_eq!(engine.playback.cc_cursor, 2);
}

/// 回归测试：mute 期间被 cc_cursor 越过但未 dispatch 的自动化事件，
/// chase_skip 不得标记——否则 unmute 后 chase 跳过这些控制器，
/// 该轨道的自动化状态永远丢失（卡在 mute 前的旧值），直到下次 seek。
#[test]
fn test_unmute_chase_skip_excludes_events_missed_while_muted() {
    use crate::audio_model::SortedCC;
    use crate::audio_model::{ChannelAudioEvent, ControlEvent};

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
    engine.playback.playing = true;

    // track 0 的两条 CC（用不同控制器以区分位图）：tick 100 = CC7 40，tick 300 = CC1 80。
    // CC1 用于验证"mute 期间越过未 dispatch"；CC7 的 dispatch 不受影响。
    engine.playback.cc_events = Arc::new(vec![
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
    assert_eq!(engine.playback.cc_cursor, 1, "tick 100 的事件应已 dispatch");

    // mute 轨道 0，继续播放越过 tick 300：CC1=80 被 cursor 越过但未 dispatch
    engine.playback.skip_track = vec![true];
    engine.dispatch_and_find_next(400, 100000);
    assert_eq!(
        engine.playback.cc_cursor, 2,
        "tick 300 的事件应被越过（未 dispatch）"
    );

    // unmute：chase 要恢复 CC1=80，chase_skip 不得标记 CC1
    //（CC7=40 在 mute 前已 dispatch，bit7 置位是预期行为）
    engine.playback.skip_track = vec![false];
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
    engine.playback.playing = true;
    // Document::empty() 的 track_audible_count 在 add_note 后未重建：显式标记全部可听。
    let all_audible = vec![false; engine.playback.skip_track.len()];
    engine.playback.skip_track = all_audible.clone();

    engine.seek_to(0);
    engine.dispatch_and_find_next(500, 100000);
    // 模拟播放推进：手动 dispatch 不会更新 current_tick（由 render 推进）。
    engine.playback.current_tick = 500;
    assert_eq!(
        engine.playback.active_notes.len(),
        1,
        "播放中应有一个活跃音符"
    );

    // mute track 1 → 立即从 active_notes 移除（发 NoteOff）
    let mut muted = all_audible.clone();
    muted[1] = true;
    engine.apply_skip_mask(&all_audible, &muted);
    assert_eq!(
        engine.playback.active_notes.len(),
        0,
        "mute 应立即停掉该轨在响音符，而不是等 gate 结束"
    );

    // unmute track 1 → 立即重启跨点音符（回到 active_notes）
    engine.apply_skip_mask(&muted, &all_audible);
    assert_eq!(
        engine.playback.active_notes.len(),
        1,
        "unmute 应立即重启跨点音符"
    );
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
    let states =
        crate::spawn::compute_chase_states_for_test(&model, 768, &engine.playback.skip_track);
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
    engine.playback.playing = true;

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
        max_active = max_active.max(engine.playback.active_notes.len());
    }

    // 全部事件已 dispatch：活跃音符清空、所有桶 cursor 到末尾
    assert_eq!(
        engine.playback.active_notes.len(),
        0,
        "所有音符都应 NoteOff"
    );
    assert!(max_active >= 2, "渲染过程中应存在叠层音符");
    for key in 0..128usize {
        assert_eq!(
            engine.playback.note_cursor[key].idx as usize,
            engine
                .playback
                .yin_model
                .as_ref()
                .map(|m| m.notes[key].len())
                .unwrap_or(0),
            "key {key} 的桶应全部 dispatch"
        );
    }
    assert_eq!(
        engine.playback.sample_position as usize,
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
    engine.playback.playing = true;

    // 一帧 512 帧：块内 tick 跨度约 1113，覆盖全部事件
    let mut out = vec![0.0f32; 1024];
    engine.render(&mut out);
    assert_eq!(
        engine.playback.sample_position, 512,
        "整块渲染完成，无死循环"
    );
    assert_eq!(engine.playback.active_notes.len(), 0, "两个音符都 NoteOff");
    assert_eq!(engine.playback.note_cursor[60].idx, 1);
    assert_eq!(engine.playback.note_cursor[64].idx, 1);
}

/// seek 到任意位置后 dispatch：跨 seek 点的音符不重复 NoteOn
/// （seek 已重启一次，dispatch 不能再次触发）。
#[test]
fn test_seek_then_dispatch_no_duplicate_note_on() {
    let model = make_model_with_tempo(vec![(0, 120.0)], vec![(60, 0, 960), (64, 480, 1440)]);
    let mask = vec![true; 16];
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(mask));
    engine.load_model(&model);
    engine.playback.playing = true;

    // seek 到 30000 sample ≈ tick 653（两个音符都已开始）
    let seek_sample = 30000u64;
    engine.seek_to(seek_sample);
    assert_eq!(engine.current_tick(), 653, "sample_to_tick(30000) = 653");
    assert_eq!(
        engine.playback.active_notes.len(),
        2,
        "seek 重启两个跨点音符"
    );

    // dispatch seek 点：不重复 NoteOn（cursor 已跳过），只处理 NoteOff 边界
    let next = engine.dispatch_and_find_next(653, 3000);
    assert_eq!(next, Some(960), "下一个事件是两个音符的 NoteOff 960");
    assert_eq!(
        engine.playback.active_notes.len(),
        2,
        "dispatch 不重复 NoteOn"
    );

    // NoteOff 960 触发一次，active 减 1
    engine.dispatch_and_find_next(960, 3000);
    assert_eq!(engine.playback.active_notes.len(), 1);
    // 1440 处第二个 NoteOff
    engine.dispatch_and_find_next(1440, 3000);
    assert_eq!(engine.playback.active_notes.len(), 0);
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
    engine.playback.playing = true;
    engine.synth.ignore_velocity = 1;

    // tick 0：vel=1 被过滤 → 没有 NoteOn，下一个边界是 480
    let next = engine.dispatch_and_find_next(0, 2000);
    assert_eq!(next, Some(480), "vel=1 不派发，边界跳到下一个音符");
    assert_eq!(engine.playback.active_notes.len(), 0);

    // tick 480：vel=2 派发
    let next = engine.dispatch_and_find_next(480, 2000);
    assert_eq!(next, Some(960));
    assert_eq!(engine.playback.active_notes.len(), 1);

    // 阈值改为 0 → vel=1 的音符重新可听（不影响已经过去的 0 tick，
    // 但说明阈值只是 dispatch 期的判断，无缓存可失效）
    engine.synth.ignore_velocity = 0;
    let next = engine.dispatch_and_find_next(960, 2000);
    assert_eq!(next, Some(1440));
    // vel=2 在 960 结束（NoteOff），vel=3 同时 NoteOn → 活跃数仍为 1。
    assert_eq!(
        engine.playback.active_notes.len(),
        1,
        "vel=3 已派发、vel=2 已结束"
    );
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
    engine.playback.playing = true;
    engine.dispatch_and_find_next(0, 60_000);

    // 打点 = 该 CC 已被实际消费（内置音源通道处理段）；外挂 insert 不再收 CC。
    let skip = engine.chase_skip();
    assert!(
        skip.cc_mask[0] & (1u128 << 7) != 0,
        "CC7 应由内置音源通道处理段消费"
    );
}
