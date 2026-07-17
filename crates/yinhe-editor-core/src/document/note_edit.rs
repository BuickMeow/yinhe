//! Note editing operations: add, delete, duplicate, transpose, pencil drag, move.

use std::sync::Arc;

use yinhe_core::NoteEvent;
use yinhe_types::PencilNoteDrag;

use crate::batch_ops;
use crate::history::{NoteDelta, UndoAction};

use super::Document;

impl Document {
    /// Add a single note. Returns an `UndoAction` if the note was added.
    pub fn add_note(&mut self, track_idx: u16, note: NoteEvent) -> Option<UndoAction> {
        let t = track_idx as usize;
        if t >= self.data.model.tracks.len() {
            return None;
        }
        if Some(track_idx) == self.edit.conductor_track_idx {
            return None;
        }
        let key = note.key;
        let typed_note = yinhe_types::Note {
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            velocity: note.velocity,
            dup_index: note.dup_index,
            track: track_idx,
        };
        {
            let model = Arc::make_mut(&mut self.data.model);
            let k = key as usize;
            let insert_pos = model.notes[k].partition_point(|n| n.start_tick < note.start_tick);
            Arc::make_mut(&mut model.notes[k]).insert(insert_pos, typed_note);
            model.mark_dirty(key);
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after: vec![(typed_note, key)],
        }))
    }

    /// Delete all selected notes. Returns an `UndoAction` if any notes were deleted.
    pub fn delete_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        // Collect before any mutation.
        let matched = batch_ops::collect_selected(&self.data.model, &self.edit.selected);
        if matched.is_empty() {
            self.edit.selected.clear();
            return None;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);
            batch_ops::remove_selected(model, &self.edit.selected);
            self.edit.selected.clear();
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: matched,
            after: vec![],
        }))
    }

    /// Duplicate all selected notes. Returns an `UndoAction` if any notes were duplicated.
    pub fn duplicate_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let after = {
            let model = Arc::make_mut(&mut self.data.model);

            let selected_data = batch_ops::collect_selected(model, &self.edit.selected);
            if selected_data.is_empty() {
                return None;
            }

            let min_start = selected_data.iter().map(|(n, _)| n.start_tick).min().unwrap();
            let max_end = selected_data.iter().map(|(n, _)| n.end_tick).max().unwrap();
            let offset = (max_end - min_start).max(1);

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, key) in &selected_data {
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick + offset,
                    end_tick: note.end_tick + offset,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: note.track,
                };
                new_by_key.entry(*key).or_default().push(new_note);
            }

            // Build after vec before moving new_by_key.
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

            batch_ops::insert_batch(model, new_by_key);

            // Offset selection rects to cover the duplicated notes.
            self.edit.selected.offset(offset as i64, 0);
            after
        };
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// Transpose all selected notes by `semitones`. Returns an `UndoAction` if any notes were transposed.
    pub fn transpose_selected(&mut self, semitones: i8) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let (before, after) = {
            let model = Arc::make_mut(&mut self.data.model);

            let moved_data = batch_ops::remove_selected(model, &self.edit.selected);
            if moved_data.is_empty() {
                return None;
            }

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, old_key) in &moved_data {
                let new_key = ((*old_key as i16) + (semitones as i16)).clamp(0, 127) as u8;
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: note.track,
                };
                new_by_key.entry(new_key).or_default().push(new_note);
            }

            // Build after vec before moving new_by_key.
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

            batch_ops::insert_batch(model, new_by_key);

            // Offset selection rects to follow the transposed notes.
            self.edit.selected.offset(0, semitones as i32);
            (moved_data, after)
        };
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }

    /// Move all selected notes by (delta_ticks, delta_keys).
    ///
    /// Returns an `UndoAction` if any notes were moved. The caller is
    /// responsible for pushing it to the history stack, marking the view
    /// dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn move_selected_notes(&mut self, delta_ticks: i64, delta_keys: i32) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        if delta_ticks == 0 && delta_keys == 0 {
            return None;
        }

        let model = Arc::make_mut(&mut self.data.model);

        // Batch removal + collect removed notes.
        let originals = batch_ops::remove_selected(model, &self.edit.selected);

        // Batch insert: group by destination key, extend.
        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> = std::collections::HashMap::new();
        for (note, old_key) in &originals {
            let new_key = ((*old_key as i32) + delta_keys).clamp(0, 127) as u8;
            let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
            let length = note.end_tick - note.start_tick;
            let moved = yinhe_types::Note {
                start_tick: new_tick,
                end_tick: new_tick + length,
                velocity: note.velocity,
                dup_index: 0,
                track: note.track,
            };
            new_by_key.entry(new_key).or_default().push(moved);
        }
        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
        batch_ops::insert_batch(model, new_by_key);

        // Offset selection rects to follow the moved notes.
        self.edit.selected.offset(delta_ticks, delta_keys);
        model.rebuild_dirty();
        self.data.bump_revision();

        Some(UndoAction::Notes(NoteDelta {
            before: originals,
            after,
        }))
    }

    /// Apply a pencil-tool drag operation (move or resize a single note).
    ///
    /// Returns an `UndoAction` if the note was modified. The caller is
    /// responsible for pushing it to the history stack, marking the view
    /// dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn pencil_drag_note(&mut self, drag: &PencilNoteDrag) -> Option<UndoAction> {
        match drag {
            PencilNoteDrag::Move { track, start_tick, key, delta_ticks, delta_keys } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k].iter().find(|n| {
                    n.track == *track && n.start_tick == *start_tick
                })?;
                let orig_note = *note;
                let new_key = ((*key as i32) + delta_keys).clamp(0, 127) as u8;
                let new_tick = (orig_note.start_tick as i64 + delta_ticks).max(0) as u32;

                if *delta_ticks != 0 || *delta_keys != 0 {
                    let model = Arc::make_mut(&mut self.data.model);
                    // Remove original from old key bucket
                    let ok = *key as usize;
                    Arc::make_mut(&mut model.notes[ok]).retain(|n| {
                        !(n.track == *track && n.start_tick == orig_note.start_tick && n.dup_index == orig_note.dup_index)
                    });
                    model.mark_dirty(*key);
                    // Insert moved note at new key bucket
                    let length = orig_note.end_tick - orig_note.start_tick;
                    let moved = yinhe_types::Note {
                        start_tick: new_tick,
                        end_tick: new_tick + length,
                        velocity: orig_note.velocity,
                        dup_index: 0,
                        track: *track,
                    };
                    let nk = new_key as usize;
                    let insert_pos = model.notes[nk].partition_point(|n| n.start_tick < moved.start_tick);
                    Arc::make_mut(&mut model.notes[nk]).insert(insert_pos, moved);
                    model.mark_dirty(new_key);
                    model.rebuild_dirty();
                    self.data.bump_revision();
                    return Some(UndoAction::Notes(NoteDelta {
                        before: vec![(orig_note, *key)],
                        after: vec![(moved, new_key)],
                    }));
                }
                None
            }
            PencilNoteDrag::ResizeRight { track, start_tick, key, new_end_tick } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k].iter().find(|n| {
                    n.track == *track && n.start_tick == *start_tick
                })?;
                if *new_end_tick != note.end_tick {
                    let before = *note;
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k]).iter_mut().find(|n| {
                        n.track == *track && n.start_tick == *start_tick
                    }) {
                        n.end_tick = (*new_end_tick).max(n.start_tick + 1);
                        let after = *n;
                        model.mark_dirty(*key);
                        model.rebuild_dirty();
                        self.data.bump_revision();
                        return Some(UndoAction::Notes(NoteDelta {
                            before: vec![(before, *key)],
                            after: vec![(after, *key)],
                        }));
                    }
                }
                None
            }
            PencilNoteDrag::ResizeLeft { track, start_tick, key, new_start_tick } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k].iter().find(|n| {
                    n.track == *track && n.start_tick == *start_tick
                })?;
                if *new_start_tick != note.start_tick {
                    let before = *note;
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k]).iter_mut().find(|n| {
                        n.track == *track && n.start_tick == *start_tick
                    }) {
                        n.start_tick = (*new_start_tick).min(n.end_tick - 1);
                        let after = *n;
                        model.mark_dirty(*key);
                        model.rebuild_dirty();
                        self.data.bump_revision();
                        return Some(UndoAction::Notes(NoteDelta {
                            before: vec![(before, *key)],
                            after: vec![(after, *key)],
                        }));
                    }
                }
                None
            }
        }
    }
}
