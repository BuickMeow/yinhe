//! Undo/redo history for a single document.
//!
//! Uses a two-phase approach:
//!
//! **Phase 1** (immediate, O(1)): `push()` stores a `ProjectData` clone (Arc bump
//! only) in a pending queue. The caller returns immediately and the user sees the
//! edit result on the next frame.
//!
//! **Phase 2** (deferred, O(N)): `compress_one()` is called once per frame from
//! the UI loop. It takes one pending snapshot, serializes + zstd-compresses it,
//! then drops the `ProjectData` — releasing the `Arc<YinModel>` so subsequent
//! edits can mutate in-place without deep copies.
//!
//! This avoids both the O(N) pause on the edit path and the memory blow-up
//! from keeping uncompressed `Arc<YinModel>` snapshots alive.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::edit_state::SelRectState;
use crate::project_data::ProjectData;

/// Maximum number of past edits kept in the undo stack.
pub const MAX_DEPTH: usize = 100;

/// One undo/redo snapshot. Returned by `snapshot_with_selection()` and
/// consumed by `UndoStack::push()` / returned by `undo()` / `redo()`.
///
/// The `data` field is a full `ProjectData` clone (Arc bump only). When pushed
/// onto the stack it is stored uncompressed in a pending queue and compressed
/// asynchronously by `compress_one()`.
#[derive(Clone)]
pub struct UndoSnapshot {
    pub data: ProjectData,
    /// Short label for debugging / future UI ("Delete notes", "Move notes", …).
    pub label: &'static str,
    /// Selected notes at the time of the snapshot.
    pub selected: yinhe_core::Selection,
    /// Selected arrangement tracks at the time of the snapshot.
    pub track_selected: HashSet<u16>,
    /// Selection rectangle at the time of the snapshot.
    pub sel_rect: SelRectState,
}

/// A fully compressed snapshot stored in the stack.
struct CompressedSnapshot {
    compressed: Vec<u8>,
    label: &'static str,
    selected: yinhe_core::Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
}

