use super::*;

#[test]
fn empty_creates_valid_document_with_conductor_and_16_tracks() {
    let doc = Document::empty();
    assert_eq!(doc.model().tracks.len(), 17);
    assert_eq!(doc.model().tracks[0].name, "Master");
    assert_eq!(doc.model().tracks[1].name, "Track 1");
    assert_eq!(doc.model().tracks[16].name, "Track 16");
    assert_eq!(doc.edit.track_cache.conductor_idx, Some(0));
    assert_eq!(doc.edit.track_visible.len(), 17);
    assert_eq!(doc.edit.track_pianoroll_visible.len(), 17);
    assert_eq!(doc.file_name, "Untitled");
    // Conductor track channels: Track 1 on ch0, Track 16 on ch15
    assert_eq!(doc.model().tracks[1].channel, 0);
    assert_eq!(doc.model().tracks[16].channel, 15);
}

#[test]
fn skip_mask_is_solo_priority() {
    let mut doc = Document::empty();
    // A 轨同时 M+S，B 轨普通：A 应该放行（Solo 优先于 Mute），B 静音。
    doc.edit.track_overrides[1].muted = true;
    doc.edit.track_overrides[1].soloed = true;
    let mask = doc.compute_skip_mask();
    assert!(!mask[1], "Solo 必须压过自身的 Mute");
    assert!(mask[2], "无 solo 命中时普通轨应被静音");

    // 再加 B 轨 Solo：A、B 都放行，C 静音。
    doc.edit.track_overrides[2].soloed = true;
    let mask = doc.compute_skip_mask();
    assert!(!mask[1]);
    assert!(!mask[2]);
    assert!(mask[3]);
}

#[test]
fn track_solo_clears_channel_solos_and_vice_versa() {
    let mut doc = Document::empty();
    let node = MixerChannelId::Midi(0);
    assert!(doc.set_channel_audibility(node, Audibility::Soloed));
    assert_eq!(doc.mixer.node_strip(node).audibility(), Audibility::Soloed);

    assert!(doc.set_track_audibility(1, Audibility::Soloed));
    assert_eq!(
        doc.mixer.node_strip(node).audibility(),
        Audibility::Normal,
        "Track solo 必须清空 Channel solo"
    );

    assert!(doc.set_channel_audibility(MixerChannelId::Midi(1), Audibility::Soloed));
    assert_eq!(
        doc.edit.track_overrides[1].audibility(),
        Audibility::Normal,
        "Channel solo 必须清空 Track solo"
    );
}

#[test]
fn empty_conductor_only_has_single_master_track() {
    let doc = Document::empty_conductor_only();
    assert_eq!(doc.model().tracks.len(), 1);
    assert_eq!(doc.model().tracks[0].name, "Master");
    assert!(doc.model().tracks[0].kind == yinhe_core::TrackKind::Master);
    assert_eq!(doc.edit.track_cache.conductor_idx, Some(0));
    assert_eq!(doc.edit.track_visible.len(), 1);
    assert_eq!(doc.edit.track_pianoroll_visible.len(), 1);
    assert_eq!(doc.model().note_count, 0);
}

/// 回归测试：每个新文档的初始 revision 必须全局唯一。
/// GPU layer 缓存键（AR notes / PR notes / AM curve）以 revision 为成分
/// 且不含文档身份；若两个文档 revision 相同（如都恒为 0），打开 .yin 后
/// 缓存键与上一文档逐位相同，渲染器跳过重建，必须滚动视口才能刷新。
#[test]
fn new_documents_have_unique_revisions() {
    let a = Document::empty();
    let b = Document::empty();
    assert_ne!(
        a.data.revision, b.data.revision,
        "新文档初始 revision 必须全局唯一，否则跨文档 GPU 缓存键碰撞"
    );
}

