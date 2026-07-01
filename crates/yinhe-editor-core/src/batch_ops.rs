//! Unified batch operations on notes.
//!
//! All large-scale note edits (delete, move, duplicate, transpose) share
//! the same pattern: group by key bucket, single `retain` per bucket for
//! removal, single `extend` per bucket for insertion, then `mark_dirty` +
//! `rebuild_dirty`. This module centralizes that pattern.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_core::YinModel;
use yinhe_types::Note;

/// Selection identity: (track, start_tick, key).
pub type SelId = (u16, u32, u8);

/// Group a selection by key bucket.
/// Returns `HashMap<key, Vec<(track, start_tick)>>`.
pub fn group_by_key(selected: &HashSet<SelId>) -> HashMap<u8, Vec<(u16, u32)>> {
    let mut by_key: HashMap<u8, Vec<(u16, u32)>> = HashMap::new();
    for &(track, start_tick, key) in selected {
        by_key.entry(key).or_default().push((track, start_tick));
    }
    by_key
}

/// Remove all notes matching `selected` from the model.
///
/// For each key bucket, does a single `retain` pass (O(B) per bucket, not
/// O(S × B)). Marks each touched bucket dirty.
///
/// Returns the removed notes with their original key, so callers can
/// re-insert them at a new position (move/transpose) or discard them
/// (delete).
pub fn remove_selected(model: &mut YinModel, selected: &HashSet<SelId>) -> Vec<(Note, u8)> {
    let by_key = group_by_key(selected);
    let mut removed: Vec<(Note, u8)> = Vec::new();

    for (key, removals) in &by_key {
        let k = *key as usize;
        let removal_set: HashSet<(u16, u32)> = removals.iter().copied().collect();

        // Collect the notes we're about to remove (for move/transpose).
        for n in model.notes[k].iter() {
            if removal_set.contains(&(n.track, n.start_tick)) {
                removed.push((*n, *key));
            }
        }

        Arc::make_mut(&mut model.notes[k]).retain(|n| !removal_set.contains(&(n.track, n.start_tick)));
        model.mark_dirty(*key);
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

/// Collect notes matching `selected` from the model (read-only, no removal).
///
/// Uses `partition_point` for O(log B) lookup per note.
/// Returns `(Note, key)` pairs.
pub fn collect_selected(model: &YinModel, selected: &HashSet<SelId>) -> Vec<(Note, u8)> {
    let by_key = group_by_key(selected);
    let mut result: Vec<(Note, u8)> = Vec::new();

    for (key, picks) in &by_key {
        let k = *key as usize;
        let bucket = &model.notes[k];
        for (track, start_tick) in picks {
            let idx = bucket.partition_point(|n| n.start_tick < *start_tick);
            if let Some(note) = bucket[idx..].iter().find(|n| n.track == *track && n.start_tick == *start_tick) {
                result.push((*note, *key));
            }
        }
    }

    result
}
