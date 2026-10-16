use crate::batch_ops;
use crate::edit_state::ResizeSide;

use super::*;

/// 回归：矩形态选区（全选/轨道选择等）移动后，成员集物化为「实际被移动
/// 的音符」——移动后的矩形若覆盖落点路人，路人不得被高亮（旧实现只平移
/// 矩形，矩形近似会把路人算作选中）。
#[test]
fn move_from_rect_selection_materializes_moved_ids() {
    let mut doc = make_doc_with_note(); // A: tick 100-200 key60
    // 落点路人 B：同 key，起点 350（A 移动 +150 后为 250-350，矩形 [250,351)
    // 恰好覆盖 B 的起点，但两者相接不算重叠 → 走快速路径）。
    doc.add_note(
        0,
        NoteEvent {
            id: 0,
            start_tick: 350,
            end_tick: 450,
            key: 60,
            velocity: 100,
        },
    );
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
    assert!(
        !doc.edit.selected.has_explicit_members(),
        "前提：矩形态选区"
    );

    doc.move_selected_notes(150, 0).expect("移动应产生 undo");
    assert!(
        doc.edit.selected.has_explicit_members(),
        "矩形态移动后应物化成员位图"
    );
    let collected = batch_ops::collect_selected(&doc.data.model, &doc.edit.selected);
    assert_eq!(
        collected.len(),
        1,
        "成员只含被移动的 A，不含矩形覆盖的落点路人 B：{collected:?}"
    );
    assert_eq!(collected[0].0.start_tick, 250, "A 应移到 250");
}

/// 回归：重叠选框下同一音符只移动一次（位移小、移动后仍落在另一 rect
/// 内时，曾按 rect 重复命中导致两倍位移）。
#[test]
fn overlapping_rects_move_applies_once() {
    let mut doc = make_doc_with_note();
    // 追加一个与原选区重叠的 rect，两 rect 都覆盖 tick=100 的音符；
    // 移动 +10 后 110 仍落在 [50,150) 内（重复命中场景）。
    doc.edit.selected.add_rect_track(50, 150, 60, 60, 0, 0);
    doc.move_selected_notes(10, 0).expect("移动应产生 undo");
    let ticks: Vec<u32> = doc.data.model.notes[60]
        .iter()
        .map(|n| n.start_tick)
        .collect();
    assert_eq!(ticks, vec![110], "只应移动一次（+10），实际 {ticks:?}");
}

#[test]
fn resize_selected_notes_right_extends_end_tick() {
    let mut doc = make_doc_with_note();
    // 原音符: tick 100~200, key 60
    let snap = doc.capture_snapshot();
    let action = doc
        .resize_selected_notes(ResizeSide::Right, 50)
        .expect("应产生 UndoAction");
    let note = doc.data.model.notes[60][0];
    assert_eq!(note.start_tick, 100, "start_tick 不变");
    assert_eq!(note.end_tick, 250, "end_tick += 50");

    // 选区右边界同步偏移
    let (ts, te, _kl, _kh, _tl, _th) = doc.edit.selected.rects[0];
    assert_eq!(ts, 100, "选区 ts 不变");
    assert_eq!(te, 251, "选区 te += 50 (原 201)");

    // 无 clamp/无重叠 → 操作式（O(1) undo）
    match &action {
        UndoAction::ResizeNotes {
            side, delta_ticks, ..
        } => {
            assert_eq!(*side, ResizeSide::Right);
            assert_eq!(*delta_ticks, 50);
        }
        other => panic!("期望 UndoAction::ResizeNotes，实际 {other:?}"),
    }
    // undo/redo 精确
    doc.push_undo(action, "resize-right", snap);
    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[60][0].end_tick, 200);
    assert!(doc.redo());
    assert_eq!(doc.data.model.notes[60][0].end_tick, 250);
}

