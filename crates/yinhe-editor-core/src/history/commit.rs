//! Undo stack, pending-edit tracking, and convenience commit helpers.

use std::collections::{HashMap, HashSet};

use yinhe_core::Selection;

use crate::edit_state::SelRectState;

use super::UndoAction;

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
    /// Length of `past` at the time of the last save.
    /// `is_dirty()` compares current `past.len()` against this value.
    pub(crate) saved_past_len: usize,
    /// Whether the document has an established "saved base state".
    /// - `true` for a fresh empty document (closing without save is fine)
    /// - `false` after loading a file (closing without save should prompt)
    /// - Set to `true` after first save or mark_loaded()
    pub(crate) has_saved_base: bool,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            saved_past_len: 0,
            // New empty document is considered "saved base" — closing without save is fine.
            has_saved_base: true,
        }
    }

    /// Whether the document has unsaved changes.
    /// Returns true if:
    /// - There are edits since last save (`past.len() != saved_past_len`), OR
    /// - The document was loaded from a file but never saved (`!has_saved_base`)
    pub fn is_dirty(&self) -> bool {
        self.past.len() != self.saved_past_len || !self.has_saved_base
    }

    /// Mark the current state as saved (called after a successful save).
    pub fn mark_saved(&mut self) {
        self.saved_past_len = self.past.len();
        self.has_saved_base = true;
    }

    /// Mark that this document was loaded from a file (not a fresh empty doc).
    /// Called after loading MIDI/.yin. Sets `has_saved_base = false` so that
    /// closing without save will prompt the user.
    pub fn mark_loaded(&mut self) {
        self.saved_past_len = 0;
        self.has_saved_base = false;
    }

    /// Record an undo entry (called *after* the edit is done).
    pub fn push(&mut self, entry: UndoEntry) {
        if self.past.len() >= super::MAX_DEPTH {
            self.past.remove(0);
        }
        self.past.push(entry);
        self.future.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Number of entries in the undo stack (public read access).
    pub fn past_len(&self) -> usize {
        self.past.len()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
        self.saved_past_len = 0;
        self.has_saved_base = true; // Reset to "fresh empty document" state
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

/// Generic commit: take old value from pending, compare with new, push undo entry if changed.
fn commit_field<T: PartialEq>(
    stack: &mut UndoStack,
    pending: &mut PendingEdits,
    id: u64,
    new_value: T,
    parse_old: impl FnOnce(&str) -> T,
    make_action: impl FnOnce(T, T) -> UndoAction,
    label: &'static str,
    selected: Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
) {
    let Some(old_str) = pending.take(id) else { return; };
    let old = parse_old(&old_str);
    if old == new_value { return; }
    stack.push(UndoEntry {
        action: make_action(old, new_value),
        label,
        selected,
        track_selected,
        sel_rect,
    });
}

/// Commit a track-name edit.
pub fn commit_track_name(
    stack: &mut UndoStack, pending: &mut PendingEdits, id: u64,
    track_idx: usize, new_name: &str,
    selected: Selection, track_selected: HashSet<u16>, sel_rect: SelRectState,
) {
    commit_field(
        stack, pending, id, new_name.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::TrackName { track_idx, old, new },
        "Edit track name", selected, track_selected, sel_rect,
    );
}

/// Commit a project-name edit.
pub fn commit_project_name(
    stack: &mut UndoStack, pending: &mut PendingEdits, id: u64,
    new_value: &str,
    selected: Selection, track_selected: HashSet<u16>, sel_rect: SelRectState,
) {
    commit_field(
        stack, pending, id, new_value.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::ProjectName { old, new },
        "Edit project name", selected, track_selected, sel_rect,
    );
}

/// Commit an artist edit.
pub fn commit_artist(
    stack: &mut UndoStack, pending: &mut PendingEdits, id: u64,
    new_value: &str,
    selected: Selection, track_selected: HashSet<u16>, sel_rect: SelRectState,
) {
    commit_field(
        stack, pending, id, new_value.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::ProjectArtist { old, new },
        "Edit artist", selected, track_selected, sel_rect,
    );
}

/// Commit a description edit.
pub fn commit_description(
    stack: &mut UndoStack, pending: &mut PendingEdits, id: u64,
    new_value: &str,
    selected: Selection, track_selected: HashSet<u16>, sel_rect: SelRectState,
) {
    commit_field(
        stack, pending, id, new_value.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::ProjectDescription { old, new },
        "Edit description", selected, track_selected, sel_rect,
    );
}

/// Commit a PPQ edit.
pub fn commit_ppq(
    stack: &mut UndoStack, pending: &mut PendingEdits, id: u64,
    new_value: u32,
    selected: Selection, track_selected: HashSet<u16>, sel_rect: SelRectState,
) {
    commit_field(
        stack, pending, id, new_value,
        |s| s.parse().unwrap_or(480),
        |old, new| UndoAction::ProjectPpq { old, new },
        "Edit PPQ", selected, track_selected, sel_rect,
    );
}

/// Commit a compression-level edit.
pub fn commit_compression_level(
    stack: &mut UndoStack, pending: &mut PendingEdits, id: u64,
    new_value: i32,
    selected: Selection, track_selected: HashSet<u16>, sel_rect: SelRectState,
) {
    commit_field(
        stack, pending, id, new_value,
        |s| s.parse().unwrap_or(3),
        |old, new| UndoAction::CompressionLevel { old, new },
        "Edit zstd level", selected, track_selected, sel_rect,
    );
}
