use std::sync::Arc;

use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};

use crate::document::Document;

use super::*;
use yinhe_core::Selection;

fn make_doc(name: &str) -> Document {
    let model = YinModel {
        conductor: Arc::new(ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: vec![TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            }],
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        }),
        tracks: vec![Arc::new({
            let mut t = TrackData::new(0, 0);
            t.name = name.to_string();
            t
        })],
        ..Default::default()
    };
    Document {
        data: crate::project_data::ProjectData::new(
            Arc::new(model),
            Default::default(),
            Default::default(),
        ),
        edit: crate::edit_state::EditState {
            track_visible: vec![true],
            track_pianoroll_visible: vec![true],
            ..Default::default()
        },
        history: UndoStack::new(),
        file_name: "test".into(),
        file_path: None,
    }
}

#[test]
fn push_stores_and_clears_redo() {
    let mut doc = make_doc("a");
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "a".into(),
            new: "b".into(),
        },
        label: "rename".to_string(),
        snapshot: EditSnapshot::default(),
    });
    assert!(doc.history.can_undo());
    assert!(!doc.history.can_redo());

    doc.undo();
    assert!(!doc.history.can_undo());
    assert!(doc.history.can_redo());

    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "c".into(),
            new: "d".into(),
        },
        label: "rename2".to_string(),
        snapshot: EditSnapshot::default(),
    });
    assert!(!doc.history.can_redo());
    assert!(doc.history.can_undo());
}

#[test]
fn undo_restores_track_name() {
    let mut doc = make_doc("old");
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "old".into(),
            new: "new".into(),
        },
        label: "rename".to_string(),
        snapshot: EditSnapshot::default(),
    });
    // Apply the forward action manually (simulating the edit)
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[0]);
        track.name = "new".into();
    }
    assert_eq!(doc.data.model.tracks[0].name, "new");

    // Undo
    assert!(doc.undo());
    assert_eq!(doc.data.model.tracks[0].name, "old");
    assert!(doc.history.can_redo());

    // Redo
    assert!(doc.redo());
    assert_eq!(doc.data.model.tracks[0].name, "new");
    assert!(doc.history.can_undo());
}

#[test]
fn track_color_undo_redo() {
    let mut doc = make_doc("t");
    doc.edit.track_colors_cache = vec![[0.1, 0.2, 0.3, 1.0]];
    doc.history.push(UndoEntry {
        action: UndoAction::TrackColor {
            track_idx: 0,
            old: [0.1, 0.2, 0.3, 1.0],
            new: [0.4, 0.5, 0.6, 0.5],
        },
        label: "recolor".to_string(),
        snapshot: EditSnapshot::default(),
    });
    // Apply the forward action manually (simulating the edit)
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[0]);
        track.color = [0.4, 0.5, 0.6, 0.5];
    }
    doc.edit.track_colors_cache[0] = [0.4, 0.5, 0.6, 0.5];
    assert_eq!(doc.data.model.tracks[0].color, [0.4, 0.5, 0.6, 0.5]);

    // Undo：颜色与显示缓存一起恢复
    assert!(doc.undo());
    assert_eq!(doc.data.model.tracks[0].color, [0.1, 0.2, 0.3, 1.0]);
    assert_eq!(doc.edit.track_colors_cache[0], [0.1, 0.2, 0.3, 1.0]);
    assert!(doc.history.can_redo());

    // Redo
    assert!(doc.redo());
    assert_eq!(doc.data.model.tracks[0].color, [0.4, 0.5, 0.6, 0.5]);
    assert_eq!(doc.edit.track_colors_cache[0], [0.4, 0.5, 0.6, 0.5]);
    assert!(doc.history.can_undo());
}