#[test]
fn resize_selected_notes_left_shifts_start_tick() {
    let mut doc = make_doc_with_note();
    // 原音符: tick 100~200, key 60
    doc.resize_selected_notes(ResizeSide::Left, -30)
        .expect("应产生 UndoAction");
    let note = doc.data.model.notes[60][0];
    assert_eq!(note.start_tick, 70, "start_tick -= 30");
    assert_eq!(note.end_tick, 200, "end_tick 不变");

    // 选区左边界同步偏移
    let (ts, te, _kl, _kh, _tl, _th) = doc.edit.selected.rects[0];
    assert_eq!(ts, 70, "选区 ts -= 30");
    assert_eq!(te, 201, "选区 te 不变");
}

/// 左拉伸快速路径 round-trip：undo/redo 的音符与选区单边跟随都精确。
#[test]
fn resize_selected_notes_left_fast_path_roundtrips() {
    let mut doc = make_doc_with_note();
    let snap = doc.capture_snapshot();
    let action = doc
        .resize_selected_notes(ResizeSide::Left, -30)
        .expect("应产生 UndoAction");
    assert!(
        matches!(
            action,
            UndoAction::ResizeNotes {
                side: ResizeSide::Left,
                ..
            }
        ),
        "无 clamp/无重叠应走操作式"
    );
    doc.push_undo(action, "resize-left", snap);

    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[60][0].start_tick, 100);
    assert_eq!(doc.edit.selected.rects[0].0, 100, "undo 后选区左边界回位");
    assert!(doc.redo());
    assert_eq!(doc.data.model.notes[60][0].start_tick, 70);
    assert_eq!(doc.edit.selected.rects[0].0, 70, "redo 后选区左边界跟随");
}

/// 拉伸目标与非选中音符重叠（禁止重叠）→ 回退副本路径并按 behavior 拦截
///（默认 KeepOriginal 且全拦 → 返回 None、音符保持原样）。
#[test]
fn resize_selected_notes_overlap_falls_back_to_blocked() {
    let mut doc = make_doc_with_note(); // t100~200, key 60, 选区 t100~201
    doc.edit.allow_overlapping_notes = false;
    doc.add_note(
        0,
        yinhe_core::NoteEvent {
            id: 0,
            start_tick: 300,
            end_tick: 400,
            key: 60,
            velocity: 90,
        },
    );
    // 右拉伸 +150 → 目标 [100,350) 与 [300,400) 重叠 → 全拦。
    assert!(
        doc.resize_selected_notes(ResizeSide::Right, 150).is_none(),
        "全拦应返回 None"
    );
    assert_eq!(
        doc.data.model.notes[60][0].end_tick, 200,
        "被拦音符保持原样"
    );
}

#[test]
fn resize_selected_notes_right_clamps_to_min_length() {
    let mut doc = make_doc_with_note();
    // 原音符: tick 100~200 (长度 100)。dt = -200 会让 end < start，应 clamp 到 start+1
    doc.resize_selected_notes(ResizeSide::Right, -200)
        .expect("应产生 UndoAction");
    let note = doc.data.model.notes[60][0];
    assert_eq!(note.start_tick, 100);
    assert_eq!(note.end_tick, 101, "end_tick 应 clamp 到 start+1");
}

#[test]
fn resize_selected_notes_left_clamps_to_min_length() {
    let mut doc = make_doc_with_note();
    // 原音符: tick 100~200。dt = 200 会让 start >= end，应 clamp 到 end-1
    doc.resize_selected_notes(ResizeSide::Left, 200)
        .expect("应产生 UndoAction");
    let note = doc.data.model.notes[60][0];
    assert_eq!(note.start_tick, 199, "start_tick 应 clamp 到 end-1");
    assert_eq!(note.end_tick, 200);
}

#[test]
fn resize_selected_notes_zero_dt_returns_none() {
    let mut doc = make_doc_with_note();
    assert!(doc.resize_selected_notes(ResizeSide::Right, 0).is_none());
    assert!(doc.resize_selected_notes(ResizeSide::Left, 0).is_none());
}

