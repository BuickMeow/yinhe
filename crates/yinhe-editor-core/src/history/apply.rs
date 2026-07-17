//! Undo/redo application logic.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_types::{AutomationEvent, Note};

use crate::document::Document;

use super::UndoAction;

// ---------------------------------------------------------------------------
// UndoAction apply
// ---------------------------------------------------------------------------

impl UndoAction {
    /// Apply the forward action (used by redo).
    pub fn redo(&self, doc: &mut Document) {
        match self {
            UndoAction::Notes(delta) => apply_note_delta(doc, &delta.before, &delta.after),
            UndoAction::Automation(delta) => apply_automation_delta(doc, delta.track_idx, delta.lane_idx, &delta.after),
            UndoAction::TrackName { track_idx, old: _, new } => {
                let model = Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*track_idx) {
                    let track = Arc::make_mut(track);
                    track.name = new.clone();
                }
            }
            UndoAction::ProjectName { old: _, new } => {
                doc.data.project_name = new.clone();
            }
            UndoAction::ProjectArtist { old: _, new } => {
                doc.data.project_artist = new.clone();
            }
            UndoAction::ProjectDescription { old: _, new } => {
                doc.data.project_description = new.clone();
            }
            UndoAction::ProjectPpq { old: _, new } => {
                doc.data.project_ppq = *new;
            }
            UndoAction::CompressionLevel { old: _, new } => {
                doc.data.compression_level = *new;
            }
            UndoAction::TrackStructure {
                tracks_before: _,
                tracks_after,
                note_remap,
                note_remap_inverse: _,
            } => {
                let model = Arc::make_mut(&mut doc.data.model);
                model.tracks = tracks_after.clone();
                for bucket in model.notes.iter_mut() {
                    let bucket = Arc::make_mut(bucket);
                    bucket.retain(|n| note_remap[n.track as usize] != u16::MAX);
                    for note in bucket.iter_mut() {
                        note.track = note_remap[note.track as usize];
                    }
                }
                model.rebuild();
                doc.data.bump_revision();
                doc.sync_track_caches();
            }
            UndoAction::Composite(actions) => {
                for action in actions {
                    action.redo(doc);
                }
            }
        }
    }

    /// Apply the reverse action (used by undo).
    ///
    /// Delegates to `self.reversed().redo(doc)` — the `reversed()` method
    /// swaps before/after (and reverses Composite order), so redo on the
    /// reversed action is equivalent to undo on the original.
    pub fn undo(&self, doc: &mut Document) {
        self.reversed().redo(doc);
    }
}

// ---------------------------------------------------------------------------
// Apply helpers
// ---------------------------------------------------------------------------

/// Remove `remove` notes and insert `insert` notes into the model.
///
/// Notes in `remove` are matched by (track, start_tick, key, dup_index).
pub(crate) fn apply_note_delta(doc: &mut Document, remove: &[(Note, u8)], insert: &[(Note, u8)]) {
    if remove.is_empty() && insert.is_empty() {
        return;
    }
    let model = Arc::make_mut(&mut doc.data.model);

    // Remove notes matching `remove`, grouped by key for a single retain per bucket.
    // Previous per-note retain was O(R × B); this is O(R + B).
    let mut remove_by_key: HashMap<u8, HashSet<(u16, u32, u8)>> = HashMap::new();
    for (note, key) in remove {
        remove_by_key
            .entry(*key)
            .or_default()
            .insert((note.track, note.start_tick, note.dup_index));
    }
    for (key, to_remove) in &remove_by_key {
        let k = *key as usize;
        Arc::make_mut(&mut model.notes[k]).retain(|n| {
            !to_remove.contains(&(n.track, n.start_tick, n.dup_index))
        });
        model.mark_dirty(*key);
    }

    // Insert `insert` notes, grouped by key.
    let mut by_key: HashMap<u8, Vec<Note>> = HashMap::new();
    for (note, key) in insert {
        by_key.entry(*key).or_default().push(*note);
    }
    for (key, notes) in by_key {
        let k = key as usize;
        Arc::make_mut(&mut model.notes[k]).extend(notes);
        model.mark_dirty(key);
    }

    model.rebuild_dirty();
    doc.data.bump_revision();
}

/// Replace the event list of `track_idx`'s `lane_idx` with `events`.
pub(crate) fn apply_automation_delta(
    doc: &mut Document,
    track_idx: usize,
    lane_idx: usize,
    events: &[AutomationEvent],
) {
    let model = Arc::make_mut(&mut doc.data.model);
    if let Some(track) = model.tracks.get_mut(track_idx) {
        let track = Arc::make_mut(track);
        if let Some(lane) = track.automation_lanes.get_mut(lane_idx) {
            lane.events.clear();
            lane.events.extend_from_slice(events);
            // 保持有序（编辑操作应已保证，但防御性排序）
            lane.events.sort_by_key(|e| e.tick);
        }
    }
    doc.data.bump_revision();
}
