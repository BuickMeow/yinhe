use super::*;

/// 剪刀切割：切点切开跨过它的音符（原 id 留左半、新 id 给右半），
/// 边界/范围外不切，undo/redo 完整回放。
#[test]
fn split_notes_at_cuts_crossing_notes_and_undo_roundtrip() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(300, 400, 60)); // 不跨切点
    doc.add_note(0, ev(100, 400, 62)); // 另一行的跨切点音符
    let before_snap = doc.capture_snapshot();
    let action = doc.split_notes_at(&[(60, 150), (62, 150)]).expect("应切割");

    // k60: [100,150) + [150,200) + [300,400)
    assert_eq!(doc.data.model.notes[60].len(), 3);
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 100 && n.end_tick == 150)
    );
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 150 && n.end_tick == 200)
    );
    // k62: [100,150) + [150,400)
    assert_eq!(doc.data.model.notes[62].len(), 2);
    assert!(
        doc.data.model.notes[62]
            .iter()
            .any(|n| n.start_tick == 150 && n.end_tick == 400)
    );
    // 左半保留原 id
    let orig_id = doc.data.model.notes[60]
        .iter()
        .find(|n| n.start_tick == 100 && n.end_tick == 150)
        .map(|n| n.id);
    assert!(orig_id.is_some());

    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 2, "两个原音符");
            assert_eq!(delta.after.len(), 4, "切出四半");
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }

    doc.push_undo(action, "split", before_snap);
    assert!(doc.undo(), "undo 应成功");
    assert_eq!(doc.data.model.notes[60].len(), 2);
    assert!(
        doc.data.model.notes[60]
            .iter()
            .any(|n| n.start_tick == 100 && n.end_tick == 200)
    );
    assert_eq!(doc.data.model.notes[62].len(), 1);
    assert!(doc.redo(), "redo 应成功");
    assert_eq!(doc.data.model.notes[60].len(), 3);
    assert_eq!(doc.data.model.notes[62].len(), 2);
}

/// 切点在音符边界（start/end）上或音符外：不切，返回 None。
#[test]
fn split_notes_at_skips_boundary_and_outside_cuts() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    assert!(
        doc.split_notes_at(&[(60, 100)]).is_none(),
        "cut == start 不切"
    );
    assert!(
        doc.split_notes_at(&[(60, 200)]).is_none(),
        "cut == end 不切"
    );
    assert!(doc.split_notes_at(&[(60, 50)]).is_none(), "音符左侧不切");
    assert!(doc.split_notes_at(&[(60, 500)]).is_none(), "音符右侧不切");
    assert!(doc.split_notes_at(&[]).is_none(), "空切点表");
    assert_eq!(doc.data.model.notes[60].len(), 1);
}

/// 作用域过滤：不可见轨、选中其他轨都不切；选中为空 = 全部可见轨。
#[test]
fn split_notes_at_respects_track_scope() {
    let mut doc = make_doc_with_note(); // track 0, k60 [100,200)
    doc.edit.track_pianoroll_visible = vec![false];
    assert!(doc.split_notes_at(&[(60, 150)]).is_none(), "不可见轨不切");
    doc.edit.track_pianoroll_visible = vec![true];
    doc.edit.track_selected.insert(5);
    assert!(
        doc.split_notes_at(&[(60, 150)]).is_none(),
        "选中其他轨时不切"
    );
    doc.edit.track_selected.clear();
    assert!(
        doc.split_notes_at(&[(60, 150)]).is_some(),
        "清空选中 = 全部可见轨"
    );
}

/// 网格切割：只切选框内的区段（含边界切点），框外头尾保留，
/// 无切点的音符不动；undo/redo 完整回放。
#[test]
fn split_selection_by_grid_cuts_inside_box_only() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(480);
    doc.add_note(0, ev(0, 960, 62)); // 跨框边界的长音
    doc.add_note(0, ev(100, 300, 64)); // 不跨任何网格点
    doc.edit.sel_rect.rects = vec![(480.0, 960.0, 60, 64)];

    let before_snap = doc.capture_snapshot();
    let action = doc.split_selection_by_grid().expect("应切割");
    // k62 [0,960) → [0,480) + [480,960)：480 同时在框边界与音符内部
    assert_eq!(doc.data.model.notes[62].len(), 2);
    assert!(
        doc.data.model.notes[62]
            .iter()
            .any(|n| n.start_tick == 0 && n.end_tick == 480)
    );
    assert!(
        doc.data.model.notes[62]
            .iter()
            .any(|n| n.start_tick == 480 && n.end_tick == 960)
    );
    // k60 [100,200)、k64 [100,300)：框内无网格点 → 不切
    assert_eq!(doc.data.model.notes[60].len(), 1);
    assert_eq!(doc.data.model.notes[64].len(), 1);

    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 1);
            assert_eq!(delta.after.len(), 2);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }

    doc.push_undo(action, "grid", before_snap);
    assert!(doc.undo(), "undo 应成功");
    assert_eq!(doc.data.model.notes[62].len(), 1);
    assert!(doc.redo(), "redo 应成功");
    assert_eq!(doc.data.model.notes[62].len(), 2);
}