#[test]
fn resize_selected_notes_empty_selection_returns_none() {
    let mut doc = make_doc_with_note();
    doc.edit.selected.clear();
    assert!(doc.resize_selected_notes(ResizeSide::Right, 50).is_none());
}

#[test]
fn resize_selected_notes_remembers_gate() {
    let mut doc = make_doc_with_note(); // t100~200 gate 100
    doc.add_note(
        0,
        yinhe_core::NoteEvent {
            id: 0,
            start_tick: 300,
            end_tick: 500,
            key: 60,
            velocity: 90,
        },
    ); // t300~500 gate 200
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 501, 60, 60, 0, 0);
    doc.edit.sel_rect.clear();
    doc.edit.sel_rect.push_rect((100.0, 501.0, 60, 60), false);
    // 右拉 20 tick：gate 变 120 / 220，记时间最晚的 220
    assert!(
        doc.resize_selected_notes(crate::edit_state::ResizeSide::Right, 20)
            .is_some()
    );
    assert_eq!(doc.edit.default_gate(0, 120), 220);
}

/// move_selected_notes：目标被非移动集合的已有音符占据的音符留在原处，
/// 其余正常移动；undo 回退副本制且只含真正移动的音符。
#[test]
fn move_selected_notes_partially_blocked_when_disallowed() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(100, 150, 62)); // B k62（待选中）
    doc.add_note(0, ev(500, 600, 60)); // C k60 占位（不选中）
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0); // 选中 A、B
    doc.edit.allow_overlapping_notes = false;
    doc.edit.overlap_blocked_behavior = crate::audio_settings::OverlapBlockedBehavior::KeepOriginal;

    // +400 tick：A 目标 [500,600) 与 C 重叠 → 留原处；B 目标 k62 [500,550) 无阻挡 → 移动
    let before_snap = doc.capture_snapshot();
    let action = doc
        .move_selected_notes(400, 0)
        .expect("部分移动应产生 undo");
    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 1, "delta 只含真正移动的 B");
            assert_eq!(delta.after.len(), 1);
            assert_eq!(delta.after[0].1, 62);
            assert_eq!(delta.after[0].0.start_tick, 500);
        }
        other => panic!("有被拦音符时应回退副本制 Notes，实际 {other:?}"),
    }
    // A 留原处、C 不动
    assert_eq!(doc.data.model.notes[60].len(), 2);
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 100 && n.end_tick == 200),
        "A 应留在原处"
    );
    // B 已移动
    let b = doc.data.model.notes[62]
        .iter()
        .find(|n| n.start_tick == 500)
        .copied();
    assert_eq!(b.map(|n| n.end_tick), Some(550), "B 应移到 [500,550)");

    // undo 回放不受开关拦截：B 回到原位置
    doc.push_undo(action, "move", before_snap);
    assert!(doc.undo(), "undo 应成功");
    assert!(
        doc.data.model.notes[62]
            .iter()
            .any(|n| n.start_tick == 100 && n.end_tick == 150),
        "undo 后 B 应回到 [100,150)"
    );
    // redo 同样不受拦截
    assert!(doc.redo(), "redo 应成功");
    assert!(
        doc.data.model.notes[62]
            .iter()
            .any(|n| n.start_tick == 500 && n.end_tick == 550),
        "redo 后 B 应再次位于 [500,550)"
    );
}

/// move_selected_notes：全部被拦时整体无视（None，选区与模型不变）。
#[test]
fn move_selected_notes_all_blocked_returns_none() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(500, 600, 60)); // C 占位
    doc.edit.allow_overlapping_notes = false;
    doc.edit.overlap_blocked_behavior = crate::audio_settings::OverlapBlockedBehavior::KeepOriginal;

    assert!(
        doc.move_selected_notes(400, 0).is_none(),
        "目标全被占据时应整体无视"
    );
    assert_eq!(doc.data.model.notes[60].len(), 2);
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 100 && n.end_tick == 200),
        "A 应留在原处"
    );
    // 选区未偏移
    assert_eq!(doc.edit.selected.rects[0].0, 100);
}