#[test]
fn note_delta_undo_redo() {
    let mut doc = make_doc("test");
    // Add a note
    let note = NoteEvent {
        id: 0,
        start_tick: 0,
        end_tick: 480,
        key: 60,
        velocity: 100,
    };
    let key = 60;
    {
        let model = Arc::make_mut(&mut doc.data.model);
        Arc::make_mut(&mut model.notes[key as usize]).insert_sorted(yinhe_types::Note {
            id: 0,
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            velocity: note.velocity,
            track: 0,
        });
        model.mark_dirty(key);
        model.rebuild_dirty();
    }

    let removed = {
        let model = Arc::make_mut(&mut doc.data.model);
        let mut sel = Selection::default();
        sel.add_rect_track(0, 480, 60, 60, 0, u16::MAX);
        let r = crate::batch_ops::remove_selected(model, &sel);
        model.rebuild_dirty();
        r
    };
    assert_eq!(removed.len(), 1);

    doc.history.push(UndoEntry {
        action: UndoAction::Notes(NoteDelta {
            before: removed,
            after: vec![],
        }),
        label: "delete".to_string(),
        snapshot: EditSnapshot::default(),
    });

    // Note should be gone
    assert!(doc.data.model.notes[60].is_empty());

    // Undo
    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[60].len(), 1);
    assert_eq!(doc.data.model.notes[60][0].start_tick, 0);

    // Redo
    assert!(doc.redo());
    assert!(doc.data.model.notes[60].is_empty());
}

#[test]
fn undo_returns_none_when_empty() {
    let mut doc = make_doc("x");
    assert!(!doc.undo());
}

#[test]
fn redo_returns_none_when_empty() {
    let mut doc = make_doc("x");
    assert!(!doc.redo());
}

#[test]
fn clear_wipes_everything() {
    let mut doc = make_doc("a");
    doc.history.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx: 0,
            old: "a".into(),
            new: "b".into(),
        },
        label: "rename".to_string(),
        snapshot: EditSnapshot::default(),
    });
    doc.undo();
    assert!(doc.history.can_undo() || doc.history.can_redo());

    doc.history.clear();
    assert!(!doc.history.can_undo());
    assert!(!doc.history.can_redo());
    assert_eq!(doc.history.past.len(), 0);
    assert_eq!(doc.history.future.len(), 0);
}

// ---------------------------------------------------------------------------
// Undo/redo round-trip：操作 + undo 后模型必须逐字段恢复原状（redo 同理）
// ---------------------------------------------------------------------------

/// 模型内容快照（逐字段，用于 undo/redo 精确比较）。
/// 不含 `next_note_id`：发号器只增不减，undo 不恢复它是设计如此。
#[derive(Clone, PartialEq, Debug, Default)]
struct ModelSnapshot {
    /// 每条 lane 的事件：(track_idx, lane_idx, tick, value_bits, shape)
    events: Vec<(usize, usize, u32, u32, u32)>,
    /// conductor tempo：(tick, value_bits, shape)
    tempo: Vec<(u32, u32, u32)>,
    /// time_sig：(tick, numerator, denominator)
    time_sig: Vec<(u32, u8, u8)>,
    /// 音符：(key, id, start_tick, end_tick, velocity, track)
    notes: Vec<(u8, u32, u32, u32, u8, u16)>,
}

fn shape_tag(shape: SegmentShape) -> u32 {
    match shape {
        SegmentShape::Step => 0,
        SegmentShape::Curve { .. } => 1,
    }
}

fn model_snapshot(model: &YinModel) -> ModelSnapshot {
    let mut events = Vec::new();
    for (ti, track) in model.tracks.iter().enumerate() {
        for (li, lane) in track.automation_lanes.iter().enumerate() {
            for e in &lane.events {
                events.push((ti, li, e.tick, e.value.to_bits(), shape_tag(e.shape)));
            }
        }
    }
    let tempo = model
        .conductor
        .tempo
        .events
        .iter()
        .map(|e| (e.tick, e.value.to_bits(), shape_tag(e.shape)))
        .collect();
    let time_sig = model
        .conductor
        .time_sig
        .iter()
        .map(|e| (e.tick, e.numerator, e.denominator))
        .collect();
    let mut notes: Vec<_> = (0..128u8)
        .flat_map(|k| {
            model.notes[k as usize]
                .iter()
                .map(move |n| (k, n.id, n.start_tick, n.end_tick, n.velocity, n.track))
        })
        .collect();
    notes.sort();
    ModelSnapshot {
        events,
        tempo,
        time_sig,
        notes,
    }
}

