//! Unified batch operations on notes.
//!
//! All large-scale note edits (delete, move, duplicate, transpose) share
//! the same pattern: group by key bucket, single `retain`/`drain` per bucket
//! for removal, single ordered append per bucket for insertion, then
//! `mark_dirty` + `rebuild_dirty` (stats only — buckets are kept sorted by
//! the write paths themselves). This module centralizes that pattern.
//!
//! Selection is always rectangular (from marquee). For each rect, iterate
//! the key range and use `partition_point` to find the tick range, then
//! `drain` or collect in a single pass per bucket.
//!
//! 框选物化后 `Selection` 带显式成员位图：谓词统一走
//! `Selection::accepts_note`（成员态查位图，矩形态查矩形 + 筛选），
//! 连续 drain 快路径只在矩形态、无属性边界且全轨时可用。

use std::collections::HashMap;
use std::sync::Arc;

use yinhe_core::{Selection, YinModel};
use yinhe_types::{MAX_KEY, Note, NoteBucket};

/// Remove all notes matching `selection` from the model.
///
/// For each rect × key range, deletes `start_tick ∈ [tick_start, tick_end)`
/// within the track range and passing the selection's attribute filter,
/// in a single pass per bucket (chunked: only hit chunks are scanned).
/// 矩形态且无属性边界、全轨选中 → 连续 `drain_range`；否则逐音符判定。
///
/// Returns the removed notes with their original key, so callers can
/// re-insert them at a new position (move/transpose) or discard them (delete).
pub fn remove_selected(model: &mut YinModel, selection: &Selection) -> Vec<(Note, u8)> {
    let mut removed: Vec<(Note, u8)> = Vec::new();
    let explicit = selection.has_explicit_members();
    let has_bounds = selection.filter.has_note_bounds();

    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            let bucket = Arc::make_mut(&mut model.notes[k]);
            // Fast path: 矩形态 + 全轨 + 无属性边界 → contiguous span drain.
            let out = if !explicit && track_lo == 0 && track_hi == u16::MAX && !has_bounds {
                bucket.drain_range(tick_start, tick_end)
            } else {
                bucket
                    .drain_range_filtered(tick_start, tick_end, |n| selection.accepts_note(n, key))
            };
            if !out.is_empty() {
                removed.extend(out.into_iter().map(|n| (n, key)));
                model.mark_dirty(key);
            }
        }
    }

    removed
}

/// Insert notes into the model, grouped by destination key.
///
/// For each key bucket, merges the sorted new notes into the chunked
/// sequence (O(N + K) per bucket, keeps chunking invariant). Marks each
/// touched bucket dirty. The caller is responsible for calling
/// `rebuild_dirty()` afterwards.
pub fn insert_batch(model: &mut YinModel, notes_by_key: HashMap<u8, Vec<Note>>) {
    for (key, notes) in notes_by_key {
        let k = key as usize;
        Arc::make_mut(&mut model.notes[k]).insert_batch_sorted(notes);
        model.mark_dirty(key);
    }
}

/// Append `new_notes` to a bucket keeping it sorted (chunked merge).
/// Kept for API compatibility with callers that pass a single bucket.
pub fn append_notes_ordered(bucket: &mut NoteBucket, new_notes: Vec<Note>) {
    bucket.insert_batch_sorted(new_notes);
}

/// Iterate notes matching `selection` (read-only), calling `f` for each.
///
/// Streaming variant of [`collect_selected`] for callers that must not
/// materialize the whole selection (e.g. writing huge clipboards to disk).
/// Honors explicit members and the selection's attribute filter.
pub fn for_each_selected(model: &YinModel, selection: &Selection, mut f: impl FnMut(&Note, u8)) {
    for &(tick_start, tick_end, key_lo, key_hi, _track_lo, _track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            for n in model.notes[k].range(tick_start, tick_end) {
                if !selection.accepts_note(n, key) {
                    continue;
                }
                f(n, key);
            }
        }
    }
}

