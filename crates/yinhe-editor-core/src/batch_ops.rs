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

use std::collections::HashMap;
use std::sync::Arc;

use yinhe_core::{Selection, YinModel};
use yinhe_types::{MAX_KEY, Note, NoteBucket};

/// Remove all notes matching `selection` from the model.
///
/// For each rect × key range, deletes `start_tick ∈ [tick_start, tick_end)`
/// within the track range, in a single pass per bucket (chunked: only hit
/// chunks are scanned). All tracks selected → contiguous `drain_range`;
/// track-filtered → `drain_range_filtered`.
///
/// Returns the removed notes with their original key, so callers can
/// re-insert them at a new position (move/transpose) or discard them (delete).
pub fn remove_selected(model: &mut YinModel, selection: &Selection) -> Vec<(Note, u8)> {
    let mut removed: Vec<(Note, u8)> = Vec::new();

    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            let bucket = Arc::make_mut(&mut model.notes[k]);
            // Fast path: all tracks selected → contiguous span drain.
            // Slow path: track-filtered → segment scan + filter.
            let out = if track_lo == 0 && track_hi == u16::MAX {
                bucket.drain_range(tick_start, tick_end)
            } else {
                bucket.drain_range_filtered(tick_start, tick_end, |n| {
                    n.track >= track_lo && n.track <= track_hi
                })
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

/// Collect notes matching `selection` from the model (read-only, no removal).
///
/// For each rect × key range, iterates `start_tick ∈ [tick_start, tick_end)`.
/// Returns `(Note, key)` pairs.
pub fn collect_selected(model: &YinModel, selection: &Selection) -> Vec<(Note, u8)> {
    let mut result: Vec<(Note, u8)> = Vec::new();

    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            for n in model.notes[k].range(tick_start, tick_end) {
                if n.track >= track_lo && n.track <= track_hi {
                    result.push((*n, key));
                }
            }
        }
    }

    result
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
/// count 带全选快路径（O(1) 返回 `note_count`，避免全选 1 亿音符时扫描）；
/// uniform 字段扫描遇到第一个不同值即短路为 None（绝大多数 mixed 情况
/// 无需遍历完整选区）。
pub fn summarize_selected(model: &YinModel, selection: &Selection) -> SelectedNoteSummary {
    // 全选快路径：单个 rect 覆盖全部 key/track 与全部 tick
    let full = selection.rects.iter().any(|&(ts, te, kl, kh, tl, th)| {
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
    for &(ts, te, kl, kh, tl, th) in &selection.rects {
        for key in kl..=kh {
            let k = key as usize;
            for n in model.notes[k].range(ts, te) {
                if n.track < tl || n.track > th {
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
}
