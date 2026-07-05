//! Undo/redo history using command pattern.
//!
//! Instead of storing full snapshots (which cost O(model) memory per entry),
//! each undo entry stores only the delta — what changed. For note operations
//! this is the before/after state of the affected notes, typically a few
//! hundred bytes instead of hundreds of megabytes.

use std::collections::{HashMap, HashSet};

use yinhe_core::Selection;
use yinhe_types::{AutomationEvent, Note};

use crate::document::Document;
use crate::edit_state::SelRectState;

/// Maximum number of past edits kept in the undo stack.
pub const MAX_DEPTH: usize = 100;

// ---------------------------------------------------------------------------
// Delta types
// ---------------------------------------------------------------------------

/// Before/after state of affected notes for a single operation.
///
/// `before` = notes as they were before the edit (at their original positions).
/// `after`  = notes as they are after the edit (at their new positions).
///
/// For a delete: `before = removed`, `after = []`.
/// For an add:    `before = []`,      `after = added`.
/// For a move:    `before = originals`, `after = moved`.
#[derive(Clone, Debug)]
pub struct NoteDelta {
    pub before: Vec<(Note, u8)>,
    pub after: Vec<(Note, u8)>,
}

/// Automation lane before/after snapshot.
///
/// Stores the full event list of the affected lane before and after the edit.
/// Automation lanes typically contain few events (hundreds at most), so
/// full-snapshot undo is simpler and cheaper than per-event deltas.
#[derive(Clone, Debug)]
pub struct AutomationDelta {
    pub track_idx: usize,
    pub lane_idx: usize,
    pub before: Vec<AutomationEvent>,
    pub after: Vec<AutomationEvent>,
}

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// What changed — the delta needed to undo/redo an operation.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// Note-level changes (delete, add, move, resize, duplicate, transpose).
    Notes(NoteDelta),
    /// Automation lane event changes (add/move/delete/shape).
    Automation(AutomationDelta),
    /// A track name was edited.
    TrackName {
        track_idx: usize,
        old: String,
        new: String,
    },
    /// Project metadata was edited.
    ProjectName { old: String, new: String },
    ProjectArtist { old: String, new: String },
    ProjectDescription { old: String, new: String },
    ProjectPpq { old: u32, new: u32 },
    CompressionLevel { old: i32, new: i32 },
}

impl UndoAction {
    /// Apply the forward action (used by redo).
    pub fn redo(&self, doc: &mut Document) {
        match self {
            UndoAction::Notes(delta) => apply_note_delta(doc, &delta.before, &delta.after),
            UndoAction::Automation(delta) => apply_automation_delta(doc, delta.track_idx, delta.lane_idx, &delta.after),
            UndoAction::TrackName { track_idx, old: _, new } => {
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*track_idx) {
                    let track = std::sync::Arc::make_mut(track);
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
        }
    }

    /// Apply the reverse action (used by undo).
    pub fn undo(&self, doc: &mut Document) {
        match self {
            UndoAction::Notes(delta) => apply_note_delta(doc, &delta.after, &delta.before),
            UndoAction::Automation(delta) => apply_automation_delta(doc, delta.track_idx, delta.lane_idx, &delta.before),
            UndoAction::TrackName { track_idx, old, new: _ } => {
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*track_idx) {
                    let track = std::sync::Arc::make_mut(track);
                    track.name = old.clone();
                }
            }
            UndoAction::ProjectName { old, new: _ } => {
                doc.data.project_name = old.clone();
            }
            UndoAction::ProjectArtist { old, new: _ } => {
                doc.data.project_artist = old.clone();
            }
            UndoAction::ProjectDescription { old, new: _ } => {
                doc.data.project_description = old.clone();
            }
            UndoAction::ProjectPpq { old, new: _ } => {
                doc.data.project_ppq = *old;
            }
            UndoAction::CompressionLevel { old, new: _ } => {
                doc.data.compression_level = *old;
            }
        }
    }

