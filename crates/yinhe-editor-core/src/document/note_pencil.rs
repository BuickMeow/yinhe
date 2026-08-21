//! Single-note editing: pencil-tool drag + velocity edits.
//!
//! 与 `note_edit.rs` 的批量操作对称的单音符编辑入口。
//! - `pencil_drag_note`：铅笔工具的单音符 move/resize
//! - `set_note_velocity` / `set_notes_velocity`：单音符/批量力度修改

use std::sync::Arc;

use yinhe_types::{PencilNoteDrag, VelocityEdit};

use crate::batch_ops;
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
                    .range(*start_tick, start_tick.saturating_add(1))
                    .find(|n| n.track == *track && n.start_tick == *start_tick)?;
                let orig_note = *note;
                let new_key =
                    ((*key as i32) + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                let new_tick = (orig_note.start_tick as i64 + delta_ticks).max(0) as u32;

                if *delta_ticks != 0 || *delta_keys != 0 {
                    // 「允许新重叠音符」关闭：目标与已有音符重叠 → 拒绝移动（模型不动，无 undo）
                    if !self.edit.allow_overlapping_notes {
                        let length = orig_note.end_tick - orig_note.start_tick;
                        if batch_ops::has_overlapping_note_excluding(
                            model,
                            *track,
                            new_key,
                            new_tick,
                            new_tick + length,
                            orig_note.id,
                        ) {
                            return None;
                        }
                    }
                    let model = Arc::make_mut(&mut self.data.model);
                    // Remove original from old key bucket by id
                    Arc::make_mut(&mut model.notes[*key as usize]).remove_by_id(orig_note.id);
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
                    Arc::make_mut(&mut model.notes[nk]).insert_sorted(moved);
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
                    .range(*start_tick, start_tick.saturating_add(1))
                    .find(|n| n.track == *track && n.start_tick == *start_tick)?;
                if *new_end_tick != note.end_tick {
                    let before = *note;
                    // 「允许新重叠音符」关闭：拉伸后与已有音符重叠 → 拒绝（模型不动）
                    if !self.edit.allow_overlapping_notes {
                        let new_end = (*new_end_tick).max(before.start_tick + 1);
                        if batch_ops::has_overlapping_note_excluding(
                            model,
                            *track,
                            *key,
                            before.start_tick,
                            new_end,
                            before.id,
                        ) {
                            return None;
                        }
                    }
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k]).find_mut(before.id) {
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
                    .range(*start_tick, start_tick.saturating_add(1))
                    .find(|n| n.track == *track && n.start_tick == *start_tick)?;
                if *new_start_tick != note.start_tick {
                    let before = *note;
                    // 「允许新重叠音符」关闭：拉伸后与已有音符重叠 → 拒绝（模型不动）
                    if !self.edit.allow_overlapping_notes {
                        let new_start = (*new_start_tick).min(before.end_tick - 1);
                        if batch_ops::has_overlapping_note_excluding(
                            model,
                            *track,
                            *key,
                            new_start,
                            before.end_tick,
                            before.id,
                        ) {
                            return None;
                        }
                    }
                    let model = Arc::make_mut(&mut self.data.model);
                    let bucket = Arc::make_mut(&mut model.notes[k]);
                    let mut moved = bucket.remove_by_id(before.id)?;
                    moved.start_tick = (*new_start_tick).min(moved.end_tick - 1);
                    // start_tick 是排序键：改值后按排序键重新插入。
                    bucket.insert_sorted(moved);
                    let after = moved;
                    model.mark_dirty(*key);
                    model.rebuild_dirty();
                    self.data.bump_revision();
                    return Some(UndoAction::Notes(NoteDelta {
                        before: vec![(before, *key)],
                        after: vec![(after, *key)],
                    }));
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
        // 实际命中的 (track, start_tick, new_vel)：用于记录"最近修改力度"。
        let mut remembered: Vec<(u16, u32, u8)> = Vec::new();
        {
            let model = &self.data.model;
            for e in edits {
                let note = model.notes[e.key as usize]
                    .range(e.start_tick, e.start_tick.saturating_add(1))
                    .find(|n| n.track == e.track);
                if let Some(n) = note
                    && n.velocity != e.velocity
                {
                    targets.push((e.key, n.id, n.velocity, e.velocity));
                    remembered.push((e.track, e.start_tick, e.velocity));
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
            // 收集阶段与修改阶段之间无并发修改，目标必然存在。
            let n = Arc::make_mut(&mut model.notes[k])
                .find_mut(id)
                .expect("velocity target vanished");
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
        if before.is_empty() {
            return None;
        }
        model.rebuild_dirty();
        self.data.bump_revision();
        // 记录"最近修改力度"：新音符默认力度跟随最近一次修改（同轨取时间最晚的音符）。
        for (track, start_tick, velocity) in remembered {
            self.edit.remember_velocity(track, start_tick, velocity);
        }
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
    fn pencil_resize_left_keeps_bucket_sorted() {
        let mut doc = make_doc_with_note(); // tick 100~200 key 60
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 400,
                key: 60,
                velocity: 100,
            },
        );
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: 500,
                end_tick: 600,
                key: 60,
                velocity: 100,
            },
        );
        // 桶: [100, 300, 500]

        // 左边缘拖到最前（50）→ 被拖音符必须移到桶头，否则桶失序。
        let action = doc
            .pencil_drag_note(&PencilNoteDrag::ResizeLeft {
                track: 0,
                start_tick: 300,
                key: 60,
                new_start_tick: 50,
            })
            .expect("应产生 UndoAction");
        let bucket = &doc.data.model.notes[60];
        assert!(bucket.is_sorted(), "桶失序");
        assert_eq!(
            bucket.iter().map(|n| n.start_tick).collect::<Vec<_>>(),
            vec![50, 100, 500],
            "左边缘拖到最前时音符应重新定位"
        );
        assert_eq!(
            bucket[0].end_tick, 400,
            "start_tick 改变时 end_tick 保持原值（长度改变）"
        );
        let UndoAction::Notes(delta) = action else {
            panic!("应产生 Notes undo");
        };
        assert_eq!(delta.after[0].0.start_tick, 50);

        // 左边缘拖到中间（450）→ 插到 500 之前。
        doc.pencil_drag_note(&PencilNoteDrag::ResizeLeft {
            track: 0,
            start_tick: 500,
            key: 60,
            new_start_tick: 450,
        })
        .expect("应产生 UndoAction");
        let bucket = &doc.data.model.notes[60];
        assert!(bucket.is_sorted(), "桶失序");
        assert_eq!(
            bucket.iter().map(|n| n.start_tick).collect::<Vec<_>>(),
            vec![50, 100, 450],
            "左边缘拖到中间时音符应重新定位"
        );

        // 左边缘超过 end_tick-1 → clamp 到 end_tick-1，桶仍有序。
        doc.pencil_drag_note(&PencilNoteDrag::ResizeLeft {
            track: 0,
            start_tick: 100,
            key: 60,
            new_start_tick: 999,
        })
        .expect("应产生 UndoAction");
        let bucket = &doc.data.model.notes[60];
        assert!(bucket.is_sorted(), "桶失序");
        assert_eq!(bucket[0].start_tick, 50);
        assert_eq!(bucket[1].start_tick, 199, "clamp 到 end_tick-1 (200-1)");
        assert_eq!(bucket[2].start_tick, 450);
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

    #[test]
    fn set_notes_velocity_remembers_latest_tick() {
        let mut doc = make_doc_with_note();
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: 200,
                end_tick: 300,
                key: 60,
                velocity: 90,
            },
        );
        // 一笔修改两个音符：t100 改 75，t200 改 60 → 记录时间最晚（t200）的 60
        let edits = [
            VelocityEdit {
                track: 0,
                start_tick: 100,
                key: 60,
                velocity: 75,
            },
            VelocityEdit {
                track: 0,
                start_tick: 200,
                key: 60,
                velocity: 60,
            },
        ];
        assert!(doc.set_notes_velocity(&edits).is_some());
        assert_eq!(doc.edit.default_velocity(0), 60);
        // 未命中/未变化的修改不记录
        let edits = [VelocityEdit {
            track: 0,
            start_tick: 999,
            key: 60,
            velocity: 10,
        }];
        assert!(doc.set_notes_velocity(&edits).is_none());
        assert_eq!(doc.edit.default_velocity(0), 60);
    }
}
