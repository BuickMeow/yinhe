use super::*;

#[test]
fn transpose_follows_visual_rects() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    let _ = doc.transpose_selected(2);
    // 数据选区 + PR 视觉选框的 key 一起平移
    assert_eq!(doc.edit.selected.rects[0].2, 62);
    assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 201.0, 62, 62));
}

#[test]
fn rescale_selection_span_doubles_notes_and_rects() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    // 跨度 101 → 202（×2）：音符 start 100→100（起点不动），end 200→300
    let action = doc.rescale_selection_span(202).expect("should edit");
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 100);
    assert_eq!(n.end_tick, 300);
    assert_eq!(doc.edit.selected.rects[0].1, 302);
    assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 302.0, 60, 60));
    match action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before[0].0.end_tick, 200);
            assert_eq!(delta.after[0].0.end_tick, 300);
        }
        _ => panic!("expected Notes"),
    }
}

#[test]
fn rescale_selection_span_halves_notes() {
    let mut doc = make_doc_with_note();
    // 跨度 101 → 51（约 /2）：end 200 → 150
    doc.rescale_selection_span(51).expect("should edit");
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 100);
    assert_eq!(n.end_tick, 150);
    assert_eq!(doc.edit.selected.rects[0].1, 151);
}

#[test]
fn rescale_selection_span_same_span_returns_none() {
    let mut doc = make_doc_with_note();
    assert!(doc.rescale_selection_span(101).is_none());
}

#[test]
fn rescale_selection_span_empty_selection_returns_none() {
    let mut doc = make_doc_with_note();
    doc.edit.selected.clear();
    assert!(doc.rescale_selection_span(200).is_none());
}

/// 快速变速：×2（变慢）把跨度翻倍、音符等比拉长。
#[test]
fn scale_selected_span_doubles() {
    let mut doc = make_doc_with_note(); // 音符 (100,200)，跨度 101
    doc.scale_selected_span(2, 1).expect("should edit");
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 100);
    assert_eq!(n.end_tick, 300, "跨度 ×2 后音符末点翻倍");
}

/// 快速变速：÷2（变快）把跨度减半。
#[test]
fn scale_selected_span_halves() {
    let mut doc = make_doc_with_note();
    doc.scale_selected_span(1, 2).expect("should edit");
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 100);
    assert!(
        n.end_tick < 200,
        "跨度减半后末点应提前，实际 {}",
        n.end_tick
    );
}

/// 变速：空选区 / den=0 无操作。
#[test]
fn scale_selected_span_noop_returns_none() {
    let mut doc = make_doc_with_note();
    assert!(doc.scale_selected_span(2, 0).is_none(), "den=0 无操作");
    doc.edit.selected.clear();
    assert!(doc.scale_selected_span(2, 1).is_none(), "空选区无操作");
}

#[test]
fn rescale_undo_restores_notes_and_rects() {
    let mut doc = make_doc_with_note();
    doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
    let before = doc.capture_snapshot();
    let action = doc.rescale_selection_span(202).expect("should edit");
    doc.push_undo(action, "rescale", before);
    assert_eq!(doc.edit.selected.rects[0].1, 302, "编辑后选区跟随缩放");
    assert!(doc.undo(), "undo 应成功");
    // 音符与选区/选框都恢复编辑前状态
    assert_eq!(doc.data.model.notes[60][0].end_tick, 200);
    assert_eq!(doc.edit.selected.rects[0].1, 201);
    assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 201.0, 60, 60));
    assert!(doc.redo(), "redo 应成功");
    assert_eq!(doc.data.model.notes[60][0].end_tick, 300);
    assert_eq!(doc.edit.selected.rects[0].1, 302);
}

#[test]
fn flip_horizontal_mirrors_ticks() {
    let mut doc = make_doc_with_note();
    // 第二个音符 (150, 250) key 60
    doc.add_note(
        0,
        NoteEvent {
            id: 1,
            start_tick: 150,
            end_tick: 250,
            key: 60,
            velocity: 100,
        },
    );
    doc.edit.selected.add_rect_track(150, 251, 60, 60, 0, 0);
    let action = doc
        .flip_selected_notes(FlipAxis::Horizontal)
        .expect("should flip");
    // t0=100, t1=251：
    // n1 (100,200) → start' = 100 + (251-200) = 151, end' = 100 + (251-100) = 251
    // n2 (150,250) → start' = 100 + (251-250) = 101, end' = 100 + (251-150) = 201
    // 桶内按 start 排序：[(101,201), (151,251)]
    assert_eq!(doc.data.model.notes[60][0].start_tick, 101);
    assert_eq!(doc.data.model.notes[60][0].end_tick, 201);
    assert_eq!(doc.data.model.notes[60][1].start_tick, 151);
    assert_eq!(doc.data.model.notes[60][1].end_tick, 251);
    match action {
        UndoAction::FlipNotes {
            selection, bounds, ..
        } => {
            assert!(!selection.rects.is_empty());
            assert_eq!(bounds, (100, 251, 60, 60));
        }
        other => panic!("expected FlipNotes, got {other:?}"),
    }
}

#[test]
fn flip_vertical_mirrors_keys() {
    let mut doc = make_doc_with_note(); // key 60（alloc id 1）
    doc.add_note(
        0,
        NoteEvent {
            id: 2,
            start_tick: 100,
            end_tick: 200,
            key: 64,
            velocity: 100,
        },
    );
    doc.edit.selected.add_rect_track(100, 201, 64, 64, 0, 0);
    doc.flip_selected_notes(FlipAxis::Vertical)
        .expect("should flip");
    // kl=60, kh=64：60↔64（add_note 忽略传入 id，按 alloc 顺序为 1、2）
    assert!(
        doc.data.model.notes[64].iter().any(|n| n.id == 1),
        "key 60 的音符应镜像到 64"
    );
    assert!(
        doc.data.model.notes[60].iter().any(|n| n.id == 2),
        "key 64 的音符应镜像到 60"
    );
}

#[test]
fn flip_undo_restores_notes() {
    let mut doc = make_doc_with_note();
    let before = doc.capture_snapshot();
    let action = doc
        .flip_selected_notes(FlipAxis::Horizontal)
        .expect("should flip");
    doc.push_undo(action, "flip", before);
    assert!(doc.undo(), "undo 应成功");
    let n = doc.data.model.notes[60][0];
    assert_eq!(n.start_tick, 100);
    assert_eq!(n.end_tick, 200);
}