/// 原地更新选中音符：逐桶 `update_matching`（按块预检、只深拷贝命中块）+
/// 命中桶 `sort`（`is_sorted` 早退，未破坏排序键时零重排）+ `mark_dirty`。
///
/// 统一生成端（note_edit/arrange_move）与 undo 回放端（history/apply）的 7 处
/// 手写骨架，谓词与原实现逐字一致（矩形空间判定 ∩ [`Selection::accepts_note`]），
/// 避免两端漂移。调用方随后调用 `rebuild_dirty()` 重建统计
/// （多次操作可合并为一次）。
///
/// 返回是否有音符被修改。
pub fn update_selected_in_place(
    model: &mut YinModel,
    selection: &Selection,
    mut f: impl FnMut(&mut Note, u8),
) -> bool {
    let mut any = false;
    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            let bucket = Arc::make_mut(&mut model.notes[k]);
            let touched = bucket.update_matching(
                |n| {
                    n.start_tick >= tick_start
                        && n.start_tick < tick_end
                        && n.track >= track_lo
                        && n.track <= track_hi
                        && selection.accepts_note(n, key)
                },
                |n| f(n, key),
            );
            if touched {
                bucket.sort();
                model.mark_dirty(key);
                any = true;
            }
        }
    }
    any
}

/// 流式判定选区是否命中至少一个音符（早停、零分配）。
///
/// 与 `collect_selected(...).is_empty()` 等价，但 1.64 亿选区下不物化 3GB。
pub fn any_selected(model: &YinModel, selection: &Selection) -> bool {
    for &(tick_start, tick_end, key_lo, key_hi, _track_lo, _track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            for n in model.notes[k].range(tick_start, tick_end) {
                if selection.accepts_note(n, key) {
                    return true;
                }
            }
        }
    }
    false
}

/// Collect notes matching `selection` from the model (read-only, no removal).
///
/// For each rect × key range, iterates `start_tick ∈ [tick_start, tick_end)`.
/// Returns `(Note, key)` pairs.
pub fn collect_selected(model: &YinModel, selection: &Selection) -> Vec<(Note, u8)> {
    let mut result: Vec<(Note, u8)> = Vec::new();
    for_each_selected(model, selection, |n, key| result.push((*n, key)));
    result
}

/// 「新音符是否与已有音符重叠」判定：同 track && [start, end) 区间相交。
///
/// 候选窗口用 `model.max_note_len` 保守左扩（与 PR 悬停 hit-test 同思路）：
/// 任何重叠音符必满足 `start_tick ∈ [start - max_note_len, end)`（相交要求
/// `ns < end && ne > start`，而 `gate ≤ max_note_len` ⟹ `ns > start - max_note_len`），
/// 再用精确条件 `n.end_tick > start` 过滤。`max_note_len` 只增不减，
/// 过期偏大只会放宽窗口、绝不漏候选。左闭右开区间：首尾相接不算重叠。
///
/// 复杂度 O(log 块数 + 窗口内候选数)，不做全桶线性扫描。
pub fn has_overlapping_note(model: &YinModel, track: u16, key: u8, start: u32, end: u32) -> bool {
    if end <= start {
        return false; // 零长/反向区间不与任何音符相交（编辑入口保证 gate >= 1，防御用）
    }
    let lo = start.saturating_sub(model.max_note_len);
    model.notes[key as usize]
        .range(lo, end)
        .any(|n| n.track == track && n.end_tick > start)
}

/// 排除自身 id 的重叠判定（单音符原地编辑：move/resize 前检查用）。
///
/// 与 `has_overlapping_note` 相同窗口算法，但跳过 `exclude_id`。
/// 用于 pencil 单音符操作等“原音符仍在桶内”的场景，避免自己与自己误判重叠。
pub fn has_overlapping_note_excluding(
    model: &YinModel,
    track: u16,
    key: u8,
    start: u32,
    end: u32,
    exclude_id: u32,
) -> bool {
    if end <= start {
        return false;
    }
    let lo = start.saturating_sub(model.max_note_len);
    model.notes[key as usize]
        .range(lo, end)
        .any(|n| n.id != exclude_id && n.track == track && n.end_tick > start)
}