/// PR 与 AR 的显隐互相独立，密度预览同理：`pianoroll_visible` 只影响
/// `density_pr`（PR 滚动条），`track_visible` 只影响 `density`（AR 滚动条）。
#[test]
fn density_pr_follows_pianoroll_visibility_separately() {
    use std::sync::Arc;
    use std::time::Duration;
    use yinhe_types::Note;
    let note = |id: u32, track: u16| Note {
        id,
        start_tick: 100,
        end_tick: 200,
        velocity: 100,
        track,
    };
    let mut doc = Document::empty();
    {
        let m = Arc::make_mut(&mut doc.data.model);
        m.insert_note(60, note(1, 0));
        m.insert_note(60, note(2, 1));
    }
    doc.data.rebuild_model_dirty();
    doc.sync_density(Duration::MAX);
    assert_eq!(doc.density.view().rows.iter().sum::<u32>(), 2);
    assert_eq!(doc.density_pr.view().rows.iter().sum::<u32>(), 2);

    // PR 画布显隐：隐藏 track 1 → 只有 density_pr 变化。
    doc.edit.track_pianoroll_visible[1] = false;
    doc.sync_density(Duration::MAX);
    assert_eq!(
        doc.density.view().rows.iter().sum::<u32>(),
        2,
        "AR 数据不受 PR 显隐影响"
    );
    assert_eq!(
        doc.density_pr.view().rows.iter().sum::<u32>(),
        1,
        "PR 数据跟随 pianoroll 显隐"
    );

    // AR 显隐：隐藏 track 0 → 只有 density 变化。
    doc.edit.track_visible[0] = false;
    doc.sync_density(Duration::MAX);
    assert_eq!(
        doc.density.view().rows.iter().sum::<u32>(),
        1,
        "AR 数据跟随 track_visible"
    );
    assert_eq!(doc.density_pr.view().rows.iter().sum::<u32>(), 1);
}

/// 导入无混音数据的 MIDI / 新建工程不预设额外通道；.yin 工程自带混音数据
/// （Some）时原样保留。
#[test]
fn from_model_new_projects_have_no_default_bus() {
    let make = |mixer: Option<MixerParams>| {
        let mut model = YinModel::new_empty_with_16_tracks();
        model.rebuild();
        Document::from_model(
            "/tmp/test.mid",
            model,
            QuantizePreset::Fraction(1, 4),
            QuantizePreset::Fraction(1, 16),
            ProjectFile::default(),
            MappingFile::default(),
            mixer,
        )
        .expect("from_model")
    };

    let imported = make(None);
    assert!(
        imported
            .mixer
            .nodes()
            .all(|node| { matches!(node.source, yinhe_types::ChannelSource::Midi(_)) })
    );

    let with_saved_mixer = make(Some(MixerParams::default()));
    assert_eq!(
        with_saved_mixer.mixer.nodes().count(),
        imported.mixer.nodes().count()
    );
}

#[test]
fn detect_conductor_none_when_track0_has_notes() {
    let t = TrackData::new(0, 0);
    let mut model = YinModel {
        tracks: vec![Arc::new(t)],
        ..Default::default()
    };
    model.load_track_notes(vec![vec![yinhe_core::NoteEvent {
        id: 0,
        start_tick: 0,
        end_tick: 480,
        key: 60,
        velocity: 100,
    }]]);
    model.rebuild();
    assert_eq!(detect_conductor_from_model(&model), None);
}

#[test]
fn detect_conductor_some_when_track0_has_no_notes_and_no_ctrl() {
    let mut t1 = TrackData::new(0, 0);
    t1.name = "Master".into();
    let mut t2 = TrackData::new(0, 0);
    t2.name = "Piano".into();
    let mut model = YinModel {
        tracks: vec![Arc::new(t1), Arc::new(t2)],
        ..Default::default()
    };
    model.load_track_notes(vec![
        vec![],
        vec![yinhe_core::NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }],
    ]);
    model.rebuild();
    assert_eq!(detect_conductor_from_model(&model), Some(0));
}

#[test]
fn track_color_conductor_is_whiteish() {
    // Conductor 使用主文字色（text_primary），而非固定白/黑
    let t = TrackData::new(0, 0);
    let color = track_color(&t, 0, Some(0));
    let dark_c = yinhe_theme::base::BaseColors::DARK.text;
    assert_eq!(
        color,
        [
            dark_c.r as f32 / 255.0,
            dark_c.g as f32 / 255.0,
            dark_c.b as f32 / 255.0,
            1.0
        ]
    );
    let color_light = track_color_with_dark(&t, 0, Some(0), false);
    let light_c = yinhe_theme::base::BaseColors::LIGHT.text;
    assert_eq!(
        color_light,
        [
            light_c.r as f32 / 255.0,
            light_c.g as f32 / 255.0,
            light_c.b as f32 / 255.0,
            1.0
        ]
    );
    // 精确 text_primary 透传
    let custom = [0.11, 0.22, 0.33, 1.0];
    assert_eq!(
        track_color_with_conductor_color(&t, 0, Some(0), custom),
        custom
    );
}