/// 网格切割：框完全包含音符时切成多段（首段保留原 id），
/// 框外部分越出时只切到框边界。
#[test]
fn split_selection_by_grid_multi_segments() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(120);
    doc.add_note(0, ev(0, 600, 62));
    doc.edit.sel_rect.rects = vec![(120.0, 480.0, 62, 62)];
    // 切点：240, 360, 480 → [0,240) 跨框左界？不对：切点含框边界 120 吗？
    // 120 在音符内部 (0<120<600) 且 t0<=120<=t1 → 是切点；
    // 240、360、480 同理 → 段：[0,120) [120,240) [240,360) [360,480) [480,600)
    let action = doc.split_selection_by_grid().expect("应切割");
    assert_eq!(doc.data.model.notes[62].len(), 5);
    let starts: Vec<u32> = doc.data.model.notes[62]
        .iter()
        .map(|n| n.start_tick)
        .collect();
    assert_eq!(starts, vec![0, 120, 240, 360, 480]);
    let orig_id = doc.data.model.notes[62]
        .iter()
        .find(|n| n.start_tick == 0)
        .map(|n| n.id);
    assert!(orig_id.is_some(), "首段保留原音符 id");
    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 1);
            assert_eq!(delta.after.len(), 5);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }
}

/// 网格切割：空选框 / 不可见轨 / interval 0 都返回 None。
#[test]
fn split_selection_by_grid_returns_none_when_scope_blocks() {
    let mut doc = make_doc_with_note();
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(480);
    assert!(doc.split_selection_by_grid().is_none(), "无选框");

    // 框与 k60 [100,200) 相交且含切点 120（边界）。
    doc.edit.sel_rect.rects = vec![(120.0, 480.0, 60, 60)];
    doc.edit.track_pianoroll_visible = vec![false];
    assert!(doc.split_selection_by_grid().is_none(), "不可见轨不切");

    doc.edit.track_pianoroll_visible = vec![true];
    doc.edit.track_selected.insert(5);
    assert!(doc.split_selection_by_grid().is_none(), "选中其他轨时不切");

    doc.edit.track_selected.clear();
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(0);
    assert!(doc.split_selection_by_grid().is_none(), "interval 0 不切");
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Off;
    assert!(doc.split_selection_by_grid().is_none(), "Off 不切");
    assert_eq!(doc.data.model.notes[60].len(), 1, "模型未被改动");
}

/// 回归：`Auto` 量化必须先按 PR 缩放出具体网格（与量化按钮显示一致），
/// 否则网格切割会静默无操作（旧网格工具同病）。
#[test]
fn split_selection_by_grid_resolves_auto_quantize() {
    let mut doc = make_doc_with_note();
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Auto;
    doc.edit.pianoroll_view.base.pixels_per_tick = 0.1; // 肉眼看合理缩放下 Auto 必解析出 >0 的网格
    let ppq = doc.data.model.meta.ppq;
    let interval = doc
        .edit
        .quantize_pianoroll
        .resolved(doc.edit.pianoroll_view.base.pixels_per_tick, ppq)
        .tick_interval(ppq);
    assert!(interval > 0, "Auto 应解析出具体网格");

    // 放一条横跨 8 个网格点的长音，框住整条 → 应切成 8 段。
    let span = interval * 8;
    doc.add_note(0, ev(0, span, 62));
    doc.edit.sel_rect.rects = vec![(0.0, span as f64, 62, 62)];
    doc.split_selection_by_grid().expect("Auto 解析后应能切");
    let starts: Vec<u32> = doc.data.model.notes[62]
        .iter()
        .map(|n| n.start_tick)
        .collect();
    let expect: Vec<u32> = (0..8).map(|i| i * interval).collect();
    assert_eq!(starts, expect, "按解析出的 Auto 网格切开");
}

