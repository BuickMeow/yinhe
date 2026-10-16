use super::*;

#[test]
fn apply_note_field_edit_velocity_add() {
    let mut doc = make_doc_with_note();
    let ops = crate::num_expr::parse_num_expr("+5").unwrap();
    let action = doc
        .apply_note_field_edit(NoteField::Velocity, &ops)
        .expect("should edit");
    assert_eq!(doc.data.model.notes[60][0].velocity, 105);
    match action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 1);
            assert_eq!(delta.before[0].0.velocity, 100);
            assert_eq!(delta.after[0].0.velocity, 105);
        }
        _ => panic!("expected Notes"),
    }
}

#[test]
fn apply_note_field_edit_velocity_clamp_127() {
    let mut doc = make_doc_with_note();
    let ops = crate::num_expr::parse_num_expr("x2").unwrap();
    doc.apply_note_field_edit(NoteField::Velocity, &ops);
    assert_eq!(doc.data.model.notes[60][0].velocity, 127);
}

#[test]
fn apply_note_field_edit_gate_add_follows_rect() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    let ops = crate::num_expr::parse_num_expr("+10").unwrap();
    doc.apply_note_field_edit(NoteField::Gate, &ops);
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.end_tick - n.start_tick, 110);
    // 选框 te 跟随 +10，ts 不动
    assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 211.0, 60, 60));
    assert_eq!(doc.edit.selected.rects[0].1, 211);
}

#[test]
fn apply_note_field_edit_tick_add_moves_rects() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    doc.edit.arr_sel_rect = vec![(100.0, 201.0, 0, 0)];
    let ops = crate::num_expr::parse_num_expr("+50").unwrap();
    doc.apply_note_field_edit(NoteField::Tick, &ops);
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 150);
    assert_eq!(n.end_tick, 250);
    // PR + AR 选框 tick 平移
    assert_eq!(doc.edit.sel_rect.rects[0], (150.0, 251.0, 60, 60));
    assert_eq!(doc.edit.arr_sel_rect[0], (150.0, 251.0, 0, 0));
    assert_eq!(doc.edit.selected.rects[0].0, 150);
}

#[test]
fn apply_note_field_edit_key_add_moves_bucket_and_rect() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    let ops = crate::num_expr::parse_num_expr("+2").unwrap();
    doc.apply_note_field_edit(NoteField::Key, &ops);
    assert!(doc.data.model.notes[60].is_empty());
    assert_eq!(doc.data.model.notes[62].len(), 1);
    // 选框 key 平移
    assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 201.0, 62, 62));
    assert_eq!(doc.edit.selected.rects[0].2, 62);
}

#[test]
fn apply_note_field_edit_tick_mul_follows_single_uniform() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    let ops = crate::num_expr::parse_num_expr("x2").unwrap();
    doc.apply_note_field_edit(NoteField::Tick, &ops);
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 200);
    assert_eq!(n.end_tick, 300);
    // 单音符 delta 一致（+100）→ 选框跟随
    assert_eq!(doc.edit.sel_rect.rects[0], (200.0, 301.0, 60, 60));
}

#[test]
fn apply_note_field_edit_empty_selection_returns_none() {
    let mut doc = make_doc_with_note();
    doc.edit.selected.clear();
    let ops = crate::num_expr::parse_num_expr("+10").unwrap();
    assert!(
        doc.apply_note_field_edit(NoteField::Velocity, &ops)
            .is_none()
    );
}

#[test]
fn apply_note_field_edit_velocity_remembers_latest_tick() {
    let mut doc = make_doc_with_note();
    doc.add_note(
        0,
        yinhe_core::NoteEvent {
            id: 0,
            start_tick: 200,
            end_tick: 300,
            key: 60,
            velocity: 90,
        },
    );
    // 选区覆盖 t100 与 t200 两个音符
    doc.edit.selected.add_rect_track(100, 300, 60, 60, 0, 0);
    let ops = crate::num_expr::parse_num_expr("60").unwrap(); // 赋值 60
    assert!(
        doc.apply_note_field_edit(NoteField::Velocity, &ops)
            .is_some()
    );
    // 记录时间最晚（t200）的 60
    assert_eq!(doc.edit.default_velocity(0), 60);
}

/// 回归：多个重叠 rect 命中同一音符时，单趟原地路径不重复收集
///（旧实现按 rect×key 遍历会 push 重复项，undo 数据膨胀）。
#[test]
fn apply_note_field_edit_overlapping_rects_dedup() {
    let mut doc = make_doc_with_note(); // key 60, t100-200, vel 100
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 300, 60, 60, 0, 0);
    doc.edit.selected.add_rect_track(100, 400, 60, 60, 0, 0);
    let ops = crate::num_expr::parse_num_expr("50").unwrap(); // 赋值 50
    let action = doc
        .apply_note_field_edit(NoteField::Velocity, &ops)
        .expect("应命中");
    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 1, "重叠 rect 不应重复收集");
            assert_eq!(delta.after.len(), 1);
            assert_eq!(delta.after[0].0.velocity, 50);
        }
        other => panic!("expected Notes, got {other:?}"),
    }
    let snap = doc.capture_snapshot();
    doc.push_undo(action, "vel-dedup", snap);
    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[60][0].velocity, 100);
    assert!(doc.redo());
    assert_eq!(doc.data.model.notes[60][0].velocity, 50);
}

#[test]
fn apply_note_field_edit_gate_remembers_latest_tick() {
    let mut doc = make_doc_with_note(); // t100 gate 100
    doc.add_note(
        0,
        yinhe_core::NoteEvent {
            id: 0,
            start_tick: 300,
            end_tick: 500,
            key: 60,
            velocity: 90,
        },
    ); // t300 gate 200
    // 选区覆盖两者
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 501, 60, 60, 0, 0);
    // 批量改 gate：赋值 240
    let ops = crate::num_expr::parse_num_expr("240").unwrap();
    assert!(doc.apply_note_field_edit(NoteField::Gate, &ops).is_some());
    // 记录时间最晚（t300）的 240，且无记忆时回退 120
    assert_eq!(doc.edit.default_gate(0, 120), 240);
    assert_eq!(doc.edit.default_gate(1, 120), 120);
    // 未命中不覆盖
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(9999, 10000, 60, 60, 0, 0);
    let ops2 = crate::num_expr::parse_num_expr("10").unwrap();
    assert!(doc.apply_note_field_edit(NoteField::Gate, &ops2).is_none());
    assert_eq!(doc.edit.default_gate(0, 120), 240);
}
