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
        let typed_note = {
            let model = Arc::make_mut(&mut self.data.model);
            let id = model.alloc_note_id();
            yinhe_types::Note {
                id,
                start_tick: note.start_tick,
                end_tick: note.end_tick,
                velocity: note.velocity,
                track: track_idx,
            }
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
                    id: model.alloc_note_id(),
                    start_tick: note.start_tick + offset,
                    end_tick: note.end_tick + offset,
                    velocity: note.velocity,
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

    /// Duplicate selected notes and offset the copies by `(delta_ticks, delta_keys)`.
    ///
    /// 原音符保留不动，副本平移到目标位置；选区同步移到副本范围，便于连续操作。
    /// 用于 Alt+拖动复制：一步操作，一个 undo entry。
    pub fn duplicate_selected_to(&mut self, delta_ticks: i64, delta_keys: i32) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let after = {
            let model = Arc::make_mut(&mut self.data.model);

            let selected_data = batch_ops::collect_selected(model, &self.edit.selected);
            if selected_data.is_empty() {
                return None;
            }

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, old_key) in &selected_data {
                let new_key = ((*old_key as i32) + delta_keys).clamp(0, 127) as u8;
                let new_start = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                let length = note.end_tick - note.start_tick;
                let new_note = yinhe_types::Note {
                    id: model.alloc_note_id(),
                    start_tick: new_start,
                    end_tick: new_start + length,
                    velocity: note.velocity,
                    track: note.track,
                };
                new_by_key.entry(new_key).or_default().push(new_note);
            }

            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

            batch_ops::insert_batch(model, new_by_key);

            // 选区跟随副本，便于连续 Alt+拖动
            self.edit.selected.offset(delta_ticks, delta_keys);
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
                    id: note.id,
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
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
                id: note.id,
                start_tick: new_tick,
                end_tick: new_tick + length,
                velocity: note.velocity,
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
                    // Remove original from old key bucket by id
                    let ok = *key as usize;
                    Arc::make_mut(&mut model.notes[ok]).retain(|n| n.id != orig_note.id);
                    model.mark_dirty(*key);
                    // Insert moved note at new key bucket（保留原 id）
                    let length = orig_note.end_tick - orig_note.start_tick;
                    let moved = yinhe_types::Note {
                        id: orig_note.id,
                        start_tick: new_tick,
                        end_tick: new_tick + length,
                        velocity: orig_note.velocity,
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
                        n.id == before.id
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
                        n.id == before.id
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use yinhe_core::{ConductorData, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};

    fn make_doc_with_note() -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData {
                tempo: AutomationLane {
                    target: AutomationTarget::Tempo,
                    track: 0,
                    events: vec![AutomationEvent { tick: 0, value: 120.0, shape: SegmentShape::Step }],
                },
                time_sig: vec![TimeSigEvent { tick: 0, numerator: 4, denominator: 2 }],
            }),
            tracks: vec![Arc::new({
                let mut t = TrackData::new(0, 0);
                t.name = "t".to_string();
                t
            })],
            ..Default::default()
        };
        let mut doc = Document {
            data: crate::project_data::ProjectData::new(
                Arc::new(model),
                vec!["t".to_string()],
                Default::default(),
                Default::default(),
            ),
            edit: crate::edit_state::EditState {
                track_visible: vec![true],
                track_pianoroll_visible: vec![true],
                ..Default::default()
            },
            history: crate::history::UndoStack::new(),
            file_name: "test".into(),
            file_path: None,
        };
        // 加一个音符 (tick 100~200, key 60)
        doc.add_note(0, NoteEvent { id: 0, start_tick: 100, end_tick: 200, key: 60, velocity: 100 });
        // 选中它
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc
    }

    #[test]
    fn duplicate_selected_to_preserves_original_and_offsets_copy() {
        let mut doc = make_doc_with_note();
        let action = doc.duplicate_selected_to(50, 12).expect("should produce action");

        // 原音符保留在 key 60 (tick 100~200)
        assert_eq!(doc.data.model.notes[60].len(), 1, "原音符应在 key 60");
        // 副本在 key 72, tick 150~250
        assert_eq!(doc.data.model.notes[72].len(), 1, "副本应在 key 72");
        let copy = doc.data.model.notes[72][0];
        assert_eq!(copy.start_tick, 150);
        assert_eq!(copy.end_tick, 250);

        // 原音符仍在 key 60
        let orig = doc.data.model.notes[60][0];
        assert_eq!(orig.start_tick, 100);
        assert_eq!(orig.end_tick, 200);

        // 选区跟随副本
        assert_eq!(doc.edit.selected.rects.len(), 1);
        let (ts, te, kl, kh, _tl, _th) = doc.edit.selected.rects[0];
        assert_eq!(ts, 150);
        assert_eq!(te, 251);
        assert_eq!(kl, 72);
        assert_eq!(kh, 72);

        // UndoAction 应该是 Notes，before 空，after 含副本
        match action {
            UndoAction::Notes(delta) => {
                assert!(delta.before.is_empty(), "复制操作 before 应为空");
                assert_eq!(delta.after.len(), 1);
                assert_eq!(delta.after[0].1, 72); // key
            }
            _ => panic!("期望 UndoAction::Notes"),
        }
    }

    #[test]
    fn duplicate_selected_to_empty_selection_returns_none() {
        let mut doc = make_doc_with_note();
        doc.edit.selected.clear();
        assert!(doc.duplicate_selected_to(50, 12).is_none());
    }

    #[test]
    fn duplicate_selected_to_clamps_key_boundary() {
        let mut doc = make_doc_with_note();
        // key 60 + 100 半音 = 160, 应 clamp 到 127
        let _ = doc.duplicate_selected_to(0, 100);
        assert_eq!(doc.data.model.notes[127].len(), 1, "应 clamp 到 key 127");
    }
}