/// 钢笔生成：沿音高行逐行、gate=量化间隔、力度=记忆力度，undo/redo 回放。
#[test]
fn generate_pen_notes_per_row_with_memory_velocity_and_undo() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.edit.track_selected.insert(0);
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(120);
    doc.edit.remember_velocity(0, 0, 77);
    // 线 (0,62) → (480,64)：key 62→0、63→240、64→480
    doc.edit.pen_path = Some(crate::pen::PenPath {
        anchors: vec![
            crate::pen::PenAnchor::new(0.0, 62.0),
            crate::pen::PenAnchor::new(480.0, 64.0),
        ],
        closed: false,
    });

    let before_snap = doc.capture_snapshot();
    let action = doc.generate_pen_notes().expect("应生成");
    assert_eq!(doc.data.model.notes[62].len(), 1);
    assert_eq!(doc.data.model.notes[63].len(), 1);
    assert_eq!(doc.data.model.notes[64].len(), 1);
    let n62 = doc.data.model.notes[62][0];
    assert_eq!((n62.start_tick, n62.end_tick, n62.velocity), (0, 120, 77));
    let n63 = doc.data.model.notes[63][0];
    assert_eq!((n63.start_tick, n63.end_tick), (240, 360));
    let n64 = doc.data.model.notes[64][0];
    assert_eq!((n64.start_tick, n64.end_tick), (480, 600));

    match &action {
        UndoAction::Notes(delta) => {
            assert!(delta.before.is_empty());
            assert_eq!(delta.after.len(), 3);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }
    doc.push_undo(action, "line", before_snap);
    assert!(doc.undo(), "undo 应成功");
    assert!(doc.data.model.notes[62].is_empty());
    assert!(doc.redo(), "redo 应成功");
    assert_eq!(doc.data.model.notes[62].len(), 1);
}

/// 钢笔生成：无主音轨时返回 None（与 PR 铅笔一致）。
#[test]
fn generate_pen_notes_requires_main_track() {
    let mut doc = make_doc_with_note();
    doc.edit.pen_path = Some(crate::pen::PenPath {
        anchors: vec![crate::pen::PenAnchor::new(0.0, 60.0)],
        closed: false,
    });
    assert!(doc.generate_pen_notes().is_none(), "无主音轨不生成");
}

/// 批量添加：「允许新重叠音符」关闭时过滤与已有音符重叠的项。
#[test]
fn add_notes_batch_filters_overlap_when_disabled() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.edit.allow_overlapping_notes = false;
    let notes = vec![
        NoteEvent {
            id: 0,
            start_tick: 150,
            end_tick: 250,
            key: 60,
            velocity: 100,
        },
        NoteEvent {
            id: 0,
            start_tick: 300,
            end_tick: 400,
            key: 60,
            velocity: 100,
        },
    ];
    let action = doc.add_notes_batch(0, &notes).expect("应添加 1 个");
    assert_eq!(doc.data.model.notes[60].len(), 2);
    match action {
        UndoAction::Notes(delta) => assert_eq!(delta.after.len(), 1),
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }

    doc.edit.allow_overlapping_notes = true;
    assert!(doc.add_notes_batch(0, &notes).is_some());
    assert_eq!(doc.data.model.notes[60].len(), 4, "允许重叠时全部添加");
}

/// 剪刀确认：按锚点线逐行切割（复用 split_notes_at 的作用域与 undo）。
#[test]
fn split_scissors_line_cuts_via_anchor_line() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(120);
    doc.add_note(0, ev(0, 600, 62));
    doc.edit.scissors_line = Some(crate::edit_state::AnchorLine {
        start: (240.0, 60),
        end: (240.0, 62),
    });
    let action = doc.split_scissors_line().expect("应切割");
    assert_eq!(doc.data.model.notes[62].len(), 2, "k62 在 240 处切开");
    assert_eq!(doc.data.model.notes[60].len(), 1, "k60 切点在音符外不切");
    assert!(matches!(action, UndoAction::Notes(_)));

    doc.edit.scissors_line = None;
    assert!(doc.split_scissors_line().is_none(), "无线时返回 None");
}