#[test]
fn gray_other_track_colors_keeps_highlight_and_alpha() {
    let colors = [
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 0.5],
        [0.0, 0.0, 1.0, 0.25],
    ];
    let out = gray_other_track_colors(&colors, &[true, false, false]);
    assert_eq!(out[0], colors[0], "高亮轨保持原色");
    assert_eq!(out[1][3], 0.5, "alpha 不变");
    assert_eq!(out[2][3], 0.25);
    for gray in &out[1..] {
        assert!(
            (gray[0] - gray[1]).abs() < 1e-6 && (gray[1] - gray[2]).abs() < 1e-6,
            "其他轨 RGB 必须相等: {gray:?}"
        );
    }
    assert!((out[1][0] - 0.587).abs() < 1e-6);
    assert!((out[2][0] - 0.114).abs() < 1e-6);
}

#[test]
fn gray_other_track_colors_without_highlight_returns_original() {
    let colors = [[1.0, 0.2, 0.3, 1.0], [0.1, 0.8, 0.2, 0.7]];
    assert_eq!(
        gray_other_track_colors(&colors, &[false, false]),
        colors.to_vec()
    );
    assert_eq!(gray_other_track_colors(&colors, &[]), colors.to_vec());
}

#[test]
fn repeat_tiles_fill_full_periods_then_partial() {
    let tiles = repeat_tiles_x(0, 1920, 2400);
    assert_eq!(
        tiles,
        vec![
            RepeatTile {
                src_start: 0,
                dst_start: 1920,
                len: 1920
            },
            RepeatTile {
                src_start: 0,
                dst_start: 3840,
                len: 480
            },
        ]
    );
}

#[test]
fn repeat_tiles_left_clips_selection_tail() {
    let tiles = repeat_tiles_x(1920, 3840, -2400);
    assert_eq!(
        tiles,
        vec![
            RepeatTile {
                src_start: 1920,
                dst_start: 0,
                len: 1920
            },
            RepeatTile {
                src_start: 3360,
                dst_start: -480,
                len: 480
            },
        ]
    );
}

#[test]
fn repeat_tiles_partial_first_copy() {
    assert_eq!(
        repeat_tiles_x(0, 1920, 480),
        vec![RepeatTile {
            src_start: 0,
            dst_start: 1920,
            len: 480
        }]
    );
}

#[test]
fn track_color_cycles_through_palette() {
    let t = TrackData::new(0, 0);
    let first = track_color(&t, 0, None);
    assert_eq!(
        first,
        [
            TRACK_PALETTE[0][0],
            TRACK_PALETTE[0][1],
            TRACK_PALETTE[0][2],
            1.0
        ]
    );
    let second = track_color(&t, 1, None);
    assert_eq!(
        second,
        [
            TRACK_PALETTE[1][0],
            TRACK_PALETTE[1][1],
            TRACK_PALETTE[1][2],
            1.0
        ]
    );
    let wrap = track_color(&t, 16, None);
    assert_eq!(
        wrap,
        [
            TRACK_PALETTE[0][0],
            TRACK_PALETTE[0][1],
            TRACK_PALETTE[0][2],
            1.0
        ]
    );
}

#[test]
fn track_color_offsets_after_conductor() {
    let t = TrackData::new(0, 0);
    let color = track_color(&t, 1, Some(0));
    assert_eq!(
        color,
        [
            TRACK_PALETTE[0][0],
            TRACK_PALETTE[0][1],
            TRACK_PALETTE[0][2],
            1.0
        ]
    );
}

#[test]
fn track_color_prefers_explicit_color_over_palette() {
    let mut t = TrackData::new(0, 0);
    t.color = [0.1, 0.2, 0.3, 0.5];
    assert_eq!(track_color(&t, 3, None), [0.1, 0.2, 0.3, 0.5]);
    // 默认占位色回退到调色板
    let t2 = TrackData::new(0, 0);
    assert_eq!(
        track_color(&t2, 3, None),
        [
            TRACK_PALETTE[3][0],
            TRACK_PALETTE[3][1],
            TRACK_PALETTE[3][2],
            1.0
        ]
    );
}

