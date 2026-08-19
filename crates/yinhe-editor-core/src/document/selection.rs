//! Selection operations: select-all and paste.

use std::sync::Arc;

use yinhe_types::MAX_KEY;

use crate::batch_ops;
use crate::history::{NoteDelta, UndoAction};

use super::Document;

impl Document {
    /// Select all notes in the currently selected track(s) for Piano Roll.
    /// Range: tick 0 → last note end (global), keys 0–MAX_KEY.
    ///
    /// Uses `model.tick_length` (O(1)) instead of scanning all key buckets (O(N)).
    /// Sets `sel_rect.rect` to the full global range so the visual selection box
    /// covers 0 → tick_length, keys 0–MAX_KEY.
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
            self.edit
                .selected
                .add_rect_track(0, max_end + 1, 0, MAX_KEY, track_idx, track_idx);
        }

        // Update visual sel_rect to show full range (PR uses f64 ticks).
        // 全选是全键选框，但属于用户主动选择（非空区域框选自动切换），
        // 不标记 auto_vertical —— 拖动时仍可上下移动。
        self.edit.sel_rect.rects = vec![(0.0, max_end as f64 + 1.0, 0, MAX_KEY)];
        self.edit.sel_rect.auto_vertical = vec![false];
    }

    /// Select all notes across all tracks for Arrange.
    /// Range: tick 0 → global last note end, keys 0–MAX_KEY, all tracks except conductor.
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
                self.edit
                    .selected
                    .add_rect_track(0, max_end + 1, 0, MAX_KEY, 0, c - 1);
            }
            _ => {}
        }
        let after = conductor.map(|c| c + 1).unwrap_or(0);
        if after < num_tracks {
            self.edit
                .selected
                .add_rect_track(0, max_end + 1, 0, MAX_KEY, after, num_tracks - 1);
        }
        // AR 选框：全范围单矩形（含 conductor track），供 AR 视图绘制。
        self.edit.arr_sel_rect = vec![(0.0, (max_end + 1) as f64, 0, num_tracks as usize - 1)];
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
                .or_else(|| self.history.past.back());
            if let Some(entry) = entry
                && let UndoAction::Notes(delta) = &entry.action
                && !delta.before.is_empty()
            {
                notes = delta
                    .before
                    .iter()
                    .filter(|(n, key)| clipboard.contains(n.track, n.start_tick, *key))
                    .cloned()
                    .collect();
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
        let allow_overlap = self.edit.allow_overlapping_notes;
        let model = Arc::make_mut(&mut self.data.model);

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        for (note, key) in &notes {
            if Some(note.track) == conductor {
                continue;
            }
            let new_start = (note.start_tick as i64 + offset).max(0) as u32;
            let new_end = (note.end_tick as i64 + offset).max(0) as u32;
            let new_track = (note.track as i32 + track_offset).clamp(0, u16::MAX as i32) as u16;
            // 「允许新重叠音符」关闭：粘贴副本与已有音符重叠 → 跳过该副本。
            // 检查在批量插入前进行，批次内部互不影响（含剪贴板源音符）。
            if !allow_overlap
                && batch_ops::has_overlapping_note(model, new_track, *key, new_start, new_end)
            {
                continue;
            }
            let new_note = yinhe_types::Note {
                id: model.alloc_note_id(),
                start_tick: new_start,
                end_tick: new_end,
                velocity: note.velocity,
                track: new_track,
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
        self.edit
            .selected
            .add_rect_track(min_tick, max_end + 1, 0, MAX_KEY, track_lo, track_hi);

        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};

    fn make_doc() -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData::default()),
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        Document {
            data: crate::project_data::ProjectData::new(
                Arc::new(model),
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
            mixer: Default::default(),
            mixer_dirty: false,
        }
    }

    fn add(doc: &mut Document, start: u32, end: u32, key: u8) {
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: end,
                key,
                velocity: 100,
            },
        );
    }

    /// paste_from_selection：与已有音符重叠的粘贴副本跳过，其余正常插入。
    #[test]
    fn paste_skips_overlapping_notes_when_disallowed() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // 源 A
        add(&mut doc, 100, 150, 62); // 源 B
        add(&mut doc, 450, 550, 60); // 占位 C（粘贴目标区）
        doc.edit.allow_overlapping_notes = false;

        // 剪贴板 = 源音符所在选框（只含 A、B）
        let mut clipboard = yinhe_core::Selection::default();
        clipboard.add_rect_track(100, 201, 60, 62, 0, 0);

        // 粘贴到 400：A 副本 [400,500) 与 C 相交 → 跳过；B 副本 k62 [400,450) → 插入
        let action = doc
            .paste_from_selection(&clipboard, 400.0, None, &std::collections::HashSet::new())
            .expect("应有部分副本插入");
        match action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.after.len(), 1, "只有 B 的副本被插入");
                assert_eq!(delta.after[0].1, 62);
            }
            other => panic!("期望 UndoAction::Notes，实际 {other:?}"),
        }
        assert_eq!(doc.data.model.notes[60].len(), 2, "k60 只有 A 和 C");
        assert_eq!(doc.data.model.notes[62].len(), 2, "B 及其副本");
    }

    /// paste_from_selection：副本全被拦时返回 None，模型不变。
    #[test]
    fn paste_all_blocked_returns_none() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // 源 A
        add(&mut doc, 450, 550, 60); // 占位 C（与 A 的副本 [400,500) 相交）
        doc.edit.allow_overlapping_notes = false;

        let mut clipboard = yinhe_core::Selection::default();
        clipboard.add_rect_track(100, 201, 60, 60, 0, 0);
        assert!(
            doc.paste_from_selection(&clipboard, 400.0, None, &std::collections::HashSet::new())
                .is_none(),
            "副本全被拦时应返回 None"
        );
        assert_eq!(doc.data.model.notes[60].len(), 2, "模型不应变化");
    }

    /// 默认允许重叠：粘贴照常（现状行为）。
    #[test]
    fn paste_allows_overlap_by_default() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60);
        add(&mut doc, 450, 550, 60);
        let mut clipboard = yinhe_core::Selection::default();
        clipboard.add_rect_track(100, 201, 60, 60, 0, 0);
        assert!(
            doc.paste_from_selection(&clipboard, 400.0, None, &std::collections::HashSet::new())
                .is_some(),
            "默认应允许重叠粘贴"
        );
        assert_eq!(doc.data.model.notes[60].len(), 3);
    }
}
