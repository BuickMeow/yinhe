use super::*;

/// 「允许新重叠音符」默认开：重叠新音符照常添加（保持现状行为）。
#[test]
fn add_note_allows_overlap_by_default() {
    let mut doc = make_doc_with_note(); // 已有 [100,200) k60
    assert!(
        doc.add_note(0, ev(150, 250, 60)).is_some(),
        "默认应允许重叠"
    );
    assert_eq!(doc.data.model.notes[60].len(), 2);
}

/// 开关关闭时 add_note 整个拒绝重叠新音符；相接/跨 key 不算重叠。
#[test]
fn add_note_rejects_overlap_when_disallowed() {
    let mut doc = make_doc_with_note(); // 已有 [100,200) k60
    doc.edit.allow_overlapping_notes = false;

    // 相交 → 无视（返回 None，模型不变，也不消耗 id）
    assert!(
        doc.add_note(0, ev(150, 250, 60)).is_none(),
        "重叠新音符应被无视"
    );
    assert_eq!(doc.data.model.notes[60].len(), 1);

    // 首尾相接（左闭右开区间）→ 不算重叠，放行
    assert!(doc.add_note(0, ev(200, 300, 60)).is_some(), "相接不算重叠");
    // 跨 key → 不同桶，放行
    assert!(
        doc.add_note(0, ev(150, 250, 61)).is_some(),
        "跨 key 不算重叠"
    );
    // 不重叠 → 放行
    assert!(doc.add_note(0, ev(400, 500, 60)).is_some());
}

#[test]
fn set_note_end_tick_updates_gate_and_undo() {
    let mut doc = make_doc_with_note();
    let action = doc
        .add_note(
            0,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 1000,
                end_tick: 1001,
                key: 62,
                velocity: 80,
            },
        )
        .expect("add_note 应成功");
    let note_id = match &action {
        crate::history::UndoAction::Notes(d) => d.after[0].0.id,
        other => panic!("unexpected action {other:?}"),
    };

    // NoteOff 闭合 gate
    let before = doc.capture_snapshot();
    let update = doc
        .set_note_end_tick(62, note_id, 1480)
        .expect("set_note_end_tick 应成功");
    doc.push_undo(update, "record", before);
    let n = doc.data.model.notes[62][0];
    assert_eq!(n.end_tick, 1480);

    // undo 恢复原 gate
    assert!(doc.undo(), "undo 应成功");
    let n = doc.data.model.notes[62][0];
    assert_eq!(n.end_tick, 1001);
}

#[test]
fn set_note_end_tick_clamps_below_start() {
    let mut doc = make_doc_with_note();
    let action = doc
        .add_note(
            0,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 1000,
                end_tick: 2000,
                key: 62,
                velocity: 80,
            },
        )
        .expect("add_note 应成功");
    let note_id = match &action {
        crate::history::UndoAction::Notes(d) => d.after[0].0.id,
        other => panic!("unexpected action {other:?}"),
    };
    // 快速弹放：end_tick == start_tick → 钳制为 start+1（仍返回 Some）
    assert!(doc.set_note_end_tick(62, note_id, 1000).is_some());
    let n = doc.data.model.notes[62][0];
    assert_eq!(n.end_tick, 1001);
}

/// undo/redo 回放路径（apply NoteDelta）绝不被重叠检查拦截：
/// 开关开着时制造重叠，关掉后 undo/redo 照常重放。
#[test]
fn undo_redo_replay_not_blocked_by_overlap_check() {
    let mut doc = make_doc_with_note(); // A [100,200) k60，默认允许重叠
    let before_snap = doc.capture_snapshot();
    let action = doc.add_note(0, ev(150, 250, 60)).expect("允许重叠时应成功");
    doc.push_undo(action, "add", before_snap);
    assert_eq!(doc.data.model.notes[60].len(), 2);

    doc.edit.allow_overlapping_notes = false;
    assert!(doc.undo(), "undo 应成功");
    assert_eq!(doc.data.model.notes[60].len(), 1);
    assert!(doc.redo(), "redo 应成功（即使重新制造重叠）");
    assert_eq!(doc.data.model.notes[60].len(), 2);
}
