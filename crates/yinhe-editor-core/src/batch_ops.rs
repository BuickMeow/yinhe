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
                        !(n.start_tick >= tick_start && n.start_tick < tick_end
                            && n.track >= track_lo && n.track <= track_hi)
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