/// 回归测试：删除轨道后 undo 必须恢复被删轨道上的音符。
/// 之前 TrackStructure 只存轨道元数据，被删音符在 retain 中物理消失，
/// undo 时彻底丢失。修复后 deleted_notes 字段携带被删音符供 undo 恢复。
#[test]
fn remove_track_undo_restores_deleted_notes() {
    use crate::history::{EditSnapshot, UndoEntry};
    use yinhe_core::NoteEvent;

    let mut doc = Document::empty();
    // conductor 在 track 0，Track 1 在 track 1。给 track 1 加 2 个音符。
    let notes_to_add = vec![
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
            velocity: 80,
        },
    ];
    for n in notes_to_add {
        let action = doc.add_note(1, n).expect("add_note should succeed");
        doc.history.push(UndoEntry {
            action,
            label: "add_note".to_string(),
            snapshot: EditSnapshot::default(),
        });
    }
    assert_eq!(doc.model().note_count, 2, "添加后应有 2 个音符");
    assert_eq!(doc.model().track_note_count[1], 2, "track 1 应有 2 个音符");

    // 删除 track 1（其上的 2 个音符应被物理删除）
    let action = doc.remove_track(1).expect("remove_track should succeed");
    doc.history.push(UndoEntry {
        action,
        label: "remove_track".to_string(),
        snapshot: EditSnapshot::default(),
    });
    assert_eq!(doc.model().note_count, 0, "删除轨道后音符应清零");
    // 删除后原 track 2..N 各自前移 1，track_note_count 长度也少了 1
    assert_eq!(doc.model().tracks.len(), 16, "轨道数应减 1");

    // Undo：必须恢复被删轨道上的音符
    assert!(doc.undo(), "undo 应成功");
    assert_eq!(doc.model().tracks.len(), 17, "undo 后轨道数应恢复");
    assert_eq!(
        doc.model().note_count,
        2,
        "undo 后音符数必须恢复（这是 bug 修复点）"
    );
    assert_eq!(doc.model().track_note_count[1], 2, "track 1 的音符必须恢复");
    // 具体音符内容也要核对（start_tick + key）
    assert_eq!(doc.model().notes[60].len(), 1);
    assert_eq!(doc.model().notes[60][0].start_tick, 0);
    assert_eq!(doc.model().notes[64].len(), 1);
    assert_eq!(doc.model().notes[64][0].start_tick, 480);

    // Redo：再次删除，音符应再次清零
    assert!(doc.redo(), "redo 应成功");
    assert_eq!(doc.model().tracks.len(), 16);
    assert_eq!(doc.model().note_count, 0, "redo 后音符应再次清零");
}

/// 点击历史行跳转：前后连续 undo/redo，future 必须保留；
/// 之后再产生新编辑，future 才被覆盖（标准 undo 分支语义）。
#[test]
fn jump_history_preserves_future_until_new_edit() {
    use crate::history::{EditSnapshot, UndoEntry};
    use yinhe_core::NoteEvent;

    let mut doc = Document::empty();
    for (i, key) in [60u8, 62, 64].into_iter().enumerate() {
        let note = NoteEvent {
            id: 0,
            start_tick: i as u32 * 480,
            end_tick: i as u32 * 480 + 120,
            key,
            velocity: 100,
        };
        let action = doc.add_note(1, note).expect("add_note");
        doc.history.push(UndoEntry {
            action,
            label: format!("step{}", i + 1),
            snapshot: EditSnapshot::default(),
        });
    }
    assert_eq!(doc.history.past_len(), 3);
    assert_eq!(doc.model().note_count, 3);

    // 回到第一步之后：后面两步进入 future，但未丢失。
    assert!(doc.jump_history_to(1));
    assert_eq!(doc.history.past_len(), 1);
    assert_eq!(doc.redo_labels().len(), 2);
    assert_eq!(doc.model().note_count, 1);

    // 再跳到最后一步：future 被依次 redo 回来。
    assert!(doc.jump_history_to(3));
    assert_eq!(doc.history.past_len(), 3);
    assert!(doc.redo_labels().is_empty());
    assert_eq!(doc.model().note_count, 3);

    // 回退后做新编辑：旧 future 分支被覆盖。
    assert!(doc.jump_history_to(1));
    assert_eq!(doc.redo_labels().len(), 2);
    let note = NoteEvent {
        id: 0,
        start_tick: 1440,
        end_tick: 1560,
        key: 67,
        velocity: 100,
    };
    let action = doc.add_note(1, note).expect("add_note");
    doc.history.push(UndoEntry {
        action,
        label: "new_branch".to_string(),
        snapshot: EditSnapshot::default(),
    });
    assert!(doc.redo_labels().is_empty(), "新编辑必须覆盖旧 future 分支");
}

