//! Undo/redo history for a single document.
//!
//! Snapshots are compressed on a **background thread** so the UI never stalls.
//!
//! - `push()`: spawns a thread to compress the snapshot, returns instantly.
//! - `poll_compression()`: call once per frame — moves finished compressions
//!   into the `past` / `future` stack.
//! - `undo()` / `redo()`: drain all pending compressions first (blocks on
//!   join), then decompress the target snapshot synchronously (~50 ms).

use std::collections::{HashMap, HashSet};

use crate::edit_state::SelRectState;
use crate::project_data::ProjectData;

/// Maximum number of past edits kept in the undo stack.
pub const MAX_DEPTH: usize = 100;

/// One undo/redo snapshot. Returned by `snapshot_with_selection()` and
/// consumed by `UndoStack::push()` / returned by `undo()` / `redo()`.
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

/// A snapshot currently being compressed on a background thread.
struct CompressingSnapshot {
    handle: std::thread::JoinHandle<Vec<u8>>,
    label: &'static str,
    selected: yinhe_core::Selection,
    track_selected: HashSet<u16>,
    sel_rect: SelRectState,
}

/// Per-document undo/redo stack.
///
/// `past` / `future` hold fully compressed snapshots.
/// `past_compressing` / `future_compressing` hold snapshots still being
/// compressed on background threads.
pub struct UndoStack {
    past: Vec<CompressedSnapshot>,
    past_compressing: Vec<CompressingSnapshot>,
    future: Vec<CompressedSnapshot>,
    future_compressing: Vec<CompressingSnapshot>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            past_compressing: Vec::new(),
            future: Vec::new(),
            future_compressing: Vec::new(),
        }
    }

    /// Record a snapshot of the state *before* an edit.
    /// Spawns a background thread to compress it. O(1) — returns instantly.
    pub fn push(&mut self, snapshot: UndoSnapshot) {
        if self.past.len() >= MAX_DEPTH {
            self.past.remove(0);
        }
        let data = snapshot.data; // move, not clone
        let handle = std::thread::spawn(move || data.compress_snapshot());
        self.past_compressing.push(CompressingSnapshot {
            handle,
            label: snapshot.label,
            selected: snapshot.selected,
            track_selected: snapshot.track_selected,
            sel_rect: snapshot.sel_rect,
        });
        self.future.clear();
        self.future_compressing.clear();
    }

    /// Call once per frame. Moves finished background compressions into
    /// the `past` / `future` stacks.
    pub fn poll_compression(&mut self) {
        Self::drain_finished(&mut self.past, &mut self.past_compressing);
        Self::drain_finished(&mut self.future, &mut self.future_compressing);
    }

    fn drain_finished(
        dest: &mut Vec<CompressedSnapshot>,
        src: &mut Vec<CompressingSnapshot>,
    ) {
        let mut i = 0;
        while i < src.len() {
            if src[i].handle.is_finished() {
                let c = src.swap_remove(i);
                let compressed = c.handle.join().unwrap();
                dest.push(CompressedSnapshot {
                    compressed,
                    label: c.label,
                    selected: c.selected,
                    track_selected: c.track_selected,
                    sel_rect: c.sel_rect,
                });
            } else {
                i += 1;
            }
        }
    }



    /// Pop the most recent past snapshot, pushing `current` onto the redo stack.
    /// If the snapshot is still being compressed on a background thread, waits
    /// only for that one thread (not all pending compressions).
    /// Returns `None` when there is nothing to undo.
    pub fn undo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        // Try already-compressed first.
        let prev = if let Some(prev) = self.past.pop() {
            prev
        } else {
            // Still compressing — wait only for the most recent one.
            let c = self.past_compressing.pop()?;
            let compressed = c.handle.join().unwrap();
            CompressedSnapshot {
                compressed,
                label: c.label,
                selected: c.selected,
                track_selected: c.track_selected,
                sel_rect: c.sel_rect,
            }
        };
        let data = ProjectData::decompress_snapshot(&prev.compressed);
        // Compress current state in background.
        let handle = std::thread::spawn(move || current.data.compress_snapshot());
        self.future_compressing.push(CompressingSnapshot {
            handle,
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
    /// If the snapshot is still being compressed on a background thread, waits
    /// only for that one thread (not all pending compressions).
    /// Returns `None` when there is nothing to redo.
    pub fn redo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        // Try already-compressed first.
        let next = if let Some(next) = self.future.pop() {
            next
        } else {
            // Still compressing — wait only for the most recent one.
            let c = self.future_compressing.pop()?;
            let compressed = c.handle.join().unwrap();
            CompressedSnapshot {
                compressed,
                label: c.label,
                selected: c.selected,
                track_selected: c.track_selected,
                sel_rect: c.sel_rect,
            }
        };
        let data = ProjectData::decompress_snapshot(&next.compressed);
        // Compress current state in background.
        let handle = std::thread::spawn(move || current.data.compress_snapshot());
        self.past_compressing.push(CompressingSnapshot {
            handle,
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
        !self.past.is_empty() || !self.past_compressing.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty() || !self.future_compressing.is_empty()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.past_compressing.clear();
        self.future.clear();
        self.future_compressing.clear();
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks per-widget "before-edit" snapshots for TextEdit-like fields.
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
        stack.poll_compression(); // wait for background compression
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
        stack.poll_compression();
        // undo pushes current ("b") to future, returns previous ("a")
        stack.undo(snap("cur", "b"));
        stack.poll_compression();
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

        stack.poll_compression();
        stack.undo(snap("cur", "b"));
        assert!(!stack.can_undo());
        assert!(stack.can_redo());

        stack.poll_compression();
        stack.redo(snap("cur2", "b"));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn clear_wipes_everything() {
        let mut stack = UndoStack::new();
        stack.push(snap("1", "a"));
        stack.poll_compression();
        stack.undo(snap("2", "b"));
        assert!(stack.can_undo() || stack.can_redo());

        stack.clear();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
        assert_eq!(stack.past.len(), 0);
        assert_eq!(stack.past_compressing.len(), 0);
        assert_eq!(stack.future.len(), 0);
        assert_eq!(stack.future_compressing.len(), 0);
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
