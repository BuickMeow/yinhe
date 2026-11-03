//! Document-level MIDI import plumbing: merge + selection + undo entry.
//!
//! The model merge itself lives in [`crate::midi_import`] so it can also run on
//! a cloned model in a background thread (desktop) or on a document moved to a
//! worker thread (mobile doc job).

use yinhe_core::YinModel;
use yinhe_types::MAX_KEY;

use crate::history::{EditSnapshot, UndoAction};
use crate::midi_import::{MidiImportMerged, MidiImportOptions, merge_midi_into_model};

use super::Document;

/// Small, copyable result for UI toasts / viewport jumps.
#[derive(Clone, Copy, Debug)]
pub struct MidiImportOutcome {
    pub first_track: usize,
    pub last_track: usize,
    pub start_tick: u32,
    pub end_tick: u32,
    pub note_count: u64,
}

impl Document {
    /// Import `src` at the end of this document.
    ///
    /// Called from a worker thread in practice; `progress` receives 0..1.
    pub fn import_midi_with_progress(
        &mut self,
        src: YinModel,
        opts: &MidiImportOptions,
        progress: impl FnMut(f32),
    ) -> Option<MidiImportOutcome> {
        let before = self.capture_snapshot();
        let model = std::sync::Arc::make_mut(&mut self.data.model);
        let merged = merge_midi_into_model(model, src, opts, progress)?;
        let outcome = self.apply_import_result(&merged);
        let action = UndoAction::ImportMidi {
            tracks_before: merged.tracks_before,
            conductor_before: merged.conductor_before,
            note_id_start: merged.note_id_start,
            note_id_end: merged.note_id_end,
            note_keys: merged.note_keys.clone(),
            undone: false,
        };
        self.push_undo(action, "Import MIDI", before);
        Some(outcome)
    }

    /// Install an already-merged model (desktop background import) and record
    /// the undo entry. `before` must be captured **before** the merge started.
    pub fn install_imported_model(
        &mut self,
        model: YinModel,
        merged: MidiImportMerged,
        before: EditSnapshot,
    ) -> MidiImportOutcome {
        self.data.model = std::sync::Arc::new(model);
        let outcome = self.apply_import_result(&merged);
        let action = UndoAction::ImportMidi {
            tracks_before: merged.tracks_before,
            conductor_before: merged.conductor_before,
            note_id_start: merged.note_id_start,
            note_id_end: merged.note_id_end,
            note_keys: merged.note_keys.clone(),
            undone: false,
        };
        self.push_undo(action, "Import MIDI", before);
        outcome
    }

    fn apply_import_result(&mut self, merged: &MidiImportMerged) -> MidiImportOutcome {
        self.sync_track_caches();
        self.data.bump_revision();

        let start = merged.start_tick;
        let end = merged.end_tick.max(start.saturating_add(1));
        self.edit.selected.clear();
        self.edit.sel_rect.clear();
        self.edit.arr_sel_rect.clear();
        self.edit.chan_ar_sel.clear();
        self.edit.track_selected.clear();
        for t in merged.first_track..=merged.last_track {
            self.edit.track_selected.insert(t as u16);
        }
        self.edit.selected.add_rect_track(
            start,
            end,
            0,
            MAX_KEY,
            merged.first_track as u16,
            merged.last_track as u16,
        );
        self.edit.arr_sel_rect.push((
            start as f64,
            end as f64,
            merged.first_track,
            merged.last_track,
        ));
        self.edit.cursor_tick = Some(start as f64);

        MidiImportOutcome {
            first_track: merged.first_track,
            last_track: merged.last_track,
            start_tick: start,
            end_tick: end,
            note_count: merged.note_count,
        }
    }
}
