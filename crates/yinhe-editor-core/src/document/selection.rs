//! Selection operations: select-all and paste.

use std::sync::Arc;

use crate::batch_ops;
use crate::history::{NoteDelta, UndoAction, UndoStack};

use super::Document;

impl Document {
    /// Select all notes in the currently selected track(s) for Piano Roll.
    /// Range: tick 0 → last note end (global), keys 0–127.
    ///
    /// Uses `model.tick_length` (O(1)) instead of scanning all key buckets (O(N)).
    /// Sets `sel_rect.rect` to the full global range so the visual selection box
    /// covers 0 → tick_length, keys 0–127.
    pub fn select_all_pr(&mut self) {
        let model = &self.data.model;
        let max_end = model.tick_length as u32;
        if max_end == 0 {
            return;
        }

        let conductor = self.edit.conductor_track_idx;
        let tracks: Vec<u16> = if self.edit.track_selected.is_empty() {
            // 没有预选 track 时，全选所有非 conductor track
            let num_tracks = model.tracks.len() as u16;
            (0..num_tracks).filter(|&t| Some(t) != conductor).collect()
        } else {
            self.edit.track_selected.iter().copied().collect()
        };
        if tracks.is_empty() {
            return;
        }

        self.edit.selected.clear();
        for &track_idx in &tracks {
            if Some(track_idx) == conductor {
                continue;
            }
            self.edit.selected.add_rect_track(0, max_end + 1, 0, 127, track_idx, track_idx);
        }

        // Update visual sel_rect to show full range (PR uses f64 ticks).
        self.edit.sel_rect.rect = Some((0.0, max_end as f64 + 1.0, 0, 127));
    }

    /// Select all notes across all tracks for Arrange.
    /// Range: tick 0 → global last note end, keys 0–127, all tracks except conductor.
    pub fn select_all_ar(&mut self) {
        let model = &self.data.model;
        let max_end = model.tick_length as u32;
        if max_end == 0 {
            return;
        }
        let conductor = self.edit.conductor_track_idx;
        let num_tracks = model.tracks.len() as u16;

        self.edit.selected.clear();
        // One rect per non-conductor track range is overkill; use a single
        // broad rect and rely on conductor guard in add_note / move_selected.
        // But to be precise, split into: tracks before conductor, tracks after.
        match conductor {
            Some(c) if c > 0 => {
                self.edit.selected.add_rect_track(0, max_end + 1, 0, 127, 0, c - 1);
            }
            _ => {}
        }
        let after = conductor.map(|c| c + 1).unwrap_or(0);
        if after < num_tracks {
            self.edit.selected.add_rect_track(0, max_end + 1, 0, 127, after, num_tracks - 1);
        }
    }

    /// Paste notes from clipboard (selection rects) at the cursor position.
    ///
    /// Clipboard stores only selection rects (not note data) for performance.
    /// Notes are queried from the model at paste time. If the notes have been
    /// deleted (e.g. after cut), falls back to the undo entry identified by
    /// `cut_past_len` which contains the deleted notes in its `before` field.
    pub fn paste_from_selection(
        &mut self,
        clipboard: &yinhe_core::Selection,
        cursor_tick: f64,
        cut_past_len: Option<usize>,
        track_selected: &std::collections::HashSet<u16>,
    ) -> Option<UndoAction> {
        if clipboard.is_empty() {
            return None;
        }

        // Try querying the model first (normal copy-paste).
        let model = &self.data.model;
        let mut notes = batch_ops::collect_selected(model, clipboard);

        // Undo bridge: if model query returned nothing (notes were cut/deleted),
        // fall back to the correct undo entry identified by cut_past_len.
        //
        // cut_past_len was captured as past.len() BEFORE the delete was pushed.
        // After push, the delete entry sits at index `cut_past_len` (push appends
        // at the end, so old length = new entry's index).
        if notes.is_empty() {
            let entry = cut_past_len
                .and_then(|len| self.history.past.get(len))
                .or_else(|| self.history.past.last());
            if let Some(entry) = entry {
                if let UndoAction::Notes(delta) = &entry.action {
                    if !delta.before.is_empty() {
                        notes = delta.before.iter()
                            .filter(|(n, key)| clipboard.contains(n.track, n.start_tick, *key))
                            .cloned()
                            .collect();
                    }
                }
            }
        }

        if notes.is_empty() {
            return None;
        }

        // Calculate offset: cursor - min start_tick.
        let min_start = notes.iter().map(|(n, _)| n.start_tick).min().unwrap_or(0);
        let offset = cursor_tick as i64 - min_start as i64;

        // Calculate track offset: first selected track - min source track.
        // If no track is selected, keep original track positions.
        let track_offset: i32 = if !track_selected.is_empty() {
            let src_min_track = notes.iter().map(|(n, _)| n.track).min().unwrap_or(0);
            let first_selected = track_selected.iter().min().copied().unwrap_or(0);
            first_selected as i32 - src_min_track as i32
        } else {
            0
        };

        let conductor = self.edit.conductor_track_idx;
        let model = Arc::make_mut(&mut self.data.model);

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        for (note, key) in &notes {
            if Some(note.track) == conductor {
                continue;
            }
            let new_note = yinhe_types::Note {
                start_tick: (note.start_tick as i64 + offset).max(0) as u32,
                end_tick: (note.end_tick as i64 + offset).max(0) as u32,
                velocity: note.velocity,
                dup_index: 0,
                track: ((note.track as i32 + track_offset).clamp(0, u16::MAX as i32) as u16),
            };
            new_by_key.entry(*key).or_default().push(new_note);
        }

        if new_by_key.is_empty() {
            return None;
        }

        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();

        batch_ops::insert_batch(model, new_by_key);

        // Update selection to cover pasted notes.
        self.edit.selected.clear();
        let max_end = after.iter().map(|(n, _)| n.end_tick).max().unwrap_or(0);
        let min_tick = after.iter().map(|(n, _)| n.start_tick).min().unwrap_or(0);
        let mut track_lo = u16::MAX;
        let mut track_hi = 0u16;
        for (n, _) in &after {
            track_lo = track_lo.min(n.track);
            track_hi = track_hi.max(n.track);
        }
        self.edit.selected.add_rect_track(min_tick, max_end + 1, 0, 127, track_lo, track_hi);

        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }
}