/// 选中音符的统计信息（Info 面板选框信息显示）。
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectedNoteSummary {
    /// 选中音符总数。
    pub count: u64,
    /// 全部选中音符 velocity 相同时为 Some（用于编辑框显示），否则 None。
    pub velocity: Option<u8>,
    /// 全部选中音符 gate（end-start）相同时为 Some。
    pub gate: Option<u32>,
    /// 全部选中音符 key 相同时为 Some。
    pub key: Option<u8>,
    /// 全部选中音符 start_tick 相同时为 Some。
    pub tick: Option<u32>,
}

/// 统计选中音符数量与 uniform 字段值。
///
/// count 带全选快路径（无属性边界且单 rect 全范围时 O(1) 返回
/// `note_count`，避免全选 1 亿音符时扫描）；uniform 字段扫描遇到第一个
/// 不同值即短路为 None（绝大多数 mixed 情况无需遍历完整选区）。
pub fn summarize_selected(model: &YinModel, selection: &Selection) -> SelectedNoteSummary {
    let explicit = selection.has_explicit_members();
    let has_bounds = selection.filter.has_note_bounds();
    // 全选快路径：矩形态 + 无属性边界 + 单个 rect 覆盖全部 key/track 与全部 tick
    let full = !explicit
        && !has_bounds
        && selection.rects.iter().any(|&(ts, te, kl, kh, tl, th)| {
            kl == 0
                && kh == MAX_KEY
                && tl == 0
                && th == u16::MAX
                && ts == 0
                && te as u64 >= model.tick_length
        });
    let mut summary = SelectedNoteSummary {
        count: if full { model.note_count } else { 0 },
        ..Default::default()
    };
    let mut first = true;
    'outer: for &(ts, te, kl, kh, _tl, _th) in &selection.rects {
        for key in kl..=kh {
            let k = key as usize;
            for n in model.notes[k].range(ts, te) {
                if !selection.accepts_note(n, key) {
                    continue;
                }
                if !full {
                    summary.count += 1;
                }
                if first {
                    summary.velocity = Some(n.velocity);
                    summary.gate = Some(n.end_tick - n.start_tick);
                    summary.key = Some(key);
                    summary.tick = Some(n.start_tick);
                    first = false;
                } else {
                    if summary.velocity.is_some() && summary.velocity != Some(n.velocity) {
                        summary.velocity = None;
                    }
                    if summary.gate.is_some() && summary.gate != Some(n.end_tick - n.start_tick) {
                        summary.gate = None;
                    }
                    if summary.key.is_some() && summary.key != Some(key) {
                        summary.key = None;
                    }
                    if summary.tick.is_some() && summary.tick != Some(n.start_tick) {
                        summary.tick = None;
                    }
                    // 全选快路径：count 已 O(1) 就绪，四个 uniform 字段全部确定为
                    // mixed 后不会再变 → 提前结束（全选 1.6 亿实测 0.33s → ~0）。
                    // 非全选时 count 必须遍历完，不能提前退出。
                    if full
                        && summary.velocity.is_none()
                        && summary.gate.is_none()
                        && summary.key.is_none()
                        && summary.tick.is_none()
                    {
                        break 'outer;
                    }
                }
            }
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_core::{NoteEvent, TrackData, YinModel};

    fn note(id: u32, start: u32, end: u32) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: end,
            velocity: 100,
            track: 0,
        }
    }

    /// 断言桶按 start_tick 有序（块级二分的正确性前提）。
    fn assert_sorted(bucket: &NoteBucket) {
        assert!(bucket.is_sorted(), "bucket 失序");
    }

    #[test]
    fn append_notes_ordered_tail_fast_path() {
        let mut bucket = NoteBucket::from_sorted(vec![note(1, 0, 480), note(2, 480, 960)]);
        append_notes_ordered(&mut bucket, vec![note(3, 1000, 1500), note(4, 2000, 2500)]);
        assert_sorted(&bucket);
        assert_eq!(bucket.len(), 4);
    }

    #[test]
    fn append_notes_ordered_merges_into_middle() {
        let mut bucket = NoteBucket::from_sorted(vec![note(1, 0, 480), note(2, 2000, 2500)]);
        append_notes_ordered(&mut bucket, vec![note(3, 500, 900), note(4, 1000, 1500)]);
        assert_sorted(&bucket);
        assert_eq!(
            bucket.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![1, 3, 4, 2],
            "归并顺序错误"
        );
    }

    #[test]
    fn append_notes_ordered_merges_to_head() {
        let mut bucket = NoteBucket::from_sorted(vec![note(1, 500, 900), note(2, 2000, 2500)]);
        append_notes_ordered(&mut bucket, vec![note(3, 100, 200)]);
        assert_sorted(&bucket);
        assert_eq!(
            bucket.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
    }

    #[test]
    fn append_notes_ordered_empty_cases() {
        let mut bucket = NoteBucket::default();
        append_notes_ordered(&mut bucket, vec![note(1, 100, 200)]);
        assert_sorted(&bucket);
        assert_eq!(bucket.len(), 1);

        let mut bucket2 = NoteBucket::from_sorted(vec![note(1, 100, 200)]);
        append_notes_ordered(&mut bucket2, vec![]);
        assert_eq!(bucket2.len(), 1, "空输入不得改变桶");
    }

    #[test]
    fn append_notes_ordered_sorts_unsorted_input() {
        // 调用方（new_by_key 遍历顺序）不保证组内有序，必须内部先排。
        let mut bucket = NoteBucket::from_sorted(vec![note(1, 0, 100)]);
        append_notes_ordered(&mut bucket, vec![note(4, 3000, 4000), note(3, 1000, 2000)]);
        assert_sorted(&bucket);
        assert_eq!(
            bucket.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![1, 3, 4]
        );
    }

    #[test]
    fn append_notes_ordered_stable_for_equal_tick() {
        // 同 start_tick：旧桶元素在前，新追加在后（稳定）。
        let mut bucket = NoteBucket::from_sorted(vec![note(1, 480, 700)]);
        append_notes_ordered(&mut bucket, vec![note(2, 480, 600)]);
        assert_sorted(&bucket);
        assert_eq!(bucket.iter().map(|n| n.id).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn insert_batch_keeps_buckets_sorted() {
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
        by_key.insert(
            60,
            vec![note(1, 300, 500), note(2, 100, 200), note(3, 400, 600)],
        );
        by_key.insert(64, vec![note(4, 50, 100)]);
        insert_batch(&mut m, by_key);
        assert_sorted(&m.notes[60]);
        assert_sorted(&m.notes[64]);
        assert!(
            m.dirty_keys[60] && m.dirty_keys[64],
            "触达的桶必须标记 dirty"
        );
    }

    #[test]
    fn insert_batch_into_existing_sorted_bucket() {
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }]]);
        m.rebuild();
        let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
        by_key.insert(60, vec![note(9, 200, 300), note(8, 1000, 1200)]);
        insert_batch(&mut m, by_key);
        assert_sorted(&m.notes[60]);
        assert_eq!(m.notes[60].len(), 3);
    }

    /// 原地更新：矩形态命中 + 排序键被破坏后内部 sort 兜底 + 只标脏命中桶。
    #[test]
    fn update_selected_in_place_keeps_sorted_and_marks_dirty() {
        let mut m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 60, 60);
        let mut hits = 0;
        let touched = update_selected_in_place(&mut m, &sel, |n, _k| {
            n.start_tick = 1000 - n.start_tick; // 故意反转顺序
            hits += 1;
        });
        assert!(touched);
        assert_eq!(hits, 2);
        assert_sorted(&m.notes[60]);
        assert!(m.dirty_keys[60]);
        assert!(!m.dirty_keys[64], "未命中的桶不得标脏");
    }

    /// 原地更新：成员态按位图命中；空选区零命中返回 false。
    #[test]
    fn update_selected_in_place_members_and_no_hit() {
        let mut m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        sel.set_members([2]);
        let touched = update_selected_in_place(&mut m, &sel, |n, _k| n.velocity = 42);
        assert!(touched);
        assert_eq!(m.notes[60][0].velocity, 100, "id=1 不在成员位图");
        assert_eq!(m.notes[60][1].velocity, 42, "id=2 命中");
        assert_eq!(m.notes[64][0].velocity, 100, "id=3 不在成员位图");

        let untouched = update_selected_in_place(&mut m, &Selection::default(), |_, _| {
            panic!("空选区不应命中任何音符")
        });
        assert!(!untouched);
    }

    fn model_with_notes() -> YinModel {
        let per_track = vec![vec![
            NoteEvent {
                id: 1,
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
            },
            NoteEvent {
                id: 2,
                start_tick: 480,
                end_tick: 960,
                key: 60,
                velocity: 80,
            },
            NoteEvent {
                id: 3,
                start_tick: 0,
                end_tick: 240,
                key: 64,
                velocity: 100,
            },
        ]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        m
    }

    #[test]
    fn summarize_partial_selection() {
        let m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, 480, 60, 60);
        let s = summarize_selected(&m, &sel);
        assert_eq!(s.count, 1);
        assert_eq!(s.velocity, Some(100));
        assert_eq!(s.gate, Some(480));
        assert_eq!(s.key, Some(60));
        assert_eq!(s.tick, Some(0));
    }

    #[test]
    fn summarize_full_selection_uses_fast_path_and_mixed() {
        let m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        let s = summarize_selected(&m, &sel);
        assert_eq!(s.count, 3, "全选快路径应返回 note_count");
        assert_eq!(s.velocity, None, "velocity 100/80 混合");
        assert_eq!(s.gate, None, "gate 480/480/240 混合");
        assert_eq!(s.key, None, "key 60/64 混合");
        assert_eq!(s.tick, None, "tick 0/480 混合");
    }

    #[test]
    fn summarize_single_uniform() {
        let m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, 480, 64, 64);
        let s = summarize_selected(&m, &sel);
        assert_eq!(s.count, 1);
        assert_eq!(s.velocity, Some(100));
    }

    /// 筛选边界：remove_selected 只删匹配属性边界的音符。
    #[test]
    fn remove_selected_honors_velocity_filter() {
        let mut m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        sel.filter.velocity = Some((90, 127)); // 只匹配两个 v100

        let removed = remove_selected(&mut m, &sel);
        assert_eq!(removed.len(), 2);
        assert_eq!(m.notes[60].len(), 1, "k60 的 v80 应保留");
        assert_eq!(m.notes[60][0].velocity, 80);
        assert_eq!(m.notes[64].len(), 0, "k64 的 v100 应删除");
    }

    /// 筛选边界：gate 过滤按 end-start 区间判定。
    #[test]
    fn remove_selected_honors_gate_filter() {
        let mut m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        sel.filter.gate = Some((300, 1000)); // k60 两条 gate=480 命中；k64 gate=240 不命中

        let removed = remove_selected(&mut m, &sel);
        assert_eq!(removed.len(), 2);
        assert_eq!(m.notes[60].len(), 0);
        assert_eq!(m.notes[64].len(), 1, "短音符保留");
    }

    /// 反选：范围内不满足边界的音符被删除（即选中它们）。
    #[test]
    fn remove_selected_honors_invert() {
        let mut m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        sel.filter.velocity = Some((90, 127));
        sel.filter.invert = true;

        let removed = remove_selected(&mut m, &sel);
        assert_eq!(removed.len(), 1, "只有 v80 满足反选条件");
        assert_eq!(removed[0].0.velocity, 80);
    }

    /// 统计与删除共用同一筛选语义（Info 面板数字与操作一致）。
    #[test]
    fn summarize_honors_filter_and_disables_fast_path() {
        let m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        sel.filter.velocity = Some((90, 127));

        let s = summarize_selected(&m, &sel);
        assert_eq!(s.count, 2, "有筛选时全选快路径必须禁用");
        assert_eq!(s.velocity, Some(100));
        assert_eq!(s.gate, None, "两条命中音符 gate 480/240 混合");
    }

    /// 成员态删除：rect 覆盖范围内但不在成员位图里的音符不得被删。
    #[test]
    fn remove_selected_with_members_leaves_bystanders() {
        let mut m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, MAX_KEY);
        sel.set_members([2]); // 只选中 k60 的 id=2

        let removed = remove_selected(&mut m, &sel);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0.id, 2);
        assert_eq!(m.notes[60].len(), 1, "同 rect 内的 id=1 应保留");
        assert_eq!(m.notes[60][0].id, 1);
        assert_eq!(m.notes[64].len(), 1, "其他 key 的 id=3 应保留");
    }

    /// 回归：框选物化 → 移动提交（选区平移）→ 再次收集，
    /// 落点处同 key 同 tick 区间的路人音符不得被收编。
    #[test]
    fn materialized_selection_survives_move_without_bystanders() {
        // k60: id=1 (0..100, v100) 被框选；id=2 (200..300, v80) 落点路人
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(vec![vec![
            NoteEvent {
                id: 1,
                start_tick: 0,
                end_tick: 100,
                key: 60,
                velocity: 100,
            },
            NoteEvent {
                id: 2,
                start_tick: 200,
                end_tick: 300,
                key: 60,
                velocity: 80,
            },
        ]]);
        m.rebuild();

        let mut sel = Selection::default();
        sel.add_rect(0, 100, 60, 60);
        sel.materialize_pending(&m);
        assert_eq!(sel.explicit_member_count(), Some(1));

        // 模拟 move_selected_notes：移除成员（只删 id=1）并插到落点 200..300。
        let removed = remove_selected(&mut m, &sel);
        assert_eq!(removed.len(), 1, "成员态移动只取成员");
        assert_eq!(removed[0].0.id, 1);
        let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
        by_key.insert(
            60,
            vec![Note {
                id: 1,
                start_tick: 200,
                end_tick: 300,
                velocity: 100,
                track: 0,
            }],
        );
        insert_batch(&mut m, by_key);

        // 移动提交后选区矩形跟随平移。
        sel.offset(200, 0);

        let collected = collect_selected(&m, &sel);
        assert_eq!(collected.len(), 1, "落点路人不得进入拖动组");
        assert_eq!(collected[0].0.id, 1, "只能是被框选的 id=1");
        assert_eq!(collected[0].0.velocity, 100);
    }

    /// 重叠判定：相交/相接/跨轨/跨 key/长音符起点远早于窗口（靠 max_note_len 左扩命中）。
    #[test]
    fn has_overlapping_note_cases() {
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(1, 1)),
            ],
            ..Default::default()
        };
        m.load_track_notes(vec![
            vec![
                // track 0：普通音符 + 跨轨参照
                NoteEvent {
                    id: 0,
                    start_tick: 100,
                    end_tick: 200,
                    key: 60,
                    velocity: 100,
                },
                // track 0：k61 长音符（起点远早于查询窗口）
                NoteEvent {
                    id: 0,
                    start_tick: 0,
                    end_tick: 1000,
                    key: 61,
                    velocity: 100,
                },
            ],
            vec![NoteEvent {
                id: 0,
                start_tick: 150,
                end_tick: 250,
                key: 60,
                velocity: 100,
            }],
        ]);
        m.rebuild();

        // 同轨同 key 相交（右交/左交/包住/被包）
        assert!(has_overlapping_note(&m, 0, 60, 150, 180));
        assert!(has_overlapping_note(&m, 0, 60, 50, 150));
        assert!(has_overlapping_note(&m, 0, 60, 50, 250));
        assert!(has_overlapping_note(&m, 0, 60, 120, 130));
        // 首尾相接（左闭右开）不算重叠
        assert!(!has_overlapping_note(&m, 0, 60, 200, 300));
        assert!(!has_overlapping_note(&m, 0, 60, 0, 100));
        // 跨轨不算（track 1 的 [150,250) 与查询区间相交但轨道不同）
        assert!(!has_overlapping_note(&m, 1, 60, 500, 600));
        // 跨 key 不算
        assert!(!has_overlapping_note(&m, 0, 62, 150, 250));
        // 长音符起点远早于 start：靠 max_note_len 左扩窗口命中
        assert!(has_overlapping_note(&m, 0, 61, 900, 950));
        // 长音符结束之后的空区
        assert!(!has_overlapping_note(&m, 0, 61, 1000, 1100));
        // 零长区间防御
        assert!(!has_overlapping_note(&m, 0, 60, 150, 150));
    }
}
