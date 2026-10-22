use super::*;

/// 回归：重叠选框下复制不产生重复副本。
#[test]
fn overlapping_rects_duplicate_once() {
    let mut doc = make_doc_with_note();
    doc.edit.selected.add_rect_track(50, 150, 60, 60, 0, 0);
    doc.duplicate_selected().expect("复制应产生 undo");
    let total: usize = (0..yinhe_types::KEY_COUNT)
        .map(|k| doc.data.model.notes[k].len())
        .sum();
    assert_eq!(total, 2, "原件 + 恰好一份副本");
}

#[test]
fn repeat_selected_creates_all_steps_in_one_action() {
    let mut doc = make_doc_with_note();
    let action = doc.repeat_selected(100, 2).expect("重复应产生 undo");
    let total: usize = (0..yinhe_types::KEY_COUNT)
        .map(|k| doc.data.model.notes[k].len())
        .sum();
    assert_eq!(total, 3, "原件 + 两个重复段");
    assert_eq!(doc.data.model.notes[60].len(), 3);
    assert_eq!(doc.data.model.notes[60][1].start_tick, 200);
    assert_eq!(doc.data.model.notes[60][2].start_tick, 300);
    // 选区保持原位置，方便继续拖动 Riff 方块。
    assert_eq!(doc.edit.selected.rects.len(), 1);
    assert_eq!(doc.edit.selected.rects[0].0, 100);
    assert_eq!(doc.edit.selected.rects[0].1, 201);
    match action {
        UndoAction::Notes(delta) => {
            assert!(delta.before.is_empty());
            assert_eq!(delta.after.len(), 2);
        }
        _ => panic!("期望 UndoAction::Notes"),
    }
}

#[test]
fn duplicate_selected_to_preserves_original_and_offsets_copy() {
    let mut doc = make_doc_with_note();
    let action = doc
        .duplicate_selected_to(50, 12)
        .expect("should produce action");

    // 原音符保留在 key 60 (tick 100~200)
    assert_eq!(doc.data.model.notes[60].len(), 1, "原音符应在 key 60");
    // 副本在 key 72, tick 150~250
    assert_eq!(doc.data.model.notes[72].len(), 1, "副本应在 key 72");
    let copy = doc.data.model.notes[72][0];
    assert_eq!(copy.start_tick, 150);
    assert_eq!(copy.end_tick, 250);

    // 原音符仍在 key 60
    let orig = doc.data.model.notes[60][0];
    assert_eq!(orig.start_tick, 100);
    assert_eq!(orig.end_tick, 200);

    // 选区跟随副本
    assert_eq!(doc.edit.selected.rects.len(), 1);
    let (ts, te, kl, kh, _tl, _th) = doc.edit.selected.rects[0];
    assert_eq!(ts, 150);
    assert_eq!(te, 251);
    assert_eq!(kl, 72);
    assert_eq!(kh, 72);

    // UndoAction 应该是 Notes，before 空，after 含副本
    match action {
        UndoAction::Notes(delta) => {
            assert!(delta.before.is_empty(), "复制操作 before 应为空");
            assert_eq!(delta.after.len(), 1);
            assert_eq!(delta.after[0].1, 72); // key
        }
        _ => panic!("期望 UndoAction::Notes"),
    }
}

#[test]
fn duplicate_selected_to_empty_selection_returns_none() {
    let mut doc = make_doc_with_note();
    doc.edit.selected.clear();
    assert!(doc.duplicate_selected_to(50, 12).is_none());
}

#[test]
fn duplicate_selected_to_clamps_key_boundary() {
    let mut doc = make_doc_with_note();
    // key 60 + 200 半音 = 260, 应 clamp 到 MAX_KEY(255)
    let _ = doc.duplicate_selected_to(0, 200);
    assert_eq!(
        doc.data.model.notes[yinhe_types::MAX_KEY as usize].len(),
        1,
        "应 clamp 到 key MAX_KEY"
    );
}

/// Bug 7 回归：Alt 拖动复制后必须 bump 文档 revision，否则 GPU 层缓存不失效、画面不更新。
#[test]
fn duplicate_selected_to_bumps_revision() {
    let mut doc = make_doc_with_note();
    let rev_before = doc.data.revision;
    assert!(doc.duplicate_selected_to(50, 12).is_some(), "复制应成功");
    assert!(
        doc.data.revision > rev_before,
        "复制后文档 revision 必须前进（GPU 缓存失效依据）"
    );
    // per-key revision 也应前进（GPU cull 增量上传依据）
    assert!(
        doc.data.note_revisions()[72] > 0,
        "副本所在 key 的 revision 应前进"
    );
}

#[test]
fn duplicate_follows_visual_rects() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    let _ = doc.duplicate_selected();
    // 数据选区 + PR 视觉选框一起平移到副本（tick 偏移 100）
    assert_eq!(doc.edit.selected.rects[0].0, 200);
    assert_eq!(doc.edit.sel_rect.rects[0], (200.0, 301.0, 60, 60));
}

/// duplicate_selected：与已有音符重叠的副本跳过，其余正常插入。
#[test]
fn duplicate_selected_skips_overlapping_copy() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(100, 150, 62)); // B k62（待选中）
    // C k60 [250,350)：在选区外，但与 A 的副本目标 [200,300) 相交
    doc.add_note(0, ev(250, 350, 60));
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0); // 选中 A、B
    doc.edit.allow_overlapping_notes = false;

    // offset = 100：A 副本 [200,300) 被 C 拦下；B 副本 k62 [200,250) 插入
    let action = doc.duplicate_selected().expect("应有部分副本插入");
    match action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.after.len(), 1, "只有 B 的副本被插入");
            assert_eq!(delta.after[0].1, 62);
        }
        other => panic!("期望 UndoAction::Notes，实际 {other:?}"),
    }
    assert_eq!(doc.data.model.notes[60].len(), 2, "k60 只有 A 和 C");
    assert_eq!(doc.data.model.notes[62].len(), 2, "B 及其副本");
}

/// duplicate_selected：副本全被拦时返回 None。
#[test]
fn duplicate_selected_all_blocked_returns_none() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(250, 350, 60)); // C 拦下副本目标 [200,300)
    doc.edit.allow_overlapping_notes = false;
    assert!(
        doc.duplicate_selected().is_none(),
        "副本全被拦时应返回 None"
    );
    assert_eq!(doc.data.model.notes[60].len(), 2);
}

/// duplicate_selected_to：与已有音符重叠的副本跳过。
#[test]
fn duplicate_selected_to_skips_overlapping_copy() {
    let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
    doc.add_note(0, ev(500, 600, 60)); // C k60 占位
    doc.edit.allow_overlapping_notes = false;

    // 副本目标 [500,600) 与 C 重叠 → 全部跳过
    assert!(doc.duplicate_selected_to(400, 0).is_none());
    assert_eq!(doc.data.model.notes[60].len(), 2);

    // 换 key 方向（k62）无阻挡 → 正常插入
    assert!(doc.duplicate_selected_to(400, 2).is_some());
    assert_eq!(doc.data.model.notes[62].len(), 1);
}
