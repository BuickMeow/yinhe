//! Undo stack, pending-edit tracking, and convenience commit helpers.

use std::collections::{HashMap, HashSet, VecDeque};

use yinhe_core::Selection;
use yinhe_types::AnchorSelRect;

use crate::document::Document;
use crate::edit_state::SelRectState;

use super::UndoAction;

// ---------------------------------------------------------------------------
// UndoEntry
// ---------------------------------------------------------------------------

/// 编辑前捕获的界面状态快照（undo/redo 恢复用）。
#[derive(Clone, Default)]
pub struct EditSnapshot {
    pub selected: Selection,
    pub track_selected: HashSet<u16>,
    pub sel_rect: SelRectState,
    /// AR 选框（f64 tick + usize track）。
    pub arr_sel_rect: Vec<(f64, f64, usize, usize)>,
    /// 每面板的 AM 选框（controller_panels 顺序）。
    pub anchor_sel_rects: Vec<Vec<AnchorSelRect>>,
}

/// A single entry on the undo/redo stack.
pub struct UndoEntry {
    pub action: UndoAction,
    pub label: String,
    pub snapshot: EditSnapshot,
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// Per-document undo/redo stack using command pattern.
///
/// Each entry stores only the delta, so memory usage is proportional to
/// the number of affected notes, not the total model size.
pub struct UndoStack {
    /// `VecDeque` 而非 `Vec`：`push` 超过 `MAX_DEPTH` 时需要弹出最旧条目，
    /// `VecDeque::pop_front` 是 O(1)，`Vec::remove(0)` 是 O(n)。
    pub(crate) past: VecDeque<UndoEntry>,
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
            past: VecDeque::new(),
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
            self.past.pop_front();
        }
        self.past.push_back(entry);
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
///
/// 快照在 push 时捕获（`doc.capture_snapshot()`）：文本类编辑不改变选区/选框，
/// 捕获时刻的界面状态即编辑前的状态。
fn commit_field<T: PartialEq>(
    doc: &mut Document,
    id: u64,
    new_value: T,
    parse_old: impl FnOnce(&str) -> T,
    make_action: impl FnOnce(T, T) -> UndoAction,
    label: &str,
) {
    let Some(old_str) = doc.edit.pending_edits.take(id) else {
        return;
    };
    let old = parse_old(&old_str);
    if old == new_value {
        return;
    }
    let snapshot = doc.capture_snapshot();
    doc.push_undo(make_action(old, new_value), label, snapshot);
}

/// Commit a track-name edit.
pub fn commit_track_name(doc: &mut Document, id: u64, track_idx: usize, new_name: &str) {
    commit_field(
        doc,
        id,
        new_name.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::TrackName {
            track_idx,
            old,
            new,
        },
        "Edit track name",
    );
}

/// Commit a project-name edit.
pub fn commit_project_name(doc: &mut Document, id: u64, new_value: &str) {
    commit_field(
        doc,
        id,
        new_value.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::ProjectName { old, new },
        "Edit project name",
    );
}

/// Commit an artist edit.
pub fn commit_artist(doc: &mut Document, id: u64, new_value: &str) {
    commit_field(
        doc,
        id,
        new_value.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::ProjectArtist { old, new },
        "Edit artist",
    );
}

/// Commit a description edit.
pub fn commit_description(doc: &mut Document, id: u64, new_value: &str) {
    commit_field(
        doc,
        id,
        new_value.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::ProjectDescription { old, new },
        "Edit description",
    );
}

/// Commit a PPQ edit.
///
/// `rescale` = true 表示此次 PPQ 变更同时 rescale 了所有音符/automation 的 tick，
/// undo/redo 需要反向 rescale 还原。
pub fn commit_ppq(doc: &mut Document, id: u64, new_value: u32, rescale: bool) {
    commit_field(
        doc,
        id,
        new_value,
        |s| s.parse().unwrap_or(480),
        |old, new| UndoAction::ProjectPpq { old, new, rescale },
        "Edit PPQ",
    );
}

/// Commit a compression-level edit.
pub fn commit_compression_level(doc: &mut Document, id: u64, new_value: i32) {
    commit_field(
        doc,
        id,
        new_value,
        |s| s.parse().unwrap_or(3),
        |old, new| UndoAction::CompressionLevel { old, new },
        "Edit zstd level",
    );
}
