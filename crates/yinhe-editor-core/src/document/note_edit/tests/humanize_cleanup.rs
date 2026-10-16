use super::*;

#[test]
fn humanize_applies_bounded_offsets() {
    let mut doc = make_doc_with_note(); // 音符：100~200，key 60，velocity 100
    let before = doc.data.model.notes[60][0];
    let mut rng = 0x1234_5678_9ABC_DEF0u64;
    let params = HumanizeParams {
        timing: 10,
        velocity: 8,
        gate: 6,
    };
    let action = doc.humanize_selected_with_rng(params, &mut rng);
    assert!(action.is_some(), "有选中音符应产生动作");
    let after = doc.data.model.notes[60][0];

    let dt = after.start_tick as i64 - before.start_tick as i64;
    assert!(dt.abs() <= 10, "时间偏移应在 ±10，实际 {dt}");
    let dv = after.velocity as i64 - before.velocity as i64;
    assert!(dv.abs() <= 8, "力度偏移应在 ±8，实际 {dv}");
    let gate_before = (before.end_tick - before.start_tick) as i64;
    let gate_after = (after.end_tick - after.start_tick) as i64;
    assert!(
        (gate_after - gate_before).abs() <= 6,
        "时值偏移应在 ±6，实际 {}",
        gate_after - gate_before
    );
    assert!(after.end_tick > after.start_tick, "end 必须大于 start");
}

#[test]
fn humanize_undo_restores_original() {
    let mut doc = make_doc_with_note();
    let before = doc.data.model.notes[60][0];
    let snap = doc.capture_snapshot();
    let mut rng = 42u64;
    let action = doc
        .humanize_selected_with_rng(
            HumanizeParams {
                timing: 12,
                velocity: 6,
                gate: 8,
            },
            &mut rng,
        )
        .expect("应产生动作");
    doc.push_undo(action, "humanize", snap);
    assert!(doc.undo(), "undo 应成功");
    let restored = doc.data.model.notes[60][0];
    assert_eq!(restored.start_tick, before.start_tick);
    assert_eq!(restored.end_tick, before.end_tick);
    assert_eq!(restored.velocity, before.velocity);
    assert_eq!(restored.id, before.id);
}

#[test]
fn humanize_noop_or_empty_returns_none() {
    let mut doc = make_doc_with_note();
    let mut rng = 7u64;
    assert!(
        doc.humanize_selected_with_rng(
            HumanizeParams {
                timing: 0,
                velocity: 0,
                gate: 0
            },
            &mut rng
        )
        .is_none(),
        "全 0 参数应无动作"
    );
    doc.edit.selected.clear();
    assert!(
        doc.humanize_selected_with_rng(
            HumanizeParams {
                timing: 5,
                velocity: 5,
                gate: 5
            },
            &mut rng
        )
        .is_none(),
        "空选区应无动作"
    );
}

#[test]
fn humanize_many_rounds_stay_valid() {
    let mut doc = make_doc_with_note();
    // 全时间轴选中，保证每轮都命中音符（抖动后不脱出）。
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, u32::MAX, 60, 60, 0, 0);
    let params = HumanizeParams {
        timing: 500,
        velocity: 127,
        gate: 500,
    };
    for seed in 1..=200u64 {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let _ = doc.humanize_selected_with_rng(params, &mut rng);
        let n = doc.data.model.notes[60][0];
        assert!(n.end_tick > n.start_tick, "seed {seed}: end 应大于 start");
        assert!(n.velocity <= 127, "seed {seed}: velocity 应 ≤127");
    }
}

/// 去重：无重叠（含首尾相接）返回 None，模型与 revision 均不变。
#[test]
fn dedup_overlapping_notes_no_overlap_returns_none() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(200, 300, 60)); // 相接不算重叠
    doc.add_note(0, ev(300, 400, 62));
    let before = note_tuples(&doc);
    let rev = doc.data.revision;

    assert!(doc.dedup_overlapping_notes(false).is_none());
    assert!(doc.dedup_overlapping_notes(true).is_none());
    assert_eq!(note_tuples(&doc), before, "无重叠不得改模型");
    assert_eq!(doc.data.revision, rev, "无重叠不得 bump revision");
}

