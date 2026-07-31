//! Unified batch operations on notes.
//!
//! All large-scale note edits (delete, move, duplicate, transpose) share
//! the same pattern: group by key bucket, single `retain` per bucket for
//! removal, single `extend` per bucket for insertion, then `mark_dirty` +
//! `rebuild_dirty`. This module centralizes that pattern.
//!
//! Selection is always rectangular (from marquee). For each rect, iterate
//! the key range and use `partition_point` to find the tick range, then
//! `drain` or collect in a single pass per bucket.

use std::collections::HashMap;
use std::sync::Arc;

use yinhe_core::{Selection, YinModel};
use yinhe_types::Note;

/// Remove all notes matching `selection` from the model.
///
/// For each rect × key range, uses `partition_point` to locate the tick
/// range, then `drain` in a single O(B) pass per bucket.
///
/// Returns the removed notes with their original key, so callers can
/// re-insert them at a new position (move/transpose) or discard them (delete).
pub fn remove_selected(model: &mut YinModel, selection: &Selection) -> Vec<(Note, u8)> {
    let mut removed: Vec<(Note, u8)> = Vec::new();

    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            let start_idx = model.notes[k].partition_point(|n| n.start_tick < tick_start);
            let end_idx = model.notes[k].partition_point(|n| n.start_tick < tick_end);

            // Collect removed notes before clearing.
            for n in &model.notes[k][start_idx..end_idx] {
                if n.track >= track_lo && n.track <= track_hi {
                    removed.push((*n, key));
                }
            }

            if start_idx < end_idx {
                let bucket = Arc::make_mut(&mut model.notes[k]);
                // Fast path: all tracks selected → contiguous drain (memmove).
                // Slow path: track-filtered → retain (full scan).
                if track_lo == 0 && track_hi == u16::MAX {
                    bucket.drain(start_idx..end_idx);
                } else {
                    bucket.retain(|n| {
                        !(n.start_tick >= tick_start
                            && n.start_tick < tick_end
                            && n.track >= track_lo
                            && n.track <= track_hi)
                    });
                }
                model.mark_dirty(key);
            }
        }
    }

    removed
}

/// Insert notes into the model, grouped by destination key.
///
/// For each key bucket, does a single `extend` (O(N) append, no per-note
/// `insert` shifting). Marks each touched bucket dirty. The caller is
/// responsible for calling `rebuild_dirty()` afterwards.
pub fn insert_batch(model: &mut YinModel, notes_by_key: HashMap<u8, Vec<Note>>) {
    for (key, notes) in notes_by_key {
        let k = key as usize;
        Arc::make_mut(&mut model.notes[k]).extend(notes);
        model.mark_dirty(key);
    }
}

/// Collect notes matching `selection` from the model (read-only, no removal).
///
/// For each rect × key range, uses `partition_point` to find the tick range.
/// Returns `(Note, key)` pairs.
pub fn collect_selected(model: &YinModel, selection: &Selection) -> Vec<(Note, u8)> {
    let mut result: Vec<(Note, u8)> = Vec::new();

    for &(tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) in &selection.rects {
        for key in key_lo..=key_hi {
            let k = key as usize;
            let start_idx = model.notes[k].partition_point(|n| n.start_tick < tick_start);
            let end_idx = model.notes[k].partition_point(|n| n.start_tick < tick_end);
            for n in &model.notes[k][start_idx..end_idx] {
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
            && kh == 127
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
            let bucket = &model.notes[k];
            let lo = bucket.partition_point(|n| n.start_tick < ts);
            let hi = bucket.partition_point(|n| n.start_tick < te);
            for n in &bucket[lo..hi] {
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
        sel.add_rect(0, u32::MAX, 0, 127);
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
