//! Undo/redo history for a single document.
//!
//! When `compression_enabled` is false (default), snapshots are stored
//! uncompressed (`ProjectData` via Arc bump) — fast but memory-heavy.
//!
//! When `compression_enabled` is true, snapshots are compressed on a
//! background thread — memory-efficient but adds a small latency on undo/redo
//! (decompress ~50 ms).

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

/// Internal storage: either compressed bytes or the full ProjectData.
enum StoredSnapshot {
    /// Zstd-compressed bincode blob.
    Compressed(Vec<u8>),
    /// Uncompressed project data (Arc bump only).
    Uncompressed(ProjectData),
}

struct StoredSnapshotMeta {
    inner: StoredSnapshot,
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
/// When `compression_enabled` is false (default), snapshots are stored
/// as `StoredSnapshot::Uncompressed` — no compression overhead.
/// When true, they are compressed on background threads.
pub struct UndoStack {
    past: Vec<StoredSnapshotMeta>,
    past_compressing: Vec<CompressingSnapshot>,
    future: Vec<StoredSnapshotMeta>,
    future_compressing: Vec<CompressingSnapshot>,
    /// When true, snapshots are compressed on a background thread.
    /// When false (default), snapshots are stored uncompressed.
    pub compression_enabled: bool,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            past_compressing: Vec::new(),
            future: Vec::new(),
            future_compressing: Vec::new(),
            compression_enabled: false,
        }
    }

    /// Record a snapshot of the state *before* an edit.
    ///
    /// If `compression_enabled` is true, spawns a background thread to
    /// compress it (O(1) — returns instantly).
    /// Otherwise, stores the snapshot uncompressed (O(1) Arc bump).
    pub fn push(&mut self, snapshot: UndoSnapshot) {
        if self.past.len() >= MAX_DEPTH {
            self.past.remove(0);
        }
        if self.compression_enabled {
            let data = snapshot.data;
            let handle = std::thread::spawn(move || data.compress_snapshot());
            self.past_compressing.push(CompressingSnapshot {
                handle,
                label: snapshot.label,
                selected: snapshot.selected,
                track_selected: snapshot.track_selected,
                sel_rect: snapshot.sel_rect,
            });
        } else {
            self.past.push(StoredSnapshotMeta {
                inner: StoredSnapshot::Uncompressed(snapshot.data),
                label: snapshot.label,
                selected: snapshot.selected,
                track_selected: snapshot.track_selected,
                sel_rect: snapshot.sel_rect,
            });
        }
        self.future.clear();
        self.future_compressing.clear();
    }

    /// Enable or disable compression.
    /// Existing snapshots are not retroactively converted.
    pub fn set_compression_enabled(&mut self, enabled: bool) {
        self.compression_enabled = enabled;
    }

    /// Call once per frame. Moves finished background compressions into
    /// the `past` / `future` stacks.
    pub fn poll_compression(&mut self) {
        Self::drain_finished(&mut self.past, &mut self.past_compressing);
        Self::drain_finished(&mut self.future, &mut self.future_compressing);
    }

    fn drain_finished(
        dest: &mut Vec<StoredSnapshotMeta>,
        src: &mut Vec<CompressingSnapshot>,
    ) {
        let mut i = 0;
        while i < src.len() {
            if src[i].handle.is_finished() {
                let c = src.swap_remove(i);
                let compressed = c.handle.join().unwrap();
                dest.push(StoredSnapshotMeta {
                    inner: StoredSnapshot::Compressed(compressed),
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
    /// Returns `None` when there is nothing to undo.
    pub fn undo(&mut self, current: UndoSnapshot) -> Option<UndoSnapshot> {
        let prev = if let Some(prev) = self.past.pop() {
            prev
        } else {
            let c = self.past_compressing.pop()?;
            let compressed = c.handle.join().unwrap();
            StoredSnapshotMeta {
                inner: StoredSnapshot::Compressed(compressed),
                label: c.label,
                selected: c.selected,
                track_selected: c.track_selected,
                sel_rect: c.sel_rect,
            }
        };
        let data = match prev.inner {
            StoredSnapshot::Compressed(bytes) => ProjectData::decompress_snapshot(&bytes),
            StoredSnapshot::Uncompressed(data) => data,
        };
        // Store current state.
        if self.compression_enabled {
            let handle = std::thread::spawn(move || current.data.compress_snapshot());
            self.future_compressing.push(CompressingSnapshot {
                handle,
                label: current.label,
                selected: current.selected,
                track_selected: current.track_selected,
                sel_rect: current.sel_rect,
            });
        } else {
            self.future.push(StoredSnapshotMeta {
                inner: StoredSnapshot::Uncompressed(current.data),
                label: current.label,
                selected: current.selected,
                track_selected: current.track_selected,
                sel_rect: current.sel_rect,
            });
        }
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
        let next = if let Some(next) = self.future.pop() {
            next
        } else {
            let c = self.future_compressing.pop()?;
            let compressed = c.handle.join().unwrap();
            StoredSnapshotMeta {
                inner: StoredSnapshot::Compressed(compressed),
                label: c.label,
                selected: c.selected,
                track_selected: c.track_selected,
                sel_rect: c.sel_rect,
            }
        };
        let data = match next.inner {
            StoredSnapshot::Compressed(bytes) => ProjectData::decompress_snapshot(&bytes),
            StoredSnapshot::Uncompressed(data) => data,
        };
        // Store current state.
        if self.compression_enabled {
            let handle = std::thread::spawn(move || current.data.compress_snapshot());
            self.past_compressing.push(CompressingSnapshot {
                handle,
                label: current.label,
                selected: current.selected,
                track_selected: current.track_selected,
                sel_rect: current.sel_rect,
            });
        } else {
            self.past.push(StoredSnapshotMeta {
                inner: StoredSnapshot::Uncompressed(current.data),
                label: current.label,
                selected: current.selected,
                track_selected: current.track_selected,
                sel_rect: current.sel_rect,
            });
        }
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
        stack.undo(snap("cur", "b"));
        assert!(stack.can_redo());
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
        assert_eq!(prev.unwrap().data.model.tracks[0].name, "old");
        assert!(stack.can_redo());
    }

    #[test]
    fn redo_returns_future_and_pushes_to_past() {
        let mut stack = UndoStack::new();
        stack.push(snap("init", "a"));
        stack.undo(snap("cur", "b"));
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

        stack.undo(snap("cur", "b"));
        assert!(!stack.can_undo());
        assert!(stack.can_redo());

        stack.redo(snap("cur2", "b"));
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
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
        assert_eq!(stack.past_compressing.len(), 0);
        assert_eq!(stack.future.len(), 0);
        assert_eq!(stack.future_compressing.len(), 0);
    }

    #[test]
    fn compression_enabled_background_thread() {
        let mut stack = UndoStack::new();
        stack.set_compression_enabled(true);
        stack.push(snap("init", "a"));
        // Should be in past_compressing, not past
        assert_eq!(stack.past.len(), 0);
        assert_eq!(stack.past_compressing.len(), 1);
        // Poll should move it to past (retry in case thread isn't done yet)
        for _ in 0..100 {
            stack.poll_compression();
            if stack.past.len() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(stack.past.len(), 1);
        assert_eq!(stack.past_compressing.len(), 0);
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