/// 去重：同轨链式重叠保留最早者、删后者，undo/redo 语义精确。
#[test]
fn dedup_overlapping_notes_same_track_roundtrip() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(150, 250, 60)); // 与 [100,200) 重叠 → 删
    doc.add_note(0, ev(200, 300, 60)); // 与保留的 [100,200) 相接 → 留
    doc.add_note(0, ev(250, 350, 60)); // 与 [200,300) 重叠 → 删
    let snap = doc.capture_snapshot();
    let action = doc.dedup_overlapping_notes(false).expect("应删两条");

    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 2, "删两条后发重叠音符");
            assert!(delta.after.is_empty(), "去重不插入");
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }
    let kept: Vec<(u32, u32)> = doc.data.model.notes[60]
        .iter()
        .map(|n| (n.start_tick, n.end_tick))
        .collect();
    assert_eq!(kept, vec![(100, 200), (200, 300)]);

    doc.push_undo(action, "dedup", snap);
    assert!(doc.undo(), "undo 应恢复被删音符");
    assert_eq!(doc.data.model.notes[60].len(), 4);
    assert!(doc.redo(), "redo 应再次删除");
    assert_eq!(doc.data.model.notes[60].len(), 2);
    assert!(doc.data.model.notes[60].is_sorted(), "桶保持有序");
}

/// 去重：cross_track=false 只看同轨；cross_track=true 跨轨去重（保留最早者）。
#[test]
fn dedup_overlapping_notes_cross_track_scope() {
    let mut doc = make_doc_tracks(2); // track0 k60 [100,200)
    doc.add_note(1, ev(150, 250, 60)); // track1 与 track0 重叠

    assert!(
        doc.dedup_overlapping_notes(false).is_none(),
        "同轨去重不得跨轨删除"
    );
    assert_eq!(doc.data.model.notes[60].len(), 2, "跨轨重叠保留");

    let rev = doc.data.revision;
    doc.dedup_overlapping_notes(true).expect("跨轨去重应删后者");
    assert_eq!(doc.data.model.notes[60].len(), 1);
    assert_eq!(doc.data.model.notes[60][0].track, 0, "保留最早的 track0");
    assert!(doc.data.revision > rev, "去重应 bump revision");
}

// ── 框选填缝 / 框选合并（见 docs/spec-note-fill-merge.md）──

/// 填缝：断续音符终点逐个贴合下一个起点；同一轨道内跨 key 共享起音网格。
#[test]
fn fill_gaps_extends_to_next_start_across_keys() {
    let mut doc = make_doc_with_note(); // k60 [100,200)（已选中）
    doc.add_note(0, ev(300, 320, 62));
    doc.add_note(0, ev(400, 410, 62)); // 最后一个起点：保持原终点
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 1000, 0, 127, 0, 0);
    let snap = doc.capture_snapshot();
    let rev = doc.data.revision;

    let action = doc.fill_gaps_selected().expect("应有终点变化");
    assert_eq!(spans(&doc, 60), vec![(100, 300, 0)], "k60 终点贴合 300");
    assert_eq!(
        spans(&doc, 62),
        vec![(300, 400, 0), (400, 410, 0)],
        "k62 贴合 400；最后一个起点保持 410"
    );
    assert!(doc.data.revision > rev);
    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 2);
            assert_eq!(delta.after.len(), 2);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }

    doc.push_undo(action, "fill", snap);
    assert!(doc.undo(), "undo 应恢复原终点");
    assert_eq!(spans(&doc, 60), vec![(100, 200, 0)]);
    assert_eq!(spans(&doc, 62), vec![(300, 320, 0), (400, 410, 0)]);
    assert!(doc.redo(), "redo 应再次贴合");
    assert_eq!(spans(&doc, 60), vec![(100, 300, 0)]);
}

/// 填缝：长音符被剪短贴合下一个起点；同起点音符终点统一；最后一个起音组不改。
#[test]
fn fill_gaps_trims_overlap_and_unifies_same_start() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(0, 300, 60)); // 跨过后面所有起点 → 剪短
    doc.add_note(0, ev(100, 200, 60)); // 与 [100,300) 同起点旧终点
    doc.add_note(0, ev(0, 50, 60)); // 与 A 同起点 [0,300)
    doc.add_note(0, ev(300, 400, 60)); // 最后一个起点：保持原终点
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 500, 60, 60, 0, 0);

    doc.fill_gaps_selected().expect("应有终点变化");
    let got = spans(&doc, 60);
    assert_eq!(
        got,
        vec![
            (0, 100, 0),
            (0, 100, 0),
            (100, 300, 0),
            (100, 300, 0),
            (300, 400, 0),
        ],
        "重叠剪短 + 同起点统一到 100；最后一个起点保持 400"
    );
}

