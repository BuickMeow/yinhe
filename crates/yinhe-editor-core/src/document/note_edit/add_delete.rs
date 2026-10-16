//! 音符增删：单音符添加/闭合 gate、按 id 删除、选中删除。

use std::sync::Arc;

use yinhe_core::NoteEvent;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

impl Document {
    /// Add a single note. Returns an `UndoAction` if the note was added.
    pub fn add_note(&mut self, track_idx: u16, note: NoteEvent) -> Option<UndoAction> {
        let t = track_idx as usize;
        if t >= self.data.model.tracks.len() {
            return None;
        }
        if Some(track_idx) == self.edit.track_cache.conductor_idx {
            return None;
        }
        let key = note.key;
        // 「允许新重叠音符」关闭：与已有音符重叠的新音符整个无视（不分配 id）。
        if !self.edit.allow_overlapping_notes
            && batch_ops::has_overlapping_note(
                &self.data.model,
                track_idx,
                key,
                note.start_tick,
                note.end_tick,
            )
        {
            return None;
        }
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
        Arc::make_mut(&mut self.data.model).insert_note(key, typed_note);
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after: vec![(typed_note, key)],
        }))
    }

    /// 修改指定音符的 `end_tick`（MIDI 录音 NoteOff 时闭合 gate）。
    ///
    /// 按 (key, note_id) 寻址；音符不存在返回 `None`。
    /// 新 end_tick 会被钳制为至少 `start_tick + 1`（保证 gate >= 1 tick）。
    pub fn set_note_end_tick(
        &mut self,
        key: u8,
        note_id: u32,
        end_tick: u32,
    ) -> Option<UndoAction> {
        let orig = *self.data.model.note_by_id(key, note_id)?;
        let new_end = end_tick.max(orig.start_tick + 1);
        if orig.end_tick == new_end {
            return None;
        }
        let new_note =
            Arc::make_mut(&mut self.data.model)
                .update_note_by_id(key, note_id, |n| n.end_tick = new_end)?;
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![(orig, key)],
            after: vec![(new_note, key)],
        }))
    }

    /// Delete a single note by `(track, start_tick, key)`. Used by quick-delete (double-click / right-click).
    pub fn delete_single_note(
        &mut self,
        track: u16,
        start_tick: u32,
        key: u8,
    ) -> Option<UndoAction> {
        let bucket = &self.data.model.notes[key as usize];
        let note = *bucket
            .range(start_tick, start_tick.saturating_add(1))
            .find(|n| n.track == track && n.start_tick == start_tick)?;
        let model = Arc::make_mut(&mut self.data.model);
        let removed = model.remove_note_by_id(key, note.id)?;
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![(removed, key)],
            after: vec![],
        }))
    }

    /// Delete many notes by `(track, start_tick, key)` in one batched pass.
    /// 用于右键拖拽擦除：整段路径合并为一个 undo 条目（只 rebuild/bump 一次）。
    pub fn delete_notes(&mut self, notes: &[(u16, u32, u8)]) -> Option<UndoAction> {
        if notes.is_empty() {
            return None;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let mut removed = Vec::new();
        for &(track, start_tick, key) in notes {
            let found = model.notes[key as usize]
                .range(start_tick, start_tick.saturating_add(1))
                .find(|n| n.track == track && n.start_tick == start_tick)
                .map(|n| n.id);
            if let Some(id) = found
                && let Some(r) = model.remove_note_by_id(key, id)
            {
                removed.push((r, key));
            }
        }
        if removed.is_empty() {
            return None;
        }
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: removed,
            after: vec![],
        }))
    }

    /// Delete all selected notes. Returns an `UndoAction` if any notes were deleted.
    pub fn delete_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        // 一次遍历完成「判空 + 删除」：remove_selected 的返回值即 undo 的 before
        //（原实现先 collect 判空、再 remove 遍历，全选删除会多物化一份 ~3GB）。
        let model = Arc::make_mut(&mut self.data.model);
        let removed = batch_ops::remove_selected(model, &self.edit.selected);
        self.edit.selected.clear();
        if removed.is_empty() {
            return None;
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: removed,
            after: vec![],
        }))
    }
}