/// Per-document undo/redo stack.
///
/// `past` holds compressed states *before* each completed edit, oldest at front.
/// `pending` holds uncompressed states waiting to be compressed (newest at back).
/// `future` holds compressed states that have been undone, ready to be redone.
pub struct UndoStack {
    past: VecDeque<CompressedSnapshot>,
    pending: VecDeque<UndoSnapshot>,
    future: Vec<CompressedSnapshot>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: VecDeque::new(),
            pending: VecDeque::new(),
            future: Vec::new(),
        }
    }

    /// Phase 1: Record a snapshot of the state *before* an edit.
    /// O(1) — just bumps the Arc refcount. The snapshot will be compressed
    /// on a later frame by `compress_one()`.
    pub fn push(&mut self, snapshot: UndoSnapshot) {
        if self.past.len() + self.pending.len() >= MAX_DEPTH {
            // Drop the oldest compressed snapshot to make room.
            if self.past.len() > 0 {
                self.past.pop_front();
            } else {
                // All slots are pending — drop the oldest pending.
                self.pending.pop_front();
            }
        }
        self.pending.push_back(snapshot);
        self.future.clear();
    }

    /// Phase 2: Compress one pending snapshot (if any).
    /// Call once per frame from the UI loop. O(N) but amortized.
    /// Returns true if a snapshot was compressed.
    pub fn compress_one(&mut self) -> bool {
        let Some(pending) = self.pending.pop_front() else {
            return false;
        };
        let compressed = pending.data.compress_snapshot();
        self.past.push_back(CompressedSnapshot {
            compressed,
            label: pending.label,
            selected: pending.selected,
            track_selected: pending.track_selected,
            sel_rect: pending.sel_rect,
        });
        // `pending.data` is dropped here → Arc refcount decreases → next
        // `make_mut` on the current model will not deep-copy.
        true
    }

    /// Pop the most recent past snapshot, pushing `current` onto the redo stack.
    /// Returns `None` when there is nothing to undo.
    pub fn undo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        // Compress any remaining pending snapshots first so undo order is correct.
        while self.compress_one() {}
        let prev = self.past.pop_back()?;
        let data = ProjectData::decompress_snapshot(&prev.compressed);
        let current_compressed = current.data.compress_snapshot();
        self.future.push(CompressedSnapshot {
            compressed: current_compressed,
            label: current.label,
            selected: current.selected,
            track_selected: current.track_selected,
            sel_rect: current.sel_rect,
        });
        Some(UndoSnapshot {
            data,
            label: prev.label,
            selected: prev.selected,
            track_selected: prev.track_selected,
            sel_rect: prev.sel_rect,
        })
    }

    /// Pop the most recent future snapshot, pushing `current` onto the undo stack.
    /// Returns `None` when there is nothing to redo.
    pub fn redo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        // Compress any remaining pending snapshots first so redo order is correct.
        while self.compress_one() {}
        let next = self.future.pop()?;
        let data = ProjectData::decompress_snapshot(&next.compressed);
        let current_compressed = current.data.compress_snapshot();
        self.past.push_back(CompressedSnapshot {
            compressed: current_compressed,
            label: current.label,
            selected: current.selected,
            track_selected: current.track_selected,
            sel_rect: current.sel_rect,
        });
        Some(UndoSnapshot {
            data,
            label: next.label,
            selected: next.selected,
            track_selected: next.track_selected,
            sel_rect: next.sel_rect,
        })
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty() || !self.pending.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.pending.clear();
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
        selected: yinhe_core::Selection::default(),
        track_selected: HashSet::new(),
        sel_rect: SelRectState::default(),
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
    use crate::project_data::ProjectData;
    use std::sync::Arc;
    use yinhe_core::{ConductorData, TempoEvent, TimeSigEvent, TrackData, YinModel};

    fn make_test_data(name: &str) -> ProjectData {
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
        ProjectData::new(
            Arc::new(model),
            vec![name.to_string()],
            Default::default(),
            Default::default(),
        )
    }

    fn snap(label: &'static str, name: &str) -> UndoSnapshot {
        UndoSnapshot {
            data: make_test_data(name),
            label,
            selected: yinhe_core::Selection::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
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
        // compress_one to move pending to past
        stack.compress_one();
        let current = snap("cur", "current");
        let prev = stack.undo(current);
        assert!(prev.is_some());
        assert_eq!(prev.unwrap().data.model.tracks[0].name, "old");
        assert!(stack.can_redo());
    }

    #[test]
    fn redo_returns_future_and_pushes_to_past() {
        let mut stack = UndoStack::new();
        stack.push(snap("init", "a"));
        stack.compress_one();
        // undo pushes current ("b") to future, returns previous ("a")
        stack.undo(snap("cur", "b"));
        // redo pops future ("b"), pushes current ("a") to past, returns "b"
        let current = snap("after_undo", "a");
        let next = stack.redo(current);
        assert!(next.is_some());
        assert_eq!(next.unwrap().data.model.tracks[0].name, "b");
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

        stack.compress_one();
        stack.undo(snap("cur", "b"));
        assert!(!stack.can_undo());
        assert!(stack.can_redo());

        stack.redo(snap("cur2", "b"));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn compress_one_releases_data() {
        let mut stack = UndoStack::new();
        stack.push(snap("test", "data"));
        assert_eq!(stack.pending.len(), 1);
        assert_eq!(stack.past.len(), 0);

        stack.compress_one();
        assert_eq!(stack.pending.len(), 0);
        assert_eq!(stack.past.len(), 1);
    }

    #[test]
    fn compress_one_on_empty_does_nothing() {
        let mut stack = UndoStack::new();
        assert!(!stack.compress_one());
    }

    #[test]
    fn clear_wipes_everything() {
        let mut stack = UndoStack::new();
        stack.push(snap("1", "a"));
        stack.compress_one();
        stack.undo(snap("2", "b"));
        assert!(stack.can_undo() || stack.can_redo());

        stack.clear();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
        assert_eq!(stack.past.len(), 0);
        assert_eq!(stack.pending.len(), 0);
        assert_eq!(stack.future.len(), 0);
    }

    #[test]
    fn compress_and_decompress_roundtrip() {
        let data = make_test_data("roundtrip_test");
        let compressed = data.compress_snapshot();
        let restored = ProjectData::decompress_snapshot(&compressed);
        assert_eq!(restored.model.tracks.len(), 1);
        assert_eq!(restored.model.tracks[0].name, "roundtrip_test");
    }
}
