//! Single-note editing: pencil-tool drag + velocity edits.
//!
//! 与 `note_edit.rs` 的批量操作对称的单音符编辑入口。
//! - `pencil_drag_note`：铅笔工具的单音符 move/resize
//! - `set_note_velocity` / `set_notes_velocity`：单音符/批量力度修改

use std::sync::Arc;

use yinhe_types::{PencilNoteDrag, VelocityEdit};

use crate::history::{NoteDelta, UndoAction};

use super::Document;

impl Document {
    /// Apply a pencil-tool drag operation (move or resize a single note).
    ///
    /// Returns an `UndoAction` if the note was modified. The caller is
    /// responsible for pushing it to the history stack, marking the view
    /// dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn pencil_drag_note(&mut self, drag: &PencilNoteDrag) -> Option<UndoAction> {
        match drag {
            PencilNoteDrag::Move {
                track,
                start_tick,
                key,
                delta_ticks,
                delta_keys,
            } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k]
                    .iter()
                    .find(|n| n.track == *track && n.start_tick == *start_tick)?;
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
                    let insert_pos =
                        model.notes[nk].partition_point(|n| n.start_tick < moved.start_tick);
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
            PencilNoteDrag::ResizeRight {
                track,
                start_tick,
                key,
                new_end_tick,
            } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k]
                    .iter()
                    .find(|n| n.track == *track && n.start_tick == *start_tick)?;
                if *new_end_tick != note.end_tick {
                    let before = *note;
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k])
                        .iter_mut()
                        .find(|n| n.id == before.id)
                    {
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
            PencilNoteDrag::ResizeLeft {
                track,
                start_tick,
                key,
                new_start_tick,
            } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k]
                    .iter()
                    .find(|n| n.track == *track && n.start_tick == *start_tick)?;
                if *new_start_tick != note.start_tick {
                    let before = *note;
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k])
                        .iter_mut()
                        .find(|n| n.id == before.id)
                    {
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

    /// 修改单个音符的 velocity（按 (track, start_tick, key) 寻址）。
    ///
    /// 与 [`pencil_drag_note`] 对称的单音符编辑入口，供事件浏览器右键编辑、
    /// 铅笔工具的力度修改等场景共用。仅修改 `velocity`，不动 tick / key / track。
    pub fn set_note_velocity(
        &mut self,
        track_idx: u16,
        start_tick: u32,
        key: u8,
        new_velocity: u8,
    ) -> Option<UndoAction> {
        self.set_notes_velocity(&[VelocityEdit {
            track: track_idx,
            start_tick,
            key,
            velocity: new_velocity,
        }])
    }

    /// 批量修改多个音符的 velocity（一笔 velocity 笔划 = 一个 undo entry）。
    ///
    /// 与 [`set_note_velocity`] 同一实现：未命中的 (track, start_tick, key)
    /// 和 velocity 未变化的条目会被跳过；全部被跳过时返回 `None`。
    pub fn set_notes_velocity(&mut self, edits: &[VelocityEdit]) -> Option<UndoAction> {
        // 先定位目标音符并记录原值（只读，按 start_tick 二分）。
        let mut targets: Vec<(u8, u32, u8, u8)> = Vec::new(); // (key, id, old_vel, new_vel)
        {
            let model = &self.data.model;
            for e in edits {
                let bucket = &model.notes[e.key as usize];
                let lo = bucket.partition_point(|n| n.start_tick < e.start_tick);
                let note = bucket[lo..]
                    .iter()
                    .take_while(|n| n.start_tick == e.start_tick)
                    .find(|n| n.track == e.track);
                if let Some(n) = note
                    && n.velocity != e.velocity
                {
                    targets.push((e.key, n.id, n.velocity, e.velocity));
                }
            }
        }
        if targets.is_empty() {
            return None;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let mut before = Vec::with_capacity(targets.len());
        let mut after = Vec::with_capacity(targets.len());
        for (key, id, old_vel, new_vel) in targets {
            let k = key as usize;
            if let Some(n) = Arc::make_mut(&mut model.notes[k])
                .iter_mut()
                .find(|n| n.id == id)
            {
                n.velocity = new_vel;
                before.push((
                    yinhe_types::Note {
                        velocity: old_vel,
                        ..*n
                    },
                    key,
                ));
                after.push((*n, key));
                model.mark_dirty(key);
            }
        }
        if before.is_empty() {
            return None;
        }
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};
    use yinhe_types::{
        AutomationEvent, AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent,
    };

    fn make_doc_with_note() -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData {
                tempo: AutomationLane {
                    target: AutomationTarget::Tempo,
                    track: 0,
                    events: vec![AutomationEvent {
                        tick: 0,
                        value: 120.0,
                        shape: SegmentShape::Step,
                    }],
                },
                time_sig: vec![TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                }],
                key_sig: Vec::new(),
                markers: Vec::new(),
                lyrics: Vec::new(),
                chord: Vec::new(),
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
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: 100,
                end_tick: 200,
                key: 60,
                velocity: 100,
            },
        );
        // 选中它
        doc.edit.selected.add_rect_track(100, 201, 60, 60, 0, 0);
        doc
    }

    #[test]
    fn set_note_velocity_updates_velocity_and_returns_undo() {
        let mut doc = make_doc_with_note();
        // 初始 velocity = 100
        assert_eq!(doc.data.model.notes[60][0].velocity, 100);

        let action = doc
            .set_note_velocity(0, 100, 60, 80)
            .expect("应产生 UndoAction");
        assert_eq!(
            doc.data.model.notes[60][0].velocity, 80,
            "velocity 应已更新为 80"
        );

        // UndoAction 应记录 before/after
        match action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1);
                assert_eq!(delta.before[0].0.velocity, 100, "before 应记录原 velocity");
                assert_eq!(delta.after.len(), 1);
                assert_eq!(delta.after[0].0.velocity, 80, "after 应记录新 velocity");
            }
            _ => panic!("期望 UndoAction::Notes"),
        }
    }

    #[test]
    fn set_note_velocity_unchanged_returns_none() {
        let mut doc = make_doc_with_note();
        // 当前 velocity = 100, 改成 100 应返回 None
        assert!(doc.set_note_velocity(0, 100, 60, 100).is_none());
    }

    #[test]
    fn set_note_velocity_missing_note_returns_none() {
        let mut doc = make_doc_with_note();
        // 不存在的 start_tick
        assert!(doc.set_note_velocity(0, 9999, 60, 80).is_none());
        // 不存在的 track
        assert!(doc.set_note_velocity(99, 100, 60, 80).is_none());
        // 不存在的 key
        assert!(doc.set_note_velocity(0, 100, 99, 80).is_none());
    }

    #[test]
    fn set_notes_velocity_batch_single_undo_entry() {
        let mut doc = make_doc_with_note();
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 400,
                key: 64,
                velocity: 90,
            },
        );
        let edits = [
            VelocityEdit {
                track: 0,
                start_tick: 100,
                key: 60,
                velocity: 80,
            },
            VelocityEdit {
                track: 0,
                start_tick: 300,
                key: 64,
                velocity: 70,
            },
        ];
        let action = doc.set_notes_velocity(&edits).expect("应产生 UndoAction");
        assert_eq!(doc.data.model.notes[60][0].velocity, 80);
        assert_eq!(doc.data.model.notes[64][0].velocity, 70);
        match action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 2, "一笔批量修改合并为一个 undo entry");
                assert_eq!(delta.after.len(), 2);
            }
            _ => panic!("期望 UndoAction::Notes"),
        }
    }

    #[test]
    fn set_notes_velocity_skips_missing_and_unchanged() {
        let mut doc = make_doc_with_note();
        // 全部无效：未命中 + 值未变化
        let edits = [
            VelocityEdit {
                track: 0,
                start_tick: 9999,
                key: 60,
                velocity: 80,
            },
            VelocityEdit {
                track: 0,
                start_tick: 100,
                key: 60,
                velocity: 100,
            },
        ];
        assert!(doc.set_notes_velocity(&edits).is_none());
        // 部分有效：只应用有效的那条
        let edits = [
            VelocityEdit {
                track: 1,
                start_tick: 100,
                key: 60,
                velocity: 80,
            },
            VelocityEdit {
                track: 0,
                start_tick: 100,
                key: 60,
                velocity: 55,
            },
        ];
        let action = doc
            .set_notes_velocity(&edits)
            .expect("部分有效应产生 UndoAction");
        assert_eq!(doc.data.model.notes[60][0].velocity, 55);
        match action {
            UndoAction::Notes(delta) => assert_eq!(delta.before.len(), 1),
            _ => panic!("期望 UndoAction::Notes"),
        }
    }
}