/// 回归测试：跨轨拖 automation 被 clamp 回原轨时事件不能蒸发。
/// 之前 arrange_move.rs:157 在 dst==src 时 continue，但 phase 1 已把
/// 被拖事件从源 lane 剔除，导致事件彻底丢失。修复后会把事件加回源 lane。
#[test]
fn arrange_move_automation_clamped_to_source_preserves_events() {
    use crate::history::{EditSnapshot, UndoEntry};
    use yinhe_core::Selection;
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    let mut doc = Document::empty();
    // conductor 在 track 0，给 track 1 加一条 CC7 lane，含 2 个事件
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 1,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 64.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 480,
                    value: 80.0,
                    shape: SegmentShape::Step,
                },
            ],
        });
    }

    // 选区覆盖 track 1 的 tick 0..481（半开区间，包含 tick 0 和 480 两个事件）
    doc.edit.selected = Selection::default();
    doc.edit
        .selected
        .add_rect_track(0, 481, 0, yinhe_types::MAX_KEY, 1, 1);

    // 跨轨上移 1：raw_dst=0 是 conductor，被 clamp 回 track 1
    let action = doc
        .move_selected_arrange(100, -1)
        .expect("move_selected_arrange should return an action");
    doc.history.push(UndoEntry {
        action,
        label: "arrange_move".to_string(),
        snapshot: EditSnapshot::default(),
    });

    // 关键断言：事件不能蒸发，应该在 track 1 的 CC7 lane 里
    let lane = &doc.model().tracks[1].automation_lanes[0];
    assert_eq!(
        lane.events.len(),
        2,
        "事件数量必须保持 2（bug 修复点：之前变 0）"
    );
    // delta_ticks=100 已应用
    assert_eq!(lane.events[0].tick, 100, "第一个事件 tick 应偏移 +100");
    assert_eq!(lane.events[1].tick, 580, "第二个事件 tick 应偏移 +100");

    // Undo：事件回到原 tick
    assert!(doc.undo());
    let lane = &doc.model().tracks[1].automation_lanes[0];
    assert_eq!(lane.events.len(), 2, "undo 后事件数量仍为 2");
    assert_eq!(lane.events[0].tick, 0, "undo 后第一个事件回到 tick 0");
    assert_eq!(lane.events[1].tick, 480, "undo 后第二个事件回到 tick 480");

    // Redo：事件再次偏移
    assert!(doc.redo());
    let lane = &doc.model().tracks[1].automation_lanes[0];
    assert_eq!(lane.events.len(), 2, "redo 后事件数量仍为 2");
    assert_eq!(lane.events[0].tick, 100);
    assert_eq!(lane.events[1].tick, 580);
}

/// AR Alt+拖动复制：原音符/原自动化事件保留，副本平移到新位置，选区跟随副本。
#[test]
fn arrange_duplicate_preserves_originals_and_offsets_copy() {
    use crate::history::UndoEntry;
    use yinhe_core::Selection;
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    let mut doc = Document::empty();
    // track 1 放一个音符 + 一条 CC7 lane（2 个事件）
    doc.add_note(
        1,
        yinhe_core::NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        },
    );
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 1,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 64.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
                    tick: 480,
                    value: 80.0,
                    shape: SegmentShape::Step,
                },
            ],
        });
    }
    doc.edit.selected = Selection::default();
    doc.edit.selected.add_rect_track(0, 481, 60, 60, 1, 1);
    let before = doc.capture_snapshot();

    let action = doc
        .duplicate_selected_arrange(480, 0)
        .expect("duplicate_selected_arrange should return an action");
    doc.history.push(UndoEntry {
        action,
        label: "duplicate_arrange".to_string(),
        snapshot: before,
    });

    // 原音符保留（[0,480) 只含原件），副本偏移 +480
    let orig: Vec<_> = doc.model().notes[60].range(0, 480).collect();
    assert_eq!(orig.len(), 1, "原音符应保留");
    let copy: Vec<_> = doc.model().notes[60].range(480, 961).collect();
    assert_eq!(copy.len(), 1, "副本应平移到 +480");
    assert_eq!(copy[0].start_tick, 480);
    assert_eq!(copy[0].end_tick, 960);
    assert_ne!(copy[0].id, orig[0].id, "副本应分配新 id");

    // 原自动化事件保留，副本偏移 +480（同轨追加到同一 lane）
    let lane = &doc.model().tracks[1].automation_lanes[0];
    assert_eq!(lane.events.len(), 4, "原 2 个 + 副本 2 个");
    assert_eq!(lane.events[0].tick, 0);
    assert_eq!(lane.events[1].tick, 480);
    assert_eq!(
        lane.events[2].tick, 480,
        "副本第一个事件（与原件同 tick 边界）"
    );
    assert_eq!(lane.events[3].tick, 960, "副本第二个事件");

    // 选区跟随副本
    assert_eq!(doc.edit.selected.rects.len(), 1);
    let (ts, te, ..) = doc.edit.selected.rects[0];
    assert_eq!((ts, te), (480, 961), "选区应平移到副本范围");

    // Undo：音符和事件全部回到复制前
    assert!(doc.undo());
    let copy: Vec<_> = doc.model().notes[60].range(480, 961).collect();
    assert_eq!(copy.len(), 0, "undo 后副本音符应消失");
    let lane = &doc.model().tracks[1].automation_lanes[0];
    assert_eq!(lane.events.len(), 2, "undo 后事件回到 2 个");
    assert_eq!(lane.events[0].tick, 0);
    assert_eq!(lane.events[1].tick, 480);
}