/// move_selected_notes：DeleteOriginal 被拦 → 原音符删除、目标保留；undo/redo 精确。
#[test]
fn move_selected_notes_delete_original_removes_blocked() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(500, 600, 60)); // C 占位
    doc.edit.allow_overlapping_notes = false;
    doc.edit.overlap_blocked_behavior =
        crate::audio_settings::OverlapBlockedBehavior::DeleteOriginal;
    let before_snap = doc.capture_snapshot();
    let action = doc.move_selected_notes(400, 0).expect("应产生 undo");

    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 1, "被拦的 A 进 before（undo 恢复用）");
            assert!(delta.after.is_empty(), "A 被删、无落点插入");
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }
    assert_eq!(doc.data.model.notes[60].len(), 1, "只剩 C");
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 500 && n.end_tick == 600)
    );

    doc.push_undo(action, "move-delete", before_snap);
    assert!(doc.undo(), "undo 应恢复 A");
    assert_eq!(doc.data.model.notes[60].len(), 2);
    assert!(doc.redo(), "redo 应再次删除 A");
    assert_eq!(doc.data.model.notes[60].len(), 1);
}

/// move_selected_notes：ReplaceTarget 被拦 → 删除落点音符并搬运；undo/redo 精确。
#[test]
fn move_selected_notes_replace_target_overwrites() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(500, 600, 60)); // C 落点
    doc.edit.allow_overlapping_notes = false;
    doc.edit.overlap_blocked_behavior =
        crate::audio_settings::OverlapBlockedBehavior::ReplaceTarget;
    let before_snap = doc.capture_snapshot();
    let action = doc.move_selected_notes(400, 0).expect("应产生 undo");

    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 2, "before = A + 被替换的 C");
            assert_eq!(delta.after.len(), 1, "after = 落点的 A");
            assert_eq!(delta.after[0].0.start_tick, 500);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }
    assert_eq!(doc.data.model.notes[60].len(), 1, "C 被覆盖");
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 500 && n.end_tick == 600)
    );

    doc.push_undo(action, "move-replace", before_snap);
    assert!(doc.undo(), "undo 应恢复 A 与 C");
    assert_eq!(doc.data.model.notes[60].len(), 2);
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 100 && n.end_tick == 200)
    );
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 500 && n.end_tick == 600)
    );
    assert!(doc.redo(), "redo 应再次替换");
    assert_eq!(doc.data.model.notes[60].len(), 1);
}

/// 筛选后移动只搬匹配音符；action 携带筛选边界，undo 精确恢复。
#[test]
fn move_with_velocity_filter_moves_subset_and_undo_restores() {
    let mut doc = make_doc_with_note(); // k60 [100,200) v100（已选中）
    doc.add_note(
        0,
        NoteEvent {
            id: 0,
            start_tick: 100,
            end_tick: 150,
            key: 62,
            velocity: 20,
        },
    );
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0);
    doc.edit.selected.filter.velocity = Some((90, 127)); // 只筛 v100 的 k60

    let before_snap = doc.capture_snapshot();
    let action = doc.move_selected_notes(400, 0).expect("应移动");
    match &action {
        UndoAction::MoveNotes { selection, .. } => {
            assert_eq!(
                selection.filter.velocity,
                Some((90, 127)),
                "操作式 undo 应携带筛选边界"
            );
        }
        other => panic!("应走操作式 MoveNotes，实际 {other:?}"),
    }
    // k60 已移动，k62（v20）原地不动
    assert!(doc.data.model.notes[60].iter().any(|n| n.start_tick == 500));
    assert!(doc.data.model.notes[62].iter().any(|n| n.start_tick == 100));

    doc.push_undo(action, "move", before_snap);
    assert!(doc.undo(), "undo 应成功");
    assert!(
        doc.data.model.notes[60].iter().any(|n| n.start_tick == 100),
        "k60 应回到原位"
    );
    assert!(
        doc.data.model.notes[62].iter().any(|n| n.start_tick == 100),
        "k62 从未移动"
    );
}
