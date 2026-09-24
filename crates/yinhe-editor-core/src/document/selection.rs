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

    /// Paste notes from the clipboard snapshot.
    ///
    /// The clipboard holds an `Arc<YinModel>` snapshot taken at copy time, so
    /// notes edited/deleted/cut after the copy are still pasted. No undo-stack
    /// bridge is needed.
    ///
    /// Returns `(undo action, content tick span)`; the span feeds the
    /// consecutive-paste advance in the UI layer.
    pub fn paste_notes(
        &mut self,
        clipboard: &crate::clipboard::NotesClipboard,
        cursor_tick: f64,
        track_selected: &std::collections::HashSet<u16>,
        mode: crate::clipboard::PasteMode,
    ) -> Option<(UndoAction, u32)> {
        use crate::clipboard::PasteMode;

        if clipboard.is_empty() {
            return None;
        }

        // 第一遍：流式扫源范围（不物化全量；1.64 亿选区下 collect 会多一份 3GB）。
        let mut src_min_start = u32::MAX;
        let mut src_max_end = 0u32;
        let mut src_min_track = u16::MAX;
        let mut src_count = 0usize;
        clipboard.for_each_note(|note, _| {
            src_min_start = src_min_start.min(note.start_tick);
            src_max_end = src_max_end.max(note.end_tick);
            src_min_track = src_min_track.min(note.track);
            src_count += 1;
        });
        if src_count == 0 {
            return None;
        }
        let span = src_max_end.saturating_sub(src_min_start);

        // AtCursor/Flipped 以光标为基准；AtOriginal 保持源坐标。
        let offset = cursor_tick as i64 - src_min_start as i64;

        // Calculate track offset: first selected track - min source track.
        // If no track is selected, keep original track positions.
        // AtOriginal 完全原位，不做轨道偏移。
        let track_offset: i32 = if !track_selected.is_empty() && mode != PasteMode::AtOriginal {
            let first_selected = track_selected.iter().min().copied().unwrap_or(0);
            first_selected as i32 - src_min_track as i32
        } else {
            0
        };

        let conductor = self.edit.conductor_track_idx;
        let allow_overlap = self.edit.allow_overlapping_notes;
        let model = Arc::make_mut(&mut self.data.model);

        // 第二遍：一次遍历同时构建「按 key 分组的插入批次」与「undo 的 after」，
        // 两者内容同源但不共享（insert_batch 会消耗分组表）。
        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::with_capacity(src_count);
        clipboard.for_each_note(|note, key| {
            if Some(note.track) == conductor {
                return;
            }
            let (new_start, new_end) = match mode {
                PasteMode::AtOriginal => (note.start_tick, note.end_tick),
                PasteMode::AtCursor => (
                    (note.start_tick as i64 + offset).max(0) as u32,
                    (note.end_tick as i64 + offset).max(0) as u32,
                ),
                PasteMode::Flipped => {
                    // 时间镜像：新起点 = 光标 + (跨度 - (源终点 - 源最早起点))，
                    // 音高/力度/gate 不变。
                    let gate = note.end_tick.saturating_sub(note.start_tick) as i64;
                    let mirrored = cursor_tick as i64 + span as i64
                        - (note.end_tick as i64 - src_min_start as i64);
                    let ns = mirrored.max(0) as u32;
                    (ns, ns.saturating_add(gate as u32))
                }
            };
            let new_track = (note.track as i32 + track_offset).clamp(0, u16::MAX as i32) as u16;
            // 「允许新重叠音符」关闭：粘贴副本与已有音符重叠 → 跳过该副本。
            // 检查在批量插入前进行，批次内部互不影响（含剪贴板源音符）。
            if !allow_overlap
                && batch_ops::has_overlapping_note(model, new_track, key, new_start, new_end)
            {
                return;
            }
            let new_note = yinhe_types::Note {
                id: model.alloc_note_id(),
                start_tick: new_start,
                end_tick: new_end,
                velocity: note.velocity,
                track: new_track,
            };
            new_by_key.entry(key).or_default().push(new_note);
            after.push((new_note, key));
        });

        if new_by_key.is_empty() {
            return None;
        }

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
        // 选区精确跟随粘贴出的副本（新 id），bbox 内的其他音符不纳入。
        self.edit
            .selected
            .set_members(after.iter().map(|(n, _)| n.id));

        self.data.rebuild_model_dirty();
        Some((
            UndoAction::Notes(NoteDelta {
                before: vec![],
                after,
            }),
            span,
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{NotesClipboard, PasteMode};
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
            doc_id: 0,
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

    /// 用当前文档模型 + 指定选框构造剪贴板快照。
    fn clipboard_with(doc: &Document, selection: yinhe_core::Selection) -> NotesClipboard {
        NotesClipboard::from_snapshot(doc.data.model.clone(), selection)
    }

    /// 与已有音符重叠的粘贴副本跳过，其余正常插入。
    #[test]
    fn paste_skips_overlapping_notes_when_disallowed() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // 源 A
        add(&mut doc, 100, 150, 62); // 源 B
        add(&mut doc, 450, 550, 60); // 占位 C（粘贴目标区）
        doc.edit.allow_overlapping_notes = false;

        // 剪贴板 = 源音符所在选框（只含 A、B）
        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 62, 0, 0);
        let clipboard = clipboard_with(&doc, selection);

        // 粘贴到 400：A 副本 [400,500) 与 C 相交 → 跳过；B 副本 k62 [400,450) → 插入
        let (action, span) = doc
            .paste_notes(
                &clipboard,
                400.0,
                &std::collections::HashSet::new(),
                PasteMode::AtCursor,
            )
            .expect("应有部分副本插入");
        assert_eq!(span, 100, "跨度 = 源内容 max_end - min_start");
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

    /// 副本全被拦时返回 None，模型不变。
    #[test]
    fn paste_all_blocked_returns_none() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60); // 源 A
        add(&mut doc, 450, 550, 60); // 占位 C（与 A 的副本 [400,500) 相交）
        doc.edit.allow_overlapping_notes = false;

        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 60, 0, 0);
        let clipboard = clipboard_with(&doc, selection);
        assert!(
            doc.paste_notes(
                &clipboard,
                400.0,
                &std::collections::HashSet::new(),
                PasteMode::AtCursor
            )
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
        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 60, 0, 0);
        let clipboard = clipboard_with(&doc, selection);
        assert!(
            doc.paste_notes(
                &clipboard,
                400.0,
                &std::collections::HashSet::new(),
                PasteMode::AtCursor
            )
            .is_some(),
            "默认应允许重叠粘贴"
        );
        assert_eq!(doc.data.model.notes[60].len(), 3);
    }

    /// 快照语义：复制后源音符被删除（等同 cut），粘贴内容不受影响。
    #[test]
    fn paste_uses_snapshot_after_source_deleted() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60);

        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 60, 0, 0);
        let clipboard = clipboard_with(&doc, selection);

        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc.delete_selected();
        assert_eq!(doc.data.model.notes[60].len(), 0, "源已删除");

        assert!(
            doc.paste_notes(
                &clipboard,
                400.0,
                &std::collections::HashSet::new(),
                PasteMode::AtCursor
            )
            .is_some(),
            "源删除后仍应从快照粘贴"
        );
        assert_eq!(doc.data.model.notes[60].len(), 1);
        let pasted = &doc.data.model.notes[60][0];
        assert_eq!(pasted.start_tick, 400);
        assert_eq!(pasted.end_tick, 500);
    }

    /// 快照语义：跨文档粘贴使用源文档复制时刻的内容。
    #[test]
    fn paste_across_documents_uses_source_snapshot() {
        let mut src = make_doc();
        add(&mut src, 100, 200, 60);
        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 60, 0, 0);
        let clipboard = clipboard_with(&src, selection);

        let mut dst = make_doc();
        // 目标文档同坐标区没有音符；若旧实现「重查当前文档」会粘空。
        assert!(
            dst.paste_notes(
                &clipboard,
                300.0,
                &std::collections::HashSet::new(),
                PasteMode::AtCursor
            )
            .is_some(),
            "跨文档粘贴应使用源快照"
        );
        assert_eq!(dst.data.model.notes[60].len(), 1);
        assert_eq!(dst.data.model.notes[60][0].start_tick, 300);
    }

    /// 原位置粘贴：保持源坐标，忽略光标与目标轨道。
    #[test]
    fn paste_at_original_keeps_source_coordinates() {
        let mut doc = make_doc();
        add(&mut doc, 100, 200, 60);
        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 60, 0, 0);
        let clipboard = clipboard_with(&doc, selection);

        doc.edit.selected.clear();
        doc.paste_notes(
            &clipboard,
            9999.0,
            &std::collections::HashSet::new(),
            PasteMode::AtOriginal,
        )
        .expect("原位置粘贴应成功");

        let notes = &doc.data.model.notes[60];
        assert_eq!(notes.len(), 2);
        assert!(
            notes
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200),
            "源坐标应保持不变"
        );
    }

    /// 翻转粘贴：tick 轴镜像，声部前后顺序反转，gate 不变。
    #[test]
    fn paste_flipped_mirrors_time_axis() {
        let mut doc = make_doc();
        add(&mut doc, 100, 150, 60); // A：靠前、短
        add(&mut doc, 180, 200, 62); // B：靠后
        let mut selection = yinhe_core::Selection::default();
        selection.add_rect_track(100, 201, 60, 62, 0, 0);
        let clipboard = clipboard_with(&doc, selection);

        doc.edit.selected.clear();
        doc.paste_notes(
            &clipboard,
            400.0,
            &std::collections::HashSet::new(),
            PasteMode::Flipped,
        )
        .expect("翻转粘贴应成功");

        // span = 100（100..200）。A 镜像后 [450,500)，B 镜像后 [400,420)。
        let a = doc.data.model.notes[60]
            .iter()
            .find(|n| n.start_tick >= 400)
            .expect("A 的镜像副本");
        assert_eq!((a.start_tick, a.end_tick), (450, 500));
        let b = doc.data.model.notes[62]
            .iter()
            .find(|n| n.start_tick >= 400)
            .expect("B 的镜像副本");
        assert_eq!((b.start_tick, b.end_tick), (400, 420));
    }
}
