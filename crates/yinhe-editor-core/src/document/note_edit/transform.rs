//! 变换：移调、水平/垂直翻转、相对选框跨度的变速（等比缩放）。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

use super::FlipAxis;

impl Document {
    /// Transpose all selected notes by `semitones`. Returns an `UndoAction` if any notes were transposed.
    ///
    /// 几何平移类：默认操作式 undo（`MoveNotes`，O(1) 内存）；
    /// 若任何音符触发 key 边界 clamp（越界回不来），回退副本制 `Notes`。
    pub fn transpose_selected(&mut self, semitones: i8) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        // 操作式 undo 需要操作**前**的选区矩形（offset_sel_keys 之前捕获）。
        let selection_before = self.edit.selected.clone();
        let (before, after, has_dest_overlap) = {
            let model = Arc::make_mut(&mut self.data.model);

            let moved_data = batch_ops::remove_selected(model, &self.edit.selected);
            if moved_data.is_empty() {
                return None;
            }

            // 检测目标位置是否已有非选中音符（操作集合已移除）：若目标选框内
            // 已有音符，操作式 undo 会误搬 B，需回退副本制。
            let dest_sel =
                selection_before.dest_overlap_selection(0, semitones as i32, 0, i32::MAX);
            let has_dest_overlap = batch_ops::compute_dest_overlap(model, &dest_sel, None);

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, old_key) in &moved_data {
                let new_key = ((*old_key as i16) + (semitones as i16))
                    .clamp(0, yinhe_types::MAX_KEY as i16) as u8;
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
            let after = batch_ops::flatten_by_key(&new_by_key);

            batch_ops::insert_batch(model, new_by_key);

            // Offset selection rects to follow the transposed notes.
            self.edit.offset_sel_keys(semitones as i32);
            (moved_data, after, has_dest_overlap)
        };
        self.data.rebuild_model_dirty();
        // 操作式 undo 前提：无音符触发 key clamp（undo 反向移动回原 key，
        // 原 key ∈ [0,MAX_KEY]，不触发新 clamp，对称性成立），且目标无重叠。
        let clamp_free = before.iter().all(|(_, k)| {
            (*k as i16 + semitones as i16) >= 0
                && (*k as i16 + semitones as i16) <= yinhe_types::MAX_KEY as i16
        });
        if clamp_free && !has_dest_overlap {
            Some(UndoAction::MoveNotes {
                selection: selection_before,
                delta_ticks: 0,
                delta_keys: semitones as i32,
            })
        } else {
            Some(UndoAction::Notes(NoteDelta { before, after }))
        }
    }

    /// 变速：把选框整体时间跨度（min ts .. max te）缩放为 `new_span` tick。
    ///
    /// 选中音符相对跨度起点等比缩放（可 undo），`selected` / `sel_rect` /
    /// `arr_sel_rect` 的 tick 范围同步缩放（key/track 不动）。
    /// 返回 `None` 表示无变化。
    pub fn rescale_selection_span(&mut self, new_span: u64) -> Option<UndoAction> {
        if self.edit.selected.is_empty() || new_span == 0 {
            return None;
        }
        let mut t0 = u64::MAX;
        let mut t1 = 0u64;
        for &(ts, te, _, _, _, _) in &self.edit.selected.rects {
            t0 = t0.min(ts as u64);
            t1 = t1.max(te as u64);
        }
        let span = t1 - t0;
        if span == 0 || new_span == span {
            return None; // 跨度相同：无操作
        }
        let factor = new_span as f64 / span as f64;
        let scale_tick = |v: u64| -> u64 {
            let s = (t0 as f64 + (v as f64 - t0 as f64) * factor)
                .round()
                .max(t0 as f64);
            if s > u32::MAX as f64 {
                u32::MAX as u64
            } else {
                s as u64
            }
        };

        let model = Arc::make_mut(&mut self.data.model);
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        if originals.is_empty() {
            return None;
        }

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        let mut changed = false;
        for (note, key) in &originals {
            let new_start = scale_tick(note.start_tick as u64) as u32;
            let new_end = scale_tick(note.end_tick as u64).max(new_start as u64 + 1) as u32;
            if new_start != note.start_tick || new_end != note.end_tick {
                changed = true;
            }
            new_by_key.entry(*key).or_default().push(yinhe_types::Note {
                start_tick: new_start,
                end_tick: new_end,
                ..*note
            });
        }
        if !changed {
            batch_ops::insert_batch(model, new_by_key);
            model.rebuild_dirty();
            return None; // remove 后原样插回，模型内容不变
        }

        let after = batch_ops::flatten_by_key(&new_by_key);
        batch_ops::insert_batch(model, new_by_key);
        model.rebuild_dirty();

        // 选框 rect 同步缩放（tick 范围，key/track 不动）
        self.edit.scale_sel_ticks(t0, factor);

        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: originals,
            after,
        }))
    }

    /// 翻转选中音符：水平（按 tick 镜像）或垂直（按 key 镜像）。
    ///
    /// 镜像范围 = 选框整体范围（min..max）；翻转后选框范围不变。
    /// 返回 `None` 表示没有选中音符。
    pub fn flip_selected_notes(&mut self, axis: FlipAxis) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        // 镜像范围 = 选框整体范围
        let mut t0 = u64::MAX;
        let mut t1 = 0u64;
        let mut kl = u8::MAX;
        let mut kh = 0u8;
        for &(ts, te, kl_, kh_, _, _) in &self.edit.selected.rects {
            t0 = t0.min(ts as u64);
            t1 = t1.max(te as u64);
            kl = kl.min(kl_);
            kh = kh.max(kh_);
        }

        let model = Arc::make_mut(&mut self.data.model);
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        if originals.is_empty() {
            return None;
        }

        let mirror_tick = |v: u32| -> u32 { (t0 as i64 + (t1 as i64 - v as i64)).max(0) as u32 };
        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        for (note, old_key) in &originals {
            match axis {
                FlipAxis::Horizontal => {
                    let new_start = mirror_tick(note.end_tick);
                    let new_end = mirror_tick(note.start_tick).max(new_start + 1);
                    new_by_key
                        .entry(*old_key)
                        .or_default()
                        .push(yinhe_types::Note {
                            start_tick: new_start,
                            end_tick: new_end,
                            ..*note
                        });
                }
                FlipAxis::Vertical => {
                    let new_key = (kl as i32 + kh as i32 - *old_key as i32)
                        .clamp(0, yinhe_types::MAX_KEY as i32)
                        as u8;
                    new_by_key.entry(new_key).or_default().push(*note);
                }
            }
        }

        let after = batch_ops::flatten_by_key(&new_by_key);
        // 检测选区内是否还有非选中音符（操作集合已移除）：若目标选框内已有
        // 音符，操作式 Flip 在撤销时会把这些 B 音符也一起翻转，造成误搬。
        let dest_sel = self.edit.selected.dest_overlap_selection(0, 0, 0, i32::MAX);
        let has_dest_overlap = batch_ops::compute_dest_overlap(model, &dest_sel, None);
        batch_ops::insert_batch(model, new_by_key);
        model.rebuild_dirty();
        self.data.bump_revision();

        // 操作式 undo 前提：水平镜像时所有音符的 end_tick 都 <= t1
        // （跨出选框的 end 镜像后触发 clamp 0，两次镜像不再恒等），
        // 且目标选框内无重叠非选中音符（否则撤销会误搬 B）。
        // 垂直镜像 key ∈ [kl, kh] 恒对称，无此问题。
        let flip_safe = axis == FlipAxis::Vertical
            || originals
                .iter()
                .all(|(n, _)| (n.end_tick as u64) <= t1 && (n.start_tick as u64) >= t0);
        if flip_safe && !has_dest_overlap {
            Some(UndoAction::FlipNotes {
                selection: self.edit.selected.clone(),
                bounds: (t0, t1, kl, kh),
                axis,
            })
        } else {
            Some(UndoAction::Notes(NoteDelta {
                before: originals,
                after,
            }))
        }
    }

    /// 变速：把选中音符的整体时间跨度按 `num/den` 缩放
    /// （`new_span = 当前跨度 × num / den`）。等价于
    /// [`Self::rescale_selection_span`]，供「快速变速」按钮复用。
    pub fn scale_selected_span(&mut self, num: u64, den: u64) -> Option<UndoAction> {
        if self.edit.selected.is_empty() || den == 0 {
            return None;
        }
        let (mut t0, mut t1) = (u64::MAX, 0u64);
        for &(ts, te, _, _, _, _) in &self.edit.selected.rects {
            t0 = t0.min(ts as u64);
            t1 = t1.max(te as u64);
        }
        if t1 <= t0 {
            return None;
        }
        let new_span = ((t1 - t0).saturating_mul(num) / den).max(1);
        self.rescale_selection_span(new_span)
    }
}