/// 填缝：只有一个不同起点 → 无变化返回 None，不 bump revision。
#[test]
fn fill_gaps_single_start_returns_none() {
    let mut doc = make_doc_with_note(); // k60 [100,200)（唯一选中音符）
    let before = note_tuples(&doc);
    let rev = doc.data.revision;
    assert!(doc.fill_gaps_selected().is_none());
    assert_eq!(note_tuples(&doc), before);
    assert_eq!(doc.data.revision, rev, "无变化不得 bump revision");
}

/// 填缝：按轨道分组——其他轨道的起点不作为本轨道的贴合目标。
#[test]
fn fill_gaps_groups_by_track() {
    let mut doc = make_doc_tracks(2); // track0 k60 [100,200)
    doc.add_note(1, ev(150, 400, 62)); // track1：若全局分组会把 k60 剪到 150
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 1000, 0, 127, 0, 1);
    assert!(
        doc.fill_gaps_selected().is_none(),
        "各轨只有一个起点：不得跨轨道贴合"
    );
    assert_eq!(spans(&doc, 60), vec![(100, 200, 0)]);
    assert_eq!(spans(&doc, 62), vec![(150, 400, 1)]);

    // 在 track0 再放一条：track0 内部贴合，track1 不受影响。
    doc.add_note(0, ev(500, 600, 62));
    doc.fill_gaps_selected().expect("track0 应有贴合");
    assert_eq!(spans(&doc, 60), vec![(100, 500, 0)]);
    assert_eq!(
        spans(&doc, 62),
        vec![(150, 400, 1), (500, 600, 0)],
        "track1 音符不被跨轨影响且自身保持原终点"
    );
}

/// 填缝：框外音符不参与（既不被改，也不作为贴合目标）。
#[test]
fn fill_gaps_ignores_notes_outside_selection() {
    let mut doc = make_doc_with_note(); // k60 [100,200)（选中，选区至 201）
    doc.add_note(0, ev(250, 300, 60)); // 框外
    let before = note_tuples(&doc);
    assert!(
        doc.fill_gaps_selected().is_none(),
        "框外音符不得作为贴合目标"
    );
    assert_eq!(note_tuples(&doc), before, "框外音符不得被修改");
}

/// 填缝：属性筛选生效——被筛掉的音符不参与、不作为贴合目标。
#[test]
fn fill_gaps_honors_selection_filter() {
    let mut doc = make_doc_with_note(); // k60 [100,200) v100
    doc.add_note(0, ev(300, 400, 60));
    doc.add_note(0, ev(500, 600, 60));
    let ids: Vec<u32> = doc.data.model.notes[60].iter().map(|n| n.id).collect();
    set_velocity(&mut doc, 60, ids[2], 30); // 最后一条力度 30 → 被筛选排除
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 1000, 60, 60, 0, 0);
    doc.edit.selected.filter.velocity = Some((90, 127));

    doc.fill_gaps_selected().expect("应有贴合到 300");
    assert_eq!(
        spans(&doc, 60),
        vec![(100, 300, 0), (300, 400, 0), (500, 600, 0)],
        "被筛掉的 [500,600) 不参与且不被改"
    );
}

/// 合并：首尾严格相接的链压成一条，保留链首 id/力度，终点取链尾终点。
#[test]
fn merge_adjacent_chain_keeps_head_attributes() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(200, 250, 60));
    doc.add_note(0, ev(250, 300, 60));
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(100, 301, 60, 60, 0, 0);
    let ids: Vec<u32> = doc.data.model.notes[60].iter().map(|n| n.id).collect();
    set_velocity(&mut doc, 60, ids[2], 40); // 链尾力度不同 → 以链首为准
    let snap = doc.capture_snapshot();

    let action = doc.merge_adjacent_selected().expect("应合并成一条");
    assert_eq!(spans(&doc, 60), vec![(100, 300, 0)]);
    let merged = doc.data.model.notes[60][0];
    assert_eq!(merged.id, ids[0], "保留链首 id");
    assert_eq!(merged.velocity, 100, "保留链首力度");
    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 3, "before 含链上全部音符");
            assert_eq!(delta.after.len(), 1);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }

    doc.push_undo(action, "merge", snap);
    assert!(doc.undo(), "undo 应恢复 3 条");
    assert_eq!(doc.data.model.notes[60].len(), 3);
    assert!(doc.redo(), "redo 应再次合并");
    assert_eq!(spans(&doc, 60), vec![(100, 300, 0)]);
    assert!(doc.data.model.notes[60].is_sorted(), "桶保持有序");
}