    /// Return the inverse action (swap before/after, old/new).
    pub fn reversed(&self) -> Self {
        match self {
            UndoAction::Notes(delta) => UndoAction::Notes(NoteDelta {
                before: delta.after.clone(),
                after: delta.before.clone(),
            }),
            UndoAction::Automation(delta) => UndoAction::Automation(AutomationDelta {
                track_idx: delta.track_idx,
                lane_idx: delta.lane_idx,
                before: delta.after.clone(),
                after: delta.before.clone(),
            }),
            UndoAction::TrackName { track_idx, old, new } => UndoAction::TrackName {
                track_idx: *track_idx,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::ProjectName { old, new } => UndoAction::ProjectName {
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::ProjectArtist { old, new } => UndoAction::ProjectArtist {
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::ProjectDescription { old, new } => UndoAction::ProjectDescription {
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::ProjectPpq { old, new } => UndoAction::ProjectPpq {
                old: *new,
                new: *old,
            },
            UndoAction::CompressionLevel { old, new } => UndoAction::CompressionLevel {
                old: *new,
                new: *old,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Apply helpers
// ---------------------------------------------------------------------------

/// Remove `remove` notes and insert `insert` notes into the model.
///
/// Notes in `remove` are matched by (track, start_tick, key, dup_index).
fn apply_note_delta(doc: &mut Document, remove: &[(Note, u8)], insert: &[(Note, u8)]) {
    if remove.is_empty() && insert.is_empty() {
        return;
    }
    let model = std::sync::Arc::make_mut(&mut doc.data.model);

    // Remove notes matching `remove`.
    for (note, key) in remove {
        let k = *key as usize;
        std::sync::Arc::make_mut(&mut model.notes[k]).retain(|n| {
            !(n.track == note.track
                && n.start_tick == note.start_tick
                && n.dup_index == note.dup_index)
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
        std::sync::Arc::make_mut(&mut model.notes[k]).extend(notes);
        model.mark_dirty(key);
    }

    model.rebuild_dirty();
    doc.data.midi_version = doc.data.midi_version.wrapping_add(1);
}

/// Replace the event list of `track_idx`'s `lane_idx` with `events`.
fn apply_automation_delta(
    doc: &mut Document,
    track_idx: usize,
    lane_idx: usize,
    events: &[AutomationEvent],
) {
    let model = std::sync::Arc::make_mut(&mut doc.data.model);
    if let Some(track) = model.tracks.get_mut(track_idx) {
        let track = std::sync::Arc::make_mut(track);
        if let Some(lane) = track.automation_lanes.get_mut(lane_idx) {
            lane.events.clear();
            lane.events.extend_from_slice(events);
            // 保持有序（编辑操作应已保证，但防御性排序）
            lane.events.sort_by_key(|e| e.tick);
        }
    }
    doc.data.midi_version = doc.data.midi_version.wrapping_add(1);
}

// ---------------------------------------------------------------------------
// UndoEntry
// ---------------------------------------------------------------------------

/// A single entry on the undo/redo stack.
pub struct UndoEntry {
    pub action: UndoAction,
    pub label: &'static str,
    pub selected: Selection,
    pub track_selected: HashSet<u16>,
    pub sel_rect: SelRectState,
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// Per-document undo/redo stack using command pattern.
///
/// Each entry stores only the delta, so memory usage is proportional to
/// the number of affected notes, not the total model size.
pub struct UndoStack {
    pub(crate) past: Vec<UndoEntry>,
    pub(crate) future: Vec<UndoEntry>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    /// Record an undo entry (called *after* the edit is done).
    pub fn push(&mut self, entry: UndoEntry) {
        if self.past.len() >= MAX_DEPTH {
            self.past.remove(0);
        }
        self.past.push(entry);
        self.future.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PendingEdits — tracks old values for text-field edits
// ---------------------------------------------------------------------------

/// Tracks old values for TextEdit-like fields.
/// On `commit`, the old value is compared with the current value and an
/// `UndoAction` is pushed if they differ.
#[derive(Default)]
pub struct PendingEdits {
    map: HashMap<u64, String>,
}

impl PendingEdits {
    pub fn has(&self, id: u64) -> bool {
        self.map.contains_key(&id)
    }

    /// Save the old value before a text edit begins.
    pub fn begin(&mut self, id: u64, old_value: &str) {
        self.map.insert(id, old_value.to_string());
    }

    /// Take the saved old value without removing it (for comparison).
    pub fn get(&self, id: u64) -> Option<&str> {
        self.map.get(&id).map(|s| s.as_str())
    }

    /// Remove and return the saved old value.
    pub fn take(&mut self, id: u64) -> Option<String> {
        self.map.remove(&id)
    }
}

// ---------------------------------------------------------------------------
// Convenience helpers for text-field edits
// ---------------------------------------------------------------------------

/// Begin tracking a TextEdit/DragValue keyed by `id`.
pub fn begin_edit(pending: &mut PendingEdits, id: u64, old_value: &str) {
    pending.begin(id, old_value);
}

/// Commit a track-name edit.
pub fn commit_track_name(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    track_idx: usize,
    new_name: &str,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old) = pending.take(id) else {
        return;
    };
    if old == new_name {
        return;
    }
    stack.push(UndoEntry {
        action: UndoAction::TrackName {
            track_idx,
            old,
            new: new_name.to_string(),
        },
        label: "Edit track name",
        selected,
        track_selected,
        sel_rect,
    });
}

/// Commit a project-name edit.
pub fn commit_project_name(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    new_value: &str,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old) = pending.take(id) else {
        return;
    };
    if old == new_value {
        return;
    }
    stack.push(UndoEntry {
        action: UndoAction::ProjectName {
            old,
            new: new_value.to_string(),
        },
        label: "Edit project name",
        selected,
        track_selected,
        sel_rect,
    });
}

/// Commit an artist edit.
pub fn commit_artist(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    new_value: &str,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old) = pending.take(id) else {
        return;
    };
    if old == new_value {
        return;
    }
    stack.push(UndoEntry {
        action: UndoAction::ProjectArtist {
            old,
            new: new_value.to_string(),
        },
        label: "Edit artist",
        selected,
        track_selected,
        sel_rect,
    });
}

/// Commit a description edit.
pub fn commit_description(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    new_value: &str,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old) = pending.take(id) else {
        return;
    };
    if old == new_value {
        return;
    }
    stack.push(UndoEntry {
        action: UndoAction::ProjectDescription {
            old,
            new: new_value.to_string(),
        },
        label: "Edit description",
        selected,
        track_selected,
        sel_rect,
    });
}

/// Commit a PPQ edit.
pub fn commit_ppq(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    new_value: u32,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old_str) = pending.take(id) else {
        return;
    };
    let old: u32 = old_str.parse().unwrap_or(480);
    if old == new_value {
        return;
    }
    stack.push(UndoEntry {
        action: UndoAction::ProjectPpq { old, new: new_value },
        label: "Edit PPQ",
        selected,
        track_selected,
        sel_rect,
    });
}

/// Commit a compression-level edit.
pub fn commit_compression_level(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    new_value: i32,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old_str) = pending.take(id) else {
        return;
    };
    let old: i32 = old_str.parse().unwrap_or(3);
    if old == new_value {
        return;
    }
    stack.push(UndoEntry {
        action: UndoAction::CompressionLevel {
            old,
            new: new_value,
        },
        label: "Edit zstd level",
        selected,
        track_selected,
        sel_rect,
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_core::{ConductorData, NoteEvent, TempoEvent, TrackData, YinModel};
    use yinhe_types::TimeSigEvent;

    fn make_doc(name: &str) -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData {
                tempo: vec![TempoEvent { tick: 0, bpm: 120.0 }],
                time_sig: vec![TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                }],
            }),
            tracks: vec![Arc::new({
                let mut t = TrackData::new(0, 0);
                t.name = name.to_string();
                t
            })],
            ..Default::default()
        };
        Document {
            data: crate::project_data::ProjectData::new(
                Arc::new(model),
                vec![name.to_string()],
                Default::default(),
                Default::default(),
            ),
            edit: crate::edit_state::EditState {
                track_visible: vec![true],
                track_pianoroll_visible: vec![true],
                ..Default::default()
            },
            history: UndoStack::new(),
            file_name: "test".into(),
            file_path: None,
        }
    }

    #[test]
    fn push_stores_and_clears_redo() {
        let mut doc = make_doc("a");
        doc.history.push(UndoEntry {
            action: UndoAction::TrackName {
                track_idx: 0,
                old: "a".into(),
                new: "b".into(),
            },
            label: "rename",
            selected: Selection::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });
        assert!(doc.history.can_undo());
        assert!(!doc.history.can_redo());

        doc.undo();
        assert!(!doc.history.can_undo());
        assert!(doc.history.can_redo());

        doc.history.push(UndoEntry {
            action: UndoAction::TrackName {
                track_idx: 0,
                old: "c".into(),
                new: "d".into(),
            },
            label: "rename2",
            selected: Selection::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });
        assert!(!doc.history.can_redo());
        assert!(doc.history.can_undo());
    }

    #[test]
    fn undo_restores_track_name() {
        let mut doc = make_doc("old");
        doc.history.push(UndoEntry {
            action: UndoAction::TrackName {
                track_idx: 0,
                old: "old".into(),
                new: "new".into(),
            },
            label: "rename",
            selected: Selection::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });
        // Apply the forward action manually (simulating the edit)
        {
            let model = Arc::make_mut(&mut doc.data.model);
            let track = Arc::make_mut(&mut model.tracks[0]);
            track.name = "new".into();
        }
        assert_eq!(doc.data.model.tracks[0].name, "new");

        // Undo
        assert!(doc.undo());
        assert_eq!(doc.data.model.tracks[0].name, "old");
        assert!(doc.history.can_redo());

        // Redo
        assert!(doc.redo());
        assert_eq!(doc.data.model.tracks[0].name, "new");
        assert!(doc.history.can_undo());
    }

    #[test]
    fn note_delta_undo_redo() {
        let mut doc = make_doc("test");
        // Add a note
        let note = NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            dup_index: 0,
        };
        let key = 60;
        {
            let model = Arc::make_mut(&mut doc.data.model);
            Arc::make_mut(&mut model.notes[key as usize]).push(yinhe_types::Note {
                start_tick: note.start_tick,
                end_tick: note.end_tick,
                velocity: note.velocity,
                dup_index: note.dup_index,
                track: 0,
            });
            model.mark_dirty(key);
            model.rebuild_dirty();
        }

        let removed = {
            let model = Arc::make_mut(&mut doc.data.model);
            let mut sel = Selection::default();
            sel.add_rect_track(0, 480, 60, 60, 0, u16::MAX);
            let r = crate::batch_ops::remove_selected(
                model,
                &sel,
            );
            model.rebuild_dirty();
            r
        };
        assert_eq!(removed.len(), 1);

        doc.history.push(UndoEntry {
            action: UndoAction::Notes(NoteDelta {
                before: removed,
                after: vec![],
            }),
            label: "delete",
            selected: Selection::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });

        // Note should be gone
        assert!(doc.data.model.notes[60].is_empty());

        // Undo
        assert!(doc.undo());
        assert_eq!(doc.data.model.notes[60].len(), 1);
        assert_eq!(doc.data.model.notes[60][0].start_tick, 0);

        // Redo
        assert!(doc.redo());
        assert!(doc.data.model.notes[60].is_empty());
    }

    #[test]
    fn undo_returns_none_when_empty() {
        let mut doc = make_doc("x");
        assert!(!doc.undo());
    }

    #[test]
    fn redo_returns_none_when_empty() {
        let mut doc = make_doc("x");
        assert!(!doc.redo());
    }

    #[test]
    fn clear_wipes_everything() {
        let mut doc = make_doc("a");
        doc.history.push(UndoEntry {
            action: UndoAction::TrackName {
                track_idx: 0,
                old: "a".into(),
                new: "b".into(),
            },
            label: "rename",
            selected: Selection::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });
        doc.undo();
        assert!(doc.history.can_undo() || doc.history.can_redo());

        doc.history.clear();
        assert!(!doc.history.can_undo());
        assert!(!doc.history.can_redo());
        assert_eq!(doc.history.past.len(), 0);
        assert_eq!(doc.history.future.len(), 0);
    }
}
