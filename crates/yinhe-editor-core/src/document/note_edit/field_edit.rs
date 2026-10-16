//! Info 面板表达式编辑（力度 / gate / key / tick）。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};
use crate::num_expr::{NumOp, apply_ops_round};

use super::NoteField;

impl Document {
    /// 对选中音符批量应用表达式编辑（Info 面板选框编辑）。
    ///
    /// - Velocity/Gate：就地修改（不改变排序位置）
    /// - Key/Tick：remove + 变换 + 重插
    ///
    /// 加减（所有变化项 delta 一致）时选框跟随平移；乘除/赋值导致
    /// delta 不一致时选框保持不动。返回 `None` 表示没有音符被修改。
    pub fn apply_note_field_edit(&mut self, field: NoteField, ops: &[NumOp]) -> Option<UndoAction> {
        if self.edit.selected.is_empty() || ops.is_empty() {
            return None;
        }
        match field {
            NoteField::Velocity | NoteField::Gate => self.edit_note_props(field, ops),
            NoteField::Key | NoteField::Tick => self.edit_note_positions(field, ops),
        }
    }

    /// Velocity/Gate：就地修改，不换桶。Gate 加减 uniform 时选框 te 跟随。
    ///
    /// 单趟完成：原地改音符 + 收集 before/after + remember_*（不再先物化
    /// `targets` 中间层——1.64 亿全选下那会多占 ~6GB）。选区先 clone 解开
    /// `self.edit` 与 `self.data.model` 的借用冲突（Selection clone 只深拷
    /// 少量 rects / 位图 BTreeMap，页 Arc 共享）。
    fn edit_note_props(&mut self, field: NoteField, ops: &[NumOp]) -> Option<UndoAction> {
        let selection = self.edit.selected.clone();
        let model = Arc::make_mut(&mut self.data.model);
        let mut before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut uniform_delta: Option<i64> = None; // 全部变化项相同 delta 时 Some（gate 加减）
        let mut any = false;

        for key in 0..yinhe_types::KEY_COUNT {
            let k = key as u8;
            let bucket = Arc::make_mut(&mut model.notes[key]);
            let touched = bucket.update_matching(
                |n| {
                    selection.rects.iter().any(|&(ts, te, kl, kh, tl, th)| {
                        n.start_tick >= ts
                            && n.start_tick < te
                            && k >= kl
                            && k <= kh
                            && n.track >= tl
                            && n.track <= th
                    }) && selection.accepts_note(n, k)
                },
                |n| {
                    let new = match field {
                        NoteField::Velocity => {
                            let v = apply_ops_round(ops, n.velocity as f64).clamp(0.0, 127.0) as u8;
                            yinhe_types::Note { velocity: v, ..*n }
                        }
                        NoteField::Gate => {
                            let gate = (n.end_tick - n.start_tick) as f64;
                            let new_gate =
                                apply_ops_round(ops, gate).clamp(1.0, u32::MAX as f64) as u32;
                            yinhe_types::Note {
                                end_tick: n.start_tick + new_gate,
                                ..*n
                            }
                        }
                        _ => unreachable!(),
                    };
                    // Note 无 PartialEq，按变更字段比较
                    let changed = match field {
                        NoteField::Velocity => new.velocity != n.velocity,
                        NoteField::Gate => new.end_tick != n.end_tick,
                        _ => unreachable!(),
                    };
                    if !changed {
                        return;
                    }
                    if field == NoteField::Gate {
                        let d = new.end_tick as i64 - n.end_tick as i64;
                        match uniform_delta {
                            None => uniform_delta = Some(d),
                            Some(u) if u != d => uniform_delta = None,
                            _ => {}
                        }
                    }
                    match field {
                        // 记录"最近修改"：新音符默认值跟随最近一次修改（同轨取时间最晚）。
                        NoteField::Velocity => {
                            self.edit
                                .remember_velocity(n.track, n.start_tick, new.velocity);
                        }
                        NoteField::Gate => {
                            let gate = new.end_tick - new.start_tick;
                            self.edit.remember_gate(n.track, n.start_tick, gate);
                        }
                        _ => unreachable!(),
                    }
                    before.push((*n, k));
                    after.push((new, k));
                    *n = new;
                    any = true;
                },
            );
            if touched {
                model.mark_dirty(k);
            }
        }
        if !any {
            return None;
        }
        model.rebuild_dirty();
        self.data.bump_revision();

        // 选框跟随：gate 加减 uniform → 选框右边缘 te 同步平移（左边缘不动）
        if let Some(d) = uniform_delta {
            self.edit.offset_sel_te(d);
        }
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }

    /// Key/Tick：remove + 变换 + 重插。加减 uniform 时选框跟随平移。
    fn edit_note_positions(&mut self, field: NoteField, ops: &[NumOp]) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        if originals.is_empty() {
            return None;
        }

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        let mut uniform_tick: Option<i64> = None;
        let mut uniform_key: Option<i32> = None;
        let mut changed = false;
        for (note, old_key) in &originals {
            match field {
                NoteField::Key => {
                    let new_key = apply_ops_round(ops, *old_key as f64)
                        .clamp(0.0, yinhe_types::MAX_KEY as f64)
                        as u8;
                    if new_key != *old_key {
                        changed = true;
                        let d = new_key as i32 - *old_key as i32;
                        match uniform_key {
                            None => uniform_key = Some(d),
                            Some(u) if u != d => uniform_key = None,
                            _ => {}
                        }
                    }
                    new_by_key.entry(new_key).or_default().push(*note);
                }
                NoteField::Tick => {
                    let new_start = apply_ops_round(ops, note.start_tick as f64)
                        .clamp(0.0, u32::MAX as f64) as u32;
                    let len = note.end_tick - note.start_tick;
                    let new_note = yinhe_types::Note {
                        start_tick: new_start,
                        end_tick: new_start + len,
                        ..*note
                    };
                    if new_start != note.start_tick {
                        changed = true;
                        let d = new_start as i64 - note.start_tick as i64;
                        match uniform_tick {
                            None => uniform_tick = Some(d),
                            Some(u) if u != d => uniform_tick = None,
                            _ => {}
                        }
                    }
                    new_by_key.entry(*old_key).or_default().push(new_note);
                }
                _ => unreachable!(),
            }
        }
        if !changed {
            // 原样插回，模型内容不变（无 undo 动作）
            batch_ops::insert_batch(model, new_by_key);
            model.rebuild_dirty();
            return None;
        }

        let after = batch_ops::flatten_by_key(&new_by_key);
        batch_ops::insert_batch(model, new_by_key);
        model.rebuild_dirty();
        self.data.bump_revision();

        // 选框跟随：加减 uniform 时平移，乘除/赋值（非 uniform）不动
        if let Some(dt) = uniform_tick {
            self.edit.offset_sel_ticks(dt);
        }
        if let Some(dk) = uniform_key {
            self.edit.offset_sel_keys(dk);
        }
        Some(UndoAction::Notes(NoteDelta {
            before: originals,
            after,
        }))
    }
}