/// 合并：1 tick 空隙或重叠都断开（不合并）。
#[test]
fn merge_adjacent_gap_or_overlap_breaks_chain() {
    // 1 tick 空隙
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(201, 300, 60));
    doc.edit.selected.add_rect_track(0, 1000, 60, 60, 0, 0);
    assert!(
        doc.merge_adjacent_selected().is_none(),
        "1 tick 空隙不得合并"
    );
    assert_eq!(doc.data.model.notes[60].len(), 2);

    // 1 tick 重叠
    let mut doc2 = make_doc_with_note(); // k60 [100,200)
    doc2.add_note(0, ev(199, 300, 60));
    doc2.edit.selected.add_rect_track(0, 1000, 60, 60, 0, 0);
    assert!(doc2.merge_adjacent_selected().is_none(), "重叠不得合并");
    assert_eq!(doc2.data.model.notes[60].len(), 2);
}

/// 合并：链上音符必须全部被框选——被筛选排除的音符会断开链。
#[test]
fn merge_adjacent_requires_all_selected() {
    let mut doc = make_doc_with_note(); // k60 [100,200) 选中
    doc.add_note(0, ev(200, 250, 60)); // 相接
    doc.add_note(0, ev(250, 300, 60)); // 相接
    let ids: Vec<u32> = doc.data.model.notes[60].iter().map(|n| n.id).collect();
    set_velocity(&mut doc, 60, ids[1], 30); // 中间一条力度 30
    doc.edit.selected.add_rect_track(0, 1000, 60, 60, 0, 0);
    doc.edit.selected.filter.velocity = Some((90, 127)); // 中间的排除

    assert!(
        doc.merge_adjacent_selected().is_none(),
        "中间音符未选中 → 链断裂，两侧不合并"
    );
    assert_eq!(doc.data.model.notes[60].len(), 3);
}

/// 合并：同 key 不同轨道不合并；同轨链正常合并。
#[test]
fn merge_adjacent_same_track_only() {
    let mut doc = make_doc_tracks(2); // track0 k60 [100,200)
    doc.add_note(1, ev(200, 300, 60)); // track1 同 key 相接
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 1000, 60, 60, 0, 1);
    assert!(
        doc.merge_adjacent_selected().is_none(),
        "跨轨同 key 不得合并"
    );
    assert_eq!(spans(&doc, 60).len(), 2);

    doc.add_note(0, ev(200, 300, 60)); // track0 相接 → 与 track0 链合并
    doc.merge_adjacent_selected().expect("track0 相邻链应合并");
    assert_eq!(
        spans(&doc, 60),
        vec![(100, 300, 0), (200, 300, 1)],
        "只合并 track0 的链，track1 音符不动"
    );
}

/// 合并：同一次点击处理多条链，单 undo 项整体回滚。
#[test]
fn merge_adjacent_multiple_chains_one_undo() {
    let mut doc = make_doc_with_note(); // k60 [100,200)
    doc.add_note(0, ev(200, 300, 60)); // 与上一条相连
    doc.add_note(0, ev(500, 510, 60));
    doc.add_note(0, ev(510, 520, 60));
    doc.add_note(0, ev(520, 530, 60)); // 三条链
    doc.edit.selected.clear();
    doc.edit.selected.add_rect_track(0, 1000, 60, 60, 0, 0);
    let snap = doc.capture_snapshot();

    let action = doc.merge_adjacent_selected().expect("两条链都应合并");
    assert_eq!(spans(&doc, 60), vec![(100, 300, 0), (500, 530, 0)]);
    match &action {
        UndoAction::Notes(delta) => {
            assert_eq!(delta.before.len(), 5);
            assert_eq!(delta.after.len(), 2);
        }
        other => panic!("期望副本制 Notes，实际 {other:?}"),
    }

    doc.push_undo(action, "merge", snap);
    assert!(doc.undo());
    assert_eq!(doc.data.model.notes[60].len(), 5, "一次 undo 恢复全部");
    assert!(doc.redo());
    assert_eq!(spans(&doc, 60), vec![(100, 300, 0), (500, 530, 0)]);
}