/// 断言操作满足 undo/redo 完全往返：undo 后 = 操作前，redo 后 = 操作后。
fn assert_roundtrip(doc: &mut Document, label: &str, action: impl FnOnce(&mut Document)) {
    let before = model_snapshot(&doc.data.model);
    action(doc);
    let after = model_snapshot(&doc.data.model);
    assert_ne!(before, after, "{label}: 操作必须改变模型");
    assert!(doc.undo(), "{label}: undo 应成功");
    assert_eq!(
        model_snapshot(&doc.data.model),
        before,
        "{label}: undo 后必须逐字段恢复"
    );
    assert!(doc.redo(), "{label}: redo 应成功");
    assert_eq!(
        model_snapshot(&doc.data.model),
        after,
        "{label}: redo 后必须逐字段恢复"
    );
}

/// 带一条 CC lane（tick 100/200/300）的 doc，供 automation round-trip 用。
fn make_doc_with_cc_lane() -> Document {
    let mut doc = make_doc("cc");
    let model = Arc::make_mut(&mut doc.data.model);
    let track = Arc::make_mut(&mut model.tracks[0]);
    track.automation_lanes.push(AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 0,
        events: vec![
            AutomationEvent {
                tick: 100,
                value: 64.0,
                shape: SegmentShape::Step,
            },
            AutomationEvent {
                tick: 200,
                value: 32.0,
                shape: SegmentShape::Step,
            },
            AutomationEvent {
                tick: 300,
                value: 96.0,
                shape: SegmentShape::linear_curve(),
            },
        ],
    });
    doc
}

#[test]
fn automation_add_roundtrips() {
    let mut doc = make_doc_with_cc_lane();
    let target = AutomationTarget::CC { controller: 7 };
    assert_roundtrip(&mut doc, "add", |doc| {
        let action = doc
            .add_automation_event(
                0,
                target.clone(),
                AutomationEvent {
                    tick: 150,
                    value: 50.0,
                    shape: SegmentShape::Step,
                },
            )
            .map(|(_, _, a)| a);
        if let Some(a) = action {
            doc.push_undo(a, "add", doc.capture_snapshot());
        }
    });
    // 增量断言：新增事件的 delta 只含该事件，不是整个 lane。
    let action = doc
        .add_automation_event(
            0,
            target,
            AutomationEvent {
                tick: 250,
                value: 70.0,
                shape: SegmentShape::Step,
            },
        )
        .map(|(_, _, a)| a)
        .expect("add 应成功");
    match action {
        UndoAction::Automation(delta) => {
            assert_eq!(delta.before.len(), 0);
            assert_eq!(delta.after.len(), 1);
            assert_eq!(delta.after[0].tick, 250);
        }
        _ => panic!("应为 Automation delta"),
    }
}

#[test]
fn automation_add_tempo_roundtrips() {
    let mut doc = make_doc("tempo");
    assert_roundtrip(&mut doc, "add-tempo", |doc| {
        let action = doc
            .add_automation_event(
                0,
                AutomationTarget::Tempo,
                AutomationEvent {
                    tick: 480,
                    value: 140.0,
                    shape: SegmentShape::Step,
                },
            )
            .map(|(_, _, a)| a);
        if let Some(a) = action {
            doc.push_undo(a, "add-tempo", doc.capture_snapshot());
        }
    });
}

#[test]
fn automation_move_roundtrips_including_conflict() {
    let mut doc = make_doc_with_cc_lane();
    // tick 200 → 300：tick 300 处已有事件（冲突项）会被移除，undo 必须恢复它。
    assert_roundtrip(&mut doc, "move-with-conflict", |doc| {
        let action = doc.move_automation_event(
            0,
            0,
            &AutomationTarget::CC { controller: 7 },
            200,
            300,
            88.0,
        );
        if let Some(a) = action {
            doc.push_undo(a, "move", doc.capture_snapshot());
        }
    });
}

#[test]
fn automation_move_value_only_roundtrips() {
    let mut doc = make_doc_with_cc_lane();
    // 只改 value（tick 不变）
    assert_roundtrip(&mut doc, "move-value-only", |doc| {
        let action = doc.move_automation_event(
            0,
            0,
            &AutomationTarget::CC { controller: 7 },
            200,
            200,
            10.0,
        );
        if let Some(a) = action {
            doc.push_undo(a, "move-value", doc.capture_snapshot());
        }
    });
}

