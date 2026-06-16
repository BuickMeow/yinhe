//! Undo/redo history for a single document.
//!
//! Stores `UndoSnapshot`s containing a full `ProjectData` clone.
//! Cloning is cheap: `Arc<MidiFile>` is O(1), metrics/names/metadata are small.
//! Actual data copy only happens when `Arc::make_mut` is called (copy-on-write).
//!
//! Selection, scroll, mute/solo and other UI state are intentionally NOT
//! captured — undo restores notes and names, not the user's cursor.

use std::collections::{HashMap, VecDeque};

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
    let snap = data.snapshot(label);
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
