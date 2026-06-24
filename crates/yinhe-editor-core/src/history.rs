//! Undo/redo history for a single document.
//!
//! Stores `UndoSnapshot`s containing a full `ProjectData` clone.
//! Cloning is cheap: `Arc<MidiFile>` is O(1), metrics/names/metadata are small.
//! Actual data copy only happens when `Arc::make_mut` is called (copy-on-write).
//!
//! Selection, scroll, mute/solo and other UI state are intentionally NOT
//! captured — undo restores notes and names, not the user's cursor.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::project_data::ProjectData;

/// Maximum number of past edits kept in the undo stack.
pub const MAX_DEPTH: usize = 100;

/// One undo/redo snapshot. Contains the full persistent project state.
/// Clone cost: Arc::clone (~16B) + metrics (~100KB) + names/metadata (~few hundred B).
#[derive(Clone)]
pub struct UndoSnapshot {
    pub data: ProjectData,
    /// Short label for debugging / future UI ("Delete notes", "Move notes", …).
    pub label: &'static str,
    /// Selected notes at the time of the snapshot.
    pub selected: HashSet<(u16, u32, u8)>,
}

/// Per-document undo/redo stack.
///
/// `past` holds states *before* each completed edit, oldest at the front.
/// `future` holds states that have been undone, ready to be redone.
pub struct UndoStack {
    past: VecDeque<UndoSnapshot>,
    future: Vec<UndoSnapshot>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: VecDeque::new(),
            future: Vec::new(),
        }
    }

    /// Record a snapshot of the state *before* an edit. Clears the redo stack.
    pub fn push(&mut self, snapshot: UndoSnapshot) {
        if self.past.len() >= MAX_DEPTH {
            self.past.pop_front();
        }
        self.past.push_back(snapshot);
        self.future.clear();
    }

    /// Pop the most recent past snapshot, pushing `current` onto the redo stack.
    /// Returns `None` when there is nothing to undo.
    pub fn undo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        let prev = self.past.pop_back()?;
        self.future.push(current);
        Some(prev)
    }

    /// Pop the most recent future snapshot, pushing `current` onto the undo stack.
    /// Returns `None` when there is nothing to redo.
    pub fn redo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        let next = self.future.pop()?;
        self.past.push_back(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Wipe all history. Call when switching to a fresh document.
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

/// Tracks per-widget "before-edit" snapshots for TextEdit-like fields.
///
/// Pattern:
/// 1. On `resp.gained_focus()` call `begin_edit(data, id, label)` — stashes
///    a snapshot of the project data at the moment focus was gained.
/// 2. On `resp.lost_focus()` (or Enter) call `commit_edit(data, stack, id)` —
///    if the data actually changed since `begin`, the stashed snapshot is
///    pushed onto the undo stack. Otherwise it is discarded.
#[derive(Default)]
pub struct PendingEdits {
    map: HashMap<u64, UndoSnapshot>,
}

impl PendingEdits {
    pub fn has(&self, id: u64) -> bool {
        self.map.contains_key(&id)
    }

    pub fn insert_raw(&mut self, id: u64, snapshot: UndoSnapshot) {
        self.map.insert(id, snapshot);
    }

    pub fn take(&mut self, id: u64) -> Option<UndoSnapshot> {
        self.map.remove(&id)
    }
}

/// Begin tracking a TextEdit/DragValue keyed by `id`.
/// Captures a baseline snapshot of `data` (overwriting any previous one).
pub fn begin_edit(data: &ProjectData, pending: &mut PendingEdits, id: u64, label: &'static str) {
    let snap = UndoSnapshot {
        data: data.clone(),
        label,
        selected: HashSet::new(),
    };
    pending.insert_raw(id, snap);
}

/// Commit a TextEdit/DragValue keyed by `id`.
/// If a baseline exists and the data diverged from it, the baseline
/// is pushed onto the undo stack. Otherwise the baseline is silently discarded.
pub fn commit_edit(
    data: &ProjectData,
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
) {
    let Some(baseline) = pending.take(id) else {
        return;
    };
    let changed = baseline.data.project_name != data.project_name
        || baseline.data.project_artist != data.project_artist
        || baseline.data.project_description != data.project_description
        || baseline.data.project_ppq != data.project_ppq
        || baseline.data.compression_level != data.compression_level
        || baseline.data.track_names != data.track_names;
    if changed {
        stack.push(baseline);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yinhe_core::YinModel;
    use yinhe_yin::{MappingFile, ProjectFile};

    fn make_test_data(name: &str) -> ProjectData {
        let model = YinModel::default();
        let mut pf = ProjectFile::default();
        pf.name = name.to_string();
        let mut data = ProjectData::new(
            Arc::new(model),
            Vec::new(),
            pf,
            MappingFile::default(),
        );
        data.project_name = name.to_string();
        data
    }

    fn snap(label: &'static str, name: &str) -> UndoSnapshot {
        UndoSnapshot {
            data: make_test_data(name),
            label,
            selected: HashSet::new(),
        }
    }

    #[test]
    fn push_stores_and_clears_redo() {
        let mut stack = UndoStack::new();
        stack.push(snap("init", "a"));
        assert!(stack.can_undo());
        // Simulate an undo to populate the redo stack
        stack.undo(snap("cur", "b"));
        assert!(stack.can_redo());
        // Push should clear redo
        stack.push(snap("new", "c"));
        assert!(!stack.can_redo());
        assert!(stack.can_undo());
    }

    #[test]
    fn undo_returns_previous_and_pushes_to_future() {
        let mut stack = UndoStack::new();
        stack.push(snap("init", "old"));
        let current = snap("cur", "current");
        let prev = stack.undo(current);
        assert!(prev.is_some());
        assert_eq!(prev.unwrap().data.project_name, "old");
        assert!(stack.can_redo());
    }

    #[test]
    fn redo_returns_future_and_pushes_to_past() {
        let mut stack = UndoStack::new();
        stack.push(snap("init", "a"));
        // undo pushes current ("b") to future, returns previous ("a")
        stack.undo(snap("cur", "b"));
        // redo pops future ("b"), pushes current ("a") to past, returns "b"
        let current = snap("after_undo", "a");
        let next = stack.redo(current);
        assert!(next.is_some());
        assert_eq!(next.unwrap().data.project_name, "b");
        assert!(stack.can_undo());
    }

    #[test]
    fn undo_returns_none_when_empty() {
        let mut stack = UndoStack::new();
        let result = stack.undo(snap("cur", "x"));
        assert!(result.is_none());
    }

    #[test]
    fn redo_returns_none_when_empty() {
        let mut stack = UndoStack::new();
        let result = stack.redo(snap("cur", "x"));
        assert!(result.is_none());
    }

    #[test]
    fn can_undo_can_redo_correctness() {
        let mut stack = UndoStack::new();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());

        stack.push(snap("1", "a"));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());

        stack.undo(snap("cur", "b"));
        assert!(!stack.can_undo());
        assert!(stack.can_redo());

        stack.redo(snap("cur2", "b"));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn push_beyond_max_depth_evicts_oldest() {
        let mut stack = UndoStack::new();
        for i in 0..=MAX_DEPTH {
            stack.push(snap("fill", &format!("item_{i}")));
        }
        assert_eq!(stack.past.len(), MAX_DEPTH);
        // The oldest item should have been evicted
        let oldest = stack.past.front().unwrap();
        assert_eq!(oldest.data.project_name, "item_1");
    }

    #[test]
    fn clear_wipes_everything() {
        let mut stack = UndoStack::new();
        stack.push(snap("1", "a"));
        stack.undo(snap("2", "b"));
        assert!(stack.can_undo() || stack.can_redo());

        stack.clear();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
        assert_eq!(stack.past.len(), 0);
        assert_eq!(stack.future.len(), 0);
    }
}