#[test]
fn automation_batch_move_roundtrips_including_conflict() {
    let mut doc = make_doc_with_cc_lane();
    // 批量：100→150、200→300（300 处冲突）、300→400
    assert_roundtrip(&mut doc, "batch-move", |doc| {
        let moves = vec![(100, 150, 10.0), (200, 300, 20.0), (300, 400, 30.0)];
        let action =
            doc.move_automation_events_batch(0, 0, &AutomationTarget::CC { controller: 7 }, &moves);
        if let Some(a) = action {
            doc.push_undo(a, "batch-move", doc.capture_snapshot());
        }
    });
}

#[test]
fn automation_delete_roundtrips() {
    let mut doc = make_doc_with_cc_lane();
    assert_roundtrip(&mut doc, "delete", |doc| {
        let action =
            doc.delete_automation_event(0, 0, &AutomationTarget::CC { controller: 7 }, 200);
        if let Some(a) = action {
            doc.push_undo(a, "delete", doc.capture_snapshot());
        }
    });
    // 增量断言：delta 只含被删事件。
    let action = doc
        .delete_automation_event(0, 0, &AutomationTarget::CC { controller: 7 }, 100)
        .expect("delete 应成功");
    match action {
        UndoAction::Automation(delta) => {
            assert_eq!(delta.before.len(), 1);
            assert_eq!(delta.before[0].tick, 100);
            assert_eq!(delta.after.len(), 0);
        }
        _ => panic!("应为 Automation delta"),
    }
}

#[test]
fn automation_shape_roundtrips() {
    let mut doc = make_doc_with_cc_lane();
    assert_roundtrip(&mut doc, "shape", |doc| {
        let action = doc.set_automation_shape(
            0,
            0,
            &AutomationTarget::CC { controller: 7 },
            200,
            SegmentShape::linear_curve(),
        );
        if let Some(a) = action {
            doc.push_undo(a, "shape", doc.capture_snapshot());
        }
    });
}

#[test]
fn automation_mixed_sequence_roundtrips() {
    // 连续多操作（add → move → delete → shape）逐个 undo，栈序回放。
    let mut doc = make_doc_with_cc_lane();
    let target = AutomationTarget::CC { controller: 7 };
    let mut expected = vec![model_snapshot(&doc.data.model)];
    let mut apply = |doc: &mut Document, label: &str, action: Option<UndoAction>| {
        if let Some(a) = action {
            doc.push_undo(a, label, doc.capture_snapshot());
            expected.push(model_snapshot(&doc.data.model));
        }
    };
    let a1 = doc
        .add_automation_event(
            0,
            target.clone(),
            AutomationEvent {
                tick: 150,
                value: 50.0,
                shape: SegmentShape::Step,
            },
        )
        .map(|(_, _, a)| a);
    apply(&mut doc, "add", a1);
    let a2 = doc.move_automation_event(0, 0, &target, 150, 250, 60.0);
    apply(&mut doc, "move", a2);
    let a3 = doc.delete_automation_event(0, 0, &target, 250);
    apply(&mut doc, "delete", a3);
    let a4 = doc.set_automation_shape(0, 0, &target, 100, SegmentShape::linear_curve());
    apply(&mut doc, "shape", a4);

    // 逐个 undo：每一步都精确回到前一个快照。
    let final_state = expected.last().unwrap().clone();
    expected.pop(); // 当前终态，无需验证
    while let Some(prev) = expected.pop() {
        assert!(doc.undo(), "逐步 undo 应成功");
        assert_eq!(
            model_snapshot(&doc.data.model),
            prev,
            "逐步 undo 快照精确恢复"
        );
    }
    assert!(!doc.history.can_undo());
    // 逐个 redo：回到所有操作后的终态。
    while doc.redo() {}
    assert_eq!(
        model_snapshot(&doc.data.model),
        final_state,
        "全部 redo 后回到终态"
    );
}