/// 跨轨复制被 clamp 回原轨时副本不能蒸发（与 move 的回归点对称）。
#[test]
fn arrange_duplicate_clamped_to_source_preserves_copy() {
    use yinhe_core::Selection;
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    let mut doc = Document::empty();
    // track 1 的 CC7 lane 含 1 个事件，向上跨轨复制（raw_dst=0 是 conductor）
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 1,
            events: vec![AutomationEvent {
                id: 0,
                tick: 0,
                value: 64.0,
                shape: SegmentShape::Step,
            }],
        });
    }
    doc.edit.selected = Selection::default();
    doc.edit
        .selected
        .add_rect_track(0, 100, 0, yinhe_types::MAX_KEY, 1, 1);

    let action = doc
        .duplicate_selected_arrange(100, -1)
        .expect("duplicate_selected_arrange should return an action");
    assert!(matches!(action, crate::history::UndoAction::Automation(_)));

    // 原件 + 被夹回的副本都在 track 1（事件不能蒸发）
    let lane = &doc.model().tracks[1].automation_lanes[0];
    assert_eq!(lane.events.len(), 2, "原件 1 个 + 副本 1 个");
    assert_eq!(lane.events[0].tick, 0, "原件不动");
    assert_eq!(lane.events[1].tick, 100, "副本偏移 +100");
}

/// 回归测试：Conductor 颜色必须跟随主文字 `text_primary`（而非固定白/黑），
/// 且 `sync_track_caches_with_conductor_color` 能精确覆盖缓存使曲线及时刷新
#[test]
fn conductor_sync_with_text_primary_changes_cache() {
    let mut doc = Document::empty();
    let dark_c = yinhe_theme::base::BaseColors::DARK.text;
    let dark_f = [
        dark_c.r as f32 / 255.0,
        dark_c.g as f32 / 255.0,
        dark_c.b as f32 / 255.0,
        1.0,
    ];
    assert_eq!(
        doc.edit.track_cache.colors[0], dark_f,
        "新建文档 Conductor 应为暗色主文字"
    );
    let light_c = yinhe_theme::base::BaseColors::LIGHT.text;
    let light_f = [
        light_c.r as f32 / 255.0,
        light_c.g as f32 / 255.0,
        light_c.b as f32 / 255.0,
        1.0,
    ];
    doc.sync_track_caches_with_conductor_color(light_f);
    assert_eq!(
        doc.edit.track_cache.colors[0], light_f,
        "切换主题后缓存应跟随主文字"
    );
    let custom = [0.345, 0.305, 0.235, 1.0];
    doc.sync_track_caches_with_conductor_color(custom);
    assert_eq!(doc.edit.track_cache.colors[0], custom);
    // 非 Conductor 轨道不受 Conductor 颜色影响，仍走调色板
    assert_eq!(
        doc.edit.track_cache.colors[1],
        [
            yinhe_theme::palette::TRACK_PALETTE[0][0],
            yinhe_theme::palette::TRACK_PALETTE[0][1],
            yinhe_theme::palette::TRACK_PALETTE[0][2],
            1.0
        ]
    );
}
