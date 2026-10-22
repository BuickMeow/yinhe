//! 复制：原地复制（源保留）与 Alt 拖动复制（选区跟随副本）。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

impl Document {
    /// Duplicate all selected notes. Returns an `UndoAction` if any notes were duplicated.
    pub fn duplicate_selected(&mut self) -> Option<UndoAction> {
        self.duplicate_impl(0, 0, true)
    }

    /// Duplicate selected notes and offset the copies by `(delta_ticks, delta_keys)`.
    ///
    /// 原音符保留不动，副本平移到目标位置；选区同步移到副本范围，便于连续操作。
    /// 用于 Alt+拖动复制：一步操作，一个 undo entry。
    pub fn duplicate_selected_to(
        &mut self,
        delta_ticks: i64,
        delta_keys: i32,
    ) -> Option<UndoAction> {
        self.duplicate_impl(delta_ticks, delta_keys, false)
    }

    /// 按选区宽度重复：`steps` 为正向右、为负向左；选区本身保持不变。
    ///
    /// 一次流式扫描选区并在同一批次插入所有副本，避免对每个重复段重新扫描
    /// 全选区；返回一个合并的 NoteDelta，只占一步 undo。
    pub fn repeat_selected(&mut self, width_ticks: i64, steps: i64) -> Option<UndoAction> {
        if width_ticks <= 0 || steps == 0 || self.edit.selected.is_empty() {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let after = {
            let model = &self.data.model;
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            let sign = if steps > 0 { 1 } else { -1 };
            batch_ops::for_each_selected(model, &self.edit.selected, |note, old_key| {
                for step in 1..=steps.unsigned_abs() as i64 {
                    let delta = sign * width_ticks * step;
                    let new_start = (note.start_tick as i64 + delta).max(0) as u32;
                    let length = note.end_tick - note.start_tick;
                    if !allow_overlap
                        && batch_ops::has_overlapping_note(
                            model,
                            note.track,
                            old_key,
                            new_start,
                            new_start + length,
                        )
                    {
                        continue;
                    }
                    new_by_key
                        .entry(old_key)
                        .or_default()
                        .push(yinhe_types::Note {
                            id: 0,
                            start_tick: new_start,
                            end_tick: new_start + length,
                            velocity: note.velocity,
                            track: note.track,
                        });
                }
            });
            if new_by_key.is_empty() {
                return None;
            }
            let model = Arc::make_mut(&mut self.data.model);
            let mut after = Vec::new();
            for (key, notes) in new_by_key.iter_mut() {
                for n in notes.iter_mut() {
                    n.id = model.alloc_note_id();
                    after.push((*n, *key));
                }
            }
            batch_ops::insert_batch(model, new_by_key);
            after
        };
        self.data.rebuild_model_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// 复制的公共实现（`duplicate_selected` / `duplicate_selected_to`）。
    ///
    /// 流式两遍（不物化 `collect_selected`）：`auto_span` 时先求选区跨度作偏移，
    /// 再逐音符构建副本批次。1.64 亿全选下省去一份 ~3GB 中间 `Vec`。
    /// 「允许新重叠音符」关闭时，与操作前已有音符重叠的副本逐个跳过；
    /// 全部被跳过则不动选区、不产生 undo。
    fn duplicate_impl(
        &mut self,
        delta_ticks: i64,
        delta_keys: i32,
        auto_span: bool,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let after = {
            let model = &self.data.model;

            // 自动偏移（duplicate_selected）：取选区跨度（流式，不物化）。
            let (delta_ticks, delta_keys) = if auto_span {
                let mut min_start = u32::MAX;
                let mut max_end = 0u32;
                batch_ops::for_each_selected(model, &self.edit.selected, |n, _| {
                    min_start = min_start.min(n.start_tick);
                    max_end = max_end.max(n.end_tick);
                });
                if min_start == u32::MAX {
                    return None;
                }
                ((((max_end - min_start).max(1)) as i64), 0)
            } else {
                (delta_ticks, delta_keys)
            };

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            batch_ops::for_each_selected(model, &self.edit.selected, |note, old_key| {
                let new_key =
                    ((old_key as i32) + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                let new_start = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                let length = note.end_tick - note.start_tick;
                // 「允许新重叠音符」关闭：副本与操作前已有音符重叠 → 跳过该副本。
                // 检查在批量插入前进行，批次内部互不影响（含各副本的原音符）。
                if !allow_overlap
                    && batch_ops::has_overlapping_note(
                        model,
                        note.track,
                        new_key,
                        new_start,
                        new_start + length,
                    )
                {
                    return;
                }
                // id 延后到插入前统一发号（流式阶段借用 &model，无法 alloc）。
                new_by_key
                    .entry(new_key)
                    .or_default()
                    .push(yinhe_types::Note {
                        id: 0,
                        start_tick: new_start,
                        end_tick: new_start + length,
                        velocity: note.velocity,
                        track: note.track,
                    });
            });

            // 全部被跳过：不动选区、不产生 undo。
            if new_by_key.is_empty() {
                return None;
            }

            let model = Arc::make_mut(&mut self.data.model);
            let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
            for (key, notes) in new_by_key.iter_mut() {
                for n in notes.iter_mut() {
                    n.id = model.alloc_note_id();
                    after.push((*n, *key));
                }
            }
            batch_ops::insert_batch(model, new_by_key);

            // 选区跟随副本（新 id 精确跟随，落点处的其他音符不纳入）。
            if auto_span {
                self.edit.offset_sel_ticks(delta_ticks);
            } else {
                self.edit.selected.offset(delta_ticks, delta_keys);
            }
            let mut copy_ids = yinhe_types::NoteBitsetBuilder::new();
            for (n, _) in &after {
                copy_ids.insert(n.id);
            }
            self.edit.selected.set_members_bitset(copy_ids.build());
            after
        };
        self.data.rebuild_model_dirty();
        // 复制会改变音符数据但 per-key revision 的变化不足以让 GPU 层缓存
        // 失效（gpu_upload 先查 data.revision），必须同步 bump 文档 revision。
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }
}
