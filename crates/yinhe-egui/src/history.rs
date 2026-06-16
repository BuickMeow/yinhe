//! Undo/redo history for a single document.
//!
//! Stores snapshots of the persistent project data (MIDI + track names).
//! Cloning a snapshot is cheap when the underlying `MidiFile` was not
//! mutated (Arc shared); a real copy only happens for snapshots taken
//! before an edit that calls `Arc::make_mut`.
//!
//! Selection, scroll, mute/solo and other UI state are intentionally NOT
//! captured — undo restores notes and names, not the user's cursor.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use eframe::egui;
use yinhe_midi::MidiFile;

use crate::document::Document;

/// Maximum number of past edits kept in the undo stack.
pub const MAX_DEPTH: usize = 100;

/// One persistent-state snapshot. Cheap to clone (Arc + Vec<String> + small fields).
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub midi: Arc<MidiFile>,
    pub track_names: Vec<String>,
    pub project_name: String,
    pub project_artist: String,
    pub project_description: String,
    pub project_ppq: u32,
    /// `Some` when the document has an associated archive (zstd compression
    /// level is the only undo-relevant field on it).
    pub archive_compression_level: Option<i32>,
    /// Short label used for debugging / future UI ("Delete notes", "Move notes", …).
    pub label: &'static str,
}

impl Snapshot {
    /// Build a snapshot from the document's current persistent state.
    pub fn capture(doc: &Document, label: &'static str) -> Self {
        Self {
            midi: Arc::clone(&doc.midi),
            track_names: doc.track_names.clone(),
            project_name: doc.project_name.clone(),
            project_artist: doc.project_artist.clone(),
            project_description: doc.project_description.clone(),
            project_ppq: doc.project_ppq,
            archive_compression_level: doc.archive.as_ref().map(|a| a.compression_level),
            label,
        }
    }

    /// Restore this snapshot's fields onto the document.
    pub fn restore(self, doc: &mut Document) {
        doc.midi = self.midi;
        doc.track_names = self.track_names;
        doc.project_name = self.project_name;
        doc.project_artist = self.project_artist;
        doc.project_description = self.project_description;
        doc.project_ppq = self.project_ppq;
        if let (Some(archive), Some(level)) = (doc.archive.as_mut(), self.archive_compression_level)
        {
            archive.compression_level = level;
        }
    }
}

/// Per-document undo/redo stack.
///
/// `past` holds states *before* each completed edit, oldest at the front.
/// `future` holds states that have been undone, ready to be redone.
pub(crate) struct History {
    past: VecDeque<Snapshot>,
    future: Vec<Snapshot>,
}

impl History {
    pub fn new() -> Self {
        Self {
            past: VecDeque::new(),
            future: Vec::new(),
        }
    }

    /// Record a snapshot of the state *before* an edit. Clears the redo stack.
    pub fn push(&mut self, snapshot: Snapshot) {
        if self.past.len() >= MAX_DEPTH {
            self.past.pop_front();
        }
        self.past.push_back(snapshot);
        self.future.clear();
    }

    /// Pop the most recent past snapshot, pushing `current` onto the redo stack.
    /// Returns `None` when there is nothing to undo.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.past.pop_back()?;
        self.future.push(current);
        Some(prev)
    }

    /// Pop the most recent future snapshot, pushing `current` onto the undo stack.
    /// Returns `None` when there is nothing to redo.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
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

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks per-widget "before-edit" snapshots for TextEdit-like fields.
///
/// Pattern:
/// 1. On `resp.gained_focus()` call `begin(id, doc, label)` — stashes the
///    snapshot of the document at the moment focus was gained.
/// 2. On `resp.lost_focus()` (or Enter) call `commit(id, doc)` — if the
///    document state actually changed since `begin`, the stashed snapshot is
///    pushed onto `doc.history`. Otherwise it is discarded.
#[derive(Default)]
pub(crate) struct PendingEdits {
    map: HashMap<egui::Id, Snapshot>,
}

impl PendingEdits {
    pub fn has(&self, id: egui::Id) -> bool {
        self.map.contains_key(&id)
    }

    pub fn insert_raw(&mut self, id: egui::Id, snapshot: Snapshot) {
        self.map.insert(id, snapshot);
    }

    pub fn take(&mut self, id: egui::Id) -> Option<Snapshot> {
        self.map.remove(&id)
    }
}

/// Begin tracking a TextEdit/DragValue on `doc` keyed by `id`.
/// Captures a baseline snapshot under that id (overwriting any previous one).
pub(crate) fn begin_edit(doc: &mut Document, id: egui::Id, label: &'static str) {
    let snap = Snapshot::capture(doc, label);
    doc.pending_edits.insert_raw(id, snap);
}

/// Commit a TextEdit/DragValue on `doc` keyed by `id`.
/// If a baseline exists and the document state diverged from it, the baseline
/// is pushed onto `doc.history`. Otherwise the baseline is silently discarded.
pub(crate) fn commit_edit(doc: &mut Document, id: egui::Id) {
    let Some(baseline) = doc.pending_edits.take(id) else {
        return;
    };
    let changed = baseline.project_name != doc.project_name
        || baseline.project_artist != doc.project_artist
        || baseline.project_description != doc.project_description
        || baseline.project_ppq != doc.project_ppq
        || baseline.archive_compression_level
            != doc.archive.as_ref().map(|a| a.compression_level)
        || baseline.track_names != doc.track_names;
    if changed {
        doc.history.push(baseline);
    }
}
