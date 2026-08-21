//! Note editing operations: add, delete, duplicate, transpose, move, resize.
//!
//! 单音符操作（pencil drag, velocity）在 `note_pencil.rs`。

use std::sync::Arc;

use yinhe_core::NoteEvent;

use crate::batch_ops;
use crate::edit_state::ResizeSide;
use crate::history::{NoteDelta, UndoAction};
use crate::num_expr::{NumOp, apply_ops_round};

use super::Document;

/// 批量编辑的字段（Info 面板选框编辑）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteField {
    /// 力度（0-127）。
    Velocity,
    /// 音符长度 gate（end - start，tick）。
    Gate,
    /// 琴键（0-MAX_KEY）。
    Key,
    /// 起始 tick。
    Tick,
}

/// 翻转方向（选中音符镜像）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipAxis {
    /// 水平翻转：按选框整体 tick 范围镜像（start/end 互换镜像，gate 不变）。
    Horizontal,
    /// 垂直翻转：按选框整体 key 范围镜像。
    Vertical,
}

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
        {
            let model = Arc::make_mut(&mut self.data.model);
            let k = key as usize;
            Arc::make_mut(&mut model.notes[k]).insert_sorted(typed_note);
            model.mark_dirty(key);
        }
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
        let model = Arc::make_mut(&mut self.data.model);
        let bucket = Arc::make_mut(&mut model.notes[key as usize]);
        let orig = *bucket.find_mut(note_id)?;
        let new_end = end_tick.max(orig.start_tick + 1);
        if orig.end_tick == new_end {
            return None;
        }
        let mut new_note = orig;
        new_note.end_tick = new_end;
        *bucket.find_mut(note_id)? = new_note;
        model.mark_dirty(key);
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
        let removed = Arc::make_mut(&mut model.notes[key as usize]).remove_by_id(note.id)?;
        model.mark_dirty(key);
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![(removed, key)],
            after: vec![],
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
        let allow_overlap = self.edit.allow_overlapping_notes;
        let after = {
            let model = Arc::make_mut(&mut self.data.model);

            let selected_data = batch_ops::collect_selected(model, &self.edit.selected);
            if selected_data.is_empty() {
                return None;
            }

            let min_start = selected_data
                .iter()
                .map(|(n, _)| n.start_tick)
                .min()
                .unwrap();
            let max_end = selected_data.iter().map(|(n, _)| n.end_tick).max().unwrap();
            let offset = (max_end - min_start).max(1);

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, key) in &selected_data {
                let new_start = note.start_tick + offset;
                let new_end = note.end_tick + offset;
                // 「允许新重叠音符」关闭：副本与操作前已有音符重叠 → 跳过该副本。
                // 检查在批量插入前进行，批次内部互不影响（含各副本的原音符）。
                if !allow_overlap
                    && batch_ops::has_overlapping_note(model, note.track, *key, new_start, new_end)
                {
                    continue;
                }
                let new_note = yinhe_types::Note {
                    id: model.alloc_note_id(),
                    start_tick: new_start,
                    end_tick: new_end,
                    velocity: note.velocity,
                    track: note.track,
                };
                new_by_key.entry(*key).or_default().push(new_note);
            }

            // 全部被跳过：不动选区、不产生 undo。
            if new_by_key.is_empty() {
                return None;
            }

            // Build after vec before moving new_by_key.
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

            batch_ops::insert_batch(model, new_by_key);

            // Offset selection rects to cover the duplicated notes.
            self.edit.offset_sel_ticks(offset as i64);
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

    /// Duplicate selected notes and offset the copies by `(delta_ticks, delta_keys)`.
    ///
    /// 原音符保留不动，副本平移到目标位置；选区同步移到副本范围，便于连续操作。
    /// 用于 Alt+拖动复制：一步操作，一个 undo entry。
    pub fn duplicate_selected_to(
        &mut self,
        delta_ticks: i64,
        delta_keys: i32,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let after = {
            let model = Arc::make_mut(&mut self.data.model);

            let selected_data = batch_ops::collect_selected(model, &self.edit.selected);
            if selected_data.is_empty() {
                return None;
            }

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, old_key) in &selected_data {
                let new_key =
                    ((*old_key as i32) + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
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
                    continue;
                }
                let new_note = yinhe_types::Note {
                    id: model.alloc_note_id(),
                    start_tick: new_start,
                    end_tick: new_start + length,
                    velocity: note.velocity,
                    track: note.track,
                };
                new_by_key.entry(new_key).or_default().push(new_note);
            }

            // 全部被跳过：不动选区、不产生 undo。
            if new_by_key.is_empty() {
                return None;
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
        // Bug 修复：缺少 bump_revision 导致框选 Alt+拖动复制后 GPU 层缓存
        // 不失效（note_key 不变），画面不更新。与 move_selected_notes 保持一致。
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// Transpose all selected notes by `semitones`. Returns an `UndoAction` if any notes were transposed.
    ///
    /// 几何平移类：默认操作式 undo（`MoveNotes`，O(1) 内存）；
    /// 若任何音符触发 key 边界 clamp（越界回不来），回退副本制 `Notes`。
    pub fn transpose_selected(&mut self, semitones: i8) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        // 操作式 undo 需要操作**前**的选区矩形（offset_sel_keys 之前捕获）。
        let rects_before = self.edit.selected.rects.clone();
        let (before, after, has_dest_overlap) = {
            let model = Arc::make_mut(&mut self.data.model);

            let moved_data = batch_ops::remove_selected(model, &self.edit.selected);
            if moved_data.is_empty() {
                return None;
            }

            // 检测目标位置是否已有非选中音符：若目标选框内已有音符，
            // 操作式 undo 会误搬 B，需回退副本制。
            let has_dest_overlap = {
                let dest_rects: Vec<(u32, u32, u8, u8, u16, u16)> = rects_before
                    .iter()
                    .map(|&(ts, te, kl, kh, tl, th)| {
                        (
                            ts,
                            te,
                            (kl as i32 + semitones as i32).clamp(0, yinhe_types::MAX_KEY as i32)
                                as u8,
                            (kh as i32 + semitones as i32).clamp(0, yinhe_types::MAX_KEY as i32)
                                as u8,
                            tl,
                            th,
                        )
                    })
                    .collect();
                let dest_sel = yinhe_core::Selection { rects: dest_rects };
                !batch_ops::collect_selected(model, &dest_sel).is_empty()
            };

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
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

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
                rects: rects_before,
                delta_ticks: 0,
                delta_keys: semitones as i32,
            })
        } else {
            Some(UndoAction::Notes(NoteDelta { before, after }))
        }
    }

    /// Move all selected notes by (delta_ticks, delta_keys).
    ///
    /// Returns an `UndoAction` if any notes were moved. The caller is
    /// responsible for pushing it to the history stack, marking the view
    /// dirty, and sending `AudioCommand::ReloadNotes`.
    ///
    /// 几何平移类：默认操作式 undo（`MoveNotes`，O(1) 内存）；
    /// 若任何音符触发 tick/key 边界 clamp（越界回不来），回退副本制 `Notes`。
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
        // 操作式 undo 需要操作**前**的选区矩形（offset 之前捕获）。
        let rects_before = self.edit.selected.rects.clone();
        // 检测目标位置是否已有非选中音符（重叠允许时也会产生）：
        // 若目标选框内已有音符，操作式 undo（按选框收集）在撤销时会把
        // 这些原本不在选区的 B 音符也一起平移回 A，造成“B 被搬到 A”的 bug。
        // 此时必须回退到副本制（NoteDelta），仅精确撤销被移动的音符。
        // 必须在插入新音符前检测（此时模型仅含 B 等非选中音符）。
        let has_dest_overlap = {
            let dest_rects: Vec<(u32, u32, u8, u8, u16, u16)> = rects_before
                .iter()
                .map(|&(ts, te, kl, kh, tl, th)| {
                    (
                        (ts as i64 + delta_ticks).max(0) as u32,
                        (te as i64 + delta_ticks).max(0) as u32,
                        (kl as i32 + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8,
                        (kh as i32 + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8,
                        tl,
                        th,
                    )
                })
                .collect();
            let dest_sel = yinhe_core::Selection { rects: dest_rects };
            !batch_ops::collect_selected(model, &dest_sel).is_empty()
        };
        let allow_overlap = self.edit.allow_overlapping_notes;
        let behavior = self.edit.overlap_blocked_behavior;
        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        // 被「禁止重叠」拦下的音符按 behavior 处理：
        // - KeepOriginal：留在原位（当前逻辑）
        // - DeleteOriginal：删除原音符（原消失，目标保留）
        // - ReplaceTarget：删除目标重叠音符并移动（原消失，目标被覆盖）
        let mut moved_before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut moved_after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut deleted_before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut replaced_before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut blocked_any = false;
        for (note, old_key) in &originals {
            let new_key =
                ((*old_key as i32) + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
            let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
            let length = note.end_tick - note.start_tick;
            // 「允许新重叠音符」关闭：目标位置与非本次移动的已有音符重叠
            // （移动集合已先移除，检查看不到它们）→ 按 behavior 处理。
            if !allow_overlap
                && batch_ops::has_overlapping_note(
                    model,
                    note.track,
                    new_key,
                    new_tick,
                    new_tick + length,
                )
            {
                match behavior {
                    crate::audio_settings::OverlapBlockedBehavior::KeepOriginal => {
                        blocked_any = true;
                        new_by_key.entry(*old_key).or_default().push(*note);
                    }
                    crate::audio_settings::OverlapBlockedBehavior::DeleteOriginal => {
                        // 删除原音符，不移动也不替换目标
                        deleted_before.push((*note, *old_key));
                        // 不插入任何位置，原音符消失
                    }
                    crate::audio_settings::OverlapBlockedBehavior::ReplaceTarget => {
                        // 删除目标处重叠音符，再移动
                        let lo = new_tick.saturating_sub(model.max_note_len);
                        let overlapping: Vec<yinhe_types::Note> = model.notes[new_key as usize]
                            .range(lo, new_tick + length)
                            .filter(|n| n.track == note.track && n.end_tick > new_tick)
                            .cloned()
                            .collect();
                        if !overlapping.is_empty() {
                            let ids: std::collections::HashSet<u32> =
                                overlapping.iter().map(|n| n.id).collect();
                            let bucket = Arc::make_mut(&mut model.notes[new_key as usize]);
                            bucket.remove_by_ids(&ids);
                            model.mark_dirty(new_key);
                            for t in overlapping {
                                replaced_before.push((t, new_key));
                            }
                        }
                        let moved = yinhe_types::Note {
                            id: note.id,
                            start_tick: new_tick,
                            end_tick: new_tick + length,
                            velocity: note.velocity,
                            track: note.track,
                        };
                        moved_before.push((*note, *old_key));
                        moved_after.push((moved, new_key));
                        new_by_key.entry(new_key).or_default().push(moved);
                    }
                }
                continue;
            }
            let moved = yinhe_types::Note {
                id: note.id,
                start_tick: new_tick,
                end_tick: new_tick + length,
                velocity: note.velocity,
                track: note.track,
            };
            moved_before.push((*note, *old_key));
            moved_after.push((moved, new_key));
            new_by_key.entry(new_key).or_default().push(moved);
        }
        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
        batch_ops::insert_batch(model, new_by_key);

        // 全部被拦且无其他变化（KeepOriginal 全拦）：音符已原样插回，选区不动，不产生 undo。
        // DeleteOriginal / ReplaceTarget 全拦时有删除/替换，需产生 delta。
        if blocked_any
            && moved_before.is_empty()
            && deleted_before.is_empty()
            && replaced_before.is_empty()
        {
            model.rebuild_dirty();
            return None;
        }

        // Offset selection rects to follow the moved notes.
        // 部分被拦时选区仍整体跟随移动手势（留在原处的音符可能脱出选区）。
        self.edit.selected.offset(delta_ticks, delta_keys);
        model.rebuild_dirty();
        self.data.bump_revision();

        // 操作式 undo 前提：无音符触发 tick/key clamp（undo 反向移动回
        // 原位置，原位置不触发新 clamp，对称性成立），且目标位置无重叠
        // 非选中音符（否则撤销会误搬 B），且无按 behavior 处理的阻拦/替换。
        let clamp_free = originals.iter().all(|(n, k)| {
            (n.start_tick as i64 + delta_ticks) >= 0
                && (*k as i32 + delta_keys) >= 0
                && (*k as i32 + delta_keys) <= yinhe_types::MAX_KEY as i32
        });
        let has_blocked_handling =
            blocked_any || !deleted_before.is_empty() || !replaced_before.is_empty();
        if clamp_free && !has_blocked_handling && !has_dest_overlap {
            Some(UndoAction::MoveNotes {
                rects: rects_before,
                delta_ticks,
                delta_keys,
            })
        } else if blocked_any
            && matches!(
                behavior,
                crate::audio_settings::OverlapBlockedBehavior::KeepOriginal
            )
        {
            // 退回：仅真正移动的音符进 delta
            Some(UndoAction::Notes(NoteDelta {
                before: moved_before,
                after: moved_after,
            }))
        } else {
            // 删除原位 / 替换目标 或目标有重叠（allow=true）：用完整 before/after
            // before 需包含被替换的目标音符（B），after 仅含新位置的移动音符
            let mut before_all = originals.clone();
            before_all.extend(replaced_before.clone());
            // deleted_before 已在 originals 中，无需额外；replaced_before 需追加
            // after 已由 new_by_key 构建（仅含移动后的音符）
            Some(UndoAction::Notes(NoteDelta {
                before: before_all,
                after,
            }))
        }
    }

    /// Resize all selected notes by shifting one edge (Left/Right) by `dt` ticks.
    ///
    /// 选框工具边缘拖动：对所有选中音符的 `start_tick`（Left）或 `end_tick`（Right）
    /// 统一偏移 `dt`。每个音符独立 clamp，保证 `end_tick > start_tick`。
    /// 选框 (`sel_rect`) 的更新由 UI 层负责（与 move 一致）。
    pub fn resize_selected_notes(&mut self, side: ResizeSide, dt: i64) -> Option<UndoAction> {
        if self.edit.selected.is_empty() || dt == 0 {
            return None;
        }

        let model = Arc::make_mut(&mut self.data.model);
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        let allow_overlap = self.edit.allow_overlapping_notes;

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        let mut moved_before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut moved_after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut blocked_any = false;
        for (note, old_key) in &originals {
            let new_note = match side {
                ResizeSide::Left => {
                    // start_tick += dt，clamp 到 [0, end_tick - 1]
                    let new_start = (note.start_tick as i64 + dt)
                        .max(0)
                        .min(note.end_tick as i64 - 1) as u32;
                    yinhe_types::Note {
                        start_tick: new_start,
                        ..*note
                    }
                }
                ResizeSide::Right => {
                    // end_tick += dt，clamp 到 [start_tick + 1, u32::MAX]
                    let new_end =
                        (note.end_tick as i64 + dt).max(note.start_tick as i64 + 1) as u32;
                    yinhe_types::Note {
                        end_tick: new_end,
                        ..*note
                    }
                }
            };
            // 「允许新重叠音符」关闭：拉伸后与已有音符重叠 → 该音符保持原样。
            // （选中集合已移除，检查看不到它们，批次内部互不影响）
            if !allow_overlap
                && batch_ops::has_overlapping_note(
                    model,
                    note.track,
                    *old_key,
                    new_note.start_tick,
                    new_note.end_tick,
                )
            {
                blocked_any = true;
                new_by_key.entry(*old_key).or_default().push(*note);
                continue;
            }
            moved_before.push((*note, *old_key));
            moved_after.push((new_note, *old_key));
            new_by_key.entry(*old_key).or_default().push(new_note);
        }

        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
        batch_ops::insert_batch(model, new_by_key);

        // 全部被拦：原样插回，选区不动，不产生 undo。
        if blocked_any && moved_before.is_empty() {
            model.rebuild_dirty();
            return None;
        }

        // 同步 Selection 的 tick 范围（用于后续操作的命中判定）。
        // Selection::offset 会同时改 ts 和 te，但 resize 只想改其中一个，手动处理。
        // 部分被拦时选区仍整体跟随手势（留在原处的音符可能脱出选区）。
        match side {
            ResizeSide::Left => {
                for r in &mut self.edit.selected.rects {
                    let new_ts = (r.0 as i64 + dt).max(0) as u32;
                    if new_ts < r.1 {
                        r.0 = new_ts;
                    }
                }
            }
            ResizeSide::Right => {
                for r in &mut self.edit.selected.rects {
                    let new_te = (r.1 as i64 + dt).max(r.0 as i64 + 1) as u32;
                    r.1 = new_te;
                }
            }
        }

        model.rebuild_dirty();
        self.data.bump_revision();

        if blocked_any {
            Some(UndoAction::Notes(NoteDelta {
                before: moved_before,
                after: moved_after,
            }))
        } else {
            Some(UndoAction::Notes(NoteDelta {
                before: originals,
                after,
            }))
        }
    }

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
    fn edit_note_props(&mut self, field: NoteField, ops: &[NumOp]) -> Option<UndoAction> {
        struct Target {
            key: u8,
            id: u32,
            old: yinhe_types::Note,
            new: yinhe_types::Note,
        }
        let mut targets: Vec<Target> = Vec::new();
        let mut uniform_delta: Option<i64> = None; // 全部变化项相同 delta 时 Some（gate 加减）
        {
            let model = &self.data.model;
            for &(ts, te, kl, kh, tl, th) in &self.edit.selected.rects {
                for key in kl..=kh {
                    let k = key as usize;
                    for n in model.notes[k].range(ts, te) {
                        if n.track < tl || n.track > th {
                            continue;
                        }
                        let new = match field {
                            NoteField::Velocity => {
                                let v =
                                    apply_ops_round(ops, n.velocity as f64).clamp(0.0, 127.0) as u8;
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
                        if changed {
                            if field == NoteField::Gate {
                                let d = new.end_tick as i64 - n.end_tick as i64;
                                match uniform_delta {
                                    None => uniform_delta = Some(d),
                                    Some(u) if u != d => uniform_delta = None,
                                    _ => {}
                                }
                            }
                            targets.push(Target {
                                key,
                                id: n.id,
                                old: *n,
                                new,
                            });
                        }
                    }
                }
            }
        }
        if targets.is_empty() {
            return None;
        }

        if field == NoteField::Velocity {
            // 记录"最近修改力度"：新音符默认力度跟随最近一次修改（同轨取时间最晚的音符）。
            for t in &targets {
                self.edit
                    .remember_velocity(t.old.track, t.old.start_tick, t.new.velocity);
            }
        }

        let model = Arc::make_mut(&mut self.data.model);
        let mut before = Vec::with_capacity(targets.len());
        let mut after = Vec::with_capacity(targets.len());
        for t in &targets {
            let bucket = Arc::make_mut(&mut model.notes[t.key as usize]);
            // 收集阶段与修改阶段之间无并发修改，目标必然存在。
            let n = bucket.find_mut(t.id).expect("edit target vanished");
            *n = t.new;
            before.push((t.old, t.key));
            after.push((t.new, t.key));
            model.mark_dirty(t.key);
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

        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
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

        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
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

        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
        // 检测选区内是否还有非选中音符：若目标选框内已有音符，
        // 操作式 Flip 在撤销时会把这些 B 音符也一起翻转，造成误搬。
        let has_dest_overlap = {
            let sel = yinhe_core::Selection {
                rects: self.edit.selected.rects.clone(),
            };
            !batch_ops::collect_selected(model, &sel).is_empty()
        };
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
                rects: self.edit.selected.rects.clone(),
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

    /// 一键为整首歌去重重叠音符（用于黑乐谱叠音清理）。
    ///
    /// - `cross_track == false`：仅同一 `(track, key)` 内 `[start,end)` 相交的视为重叠，保留最早的，删后者；
    /// - `cross_track == true`：同一 `key` 下跨轨也视为重叠（全局 per-key 去重）。
    ///
    /// 已按 `start_tick` 有序的桶直接线性扫描 `O(N)`，每 key 单独 `from_sorted` 重建，
    /// 空桶跳过。对 1 亿音符（128 key × ~78万）峰值临时 `Vec` 约 12MB。
    pub fn dedup_overlapping_notes(&mut self, cross_track: bool) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let mut removed: Vec<(yinhe_types::Note, u8)> = Vec::new();
        for key in 0u8..=yinhe_types::MAX_KEY {
            let bucket = Arc::make_mut(&mut model.notes[key as usize]);
            if bucket.is_empty() {
                continue;
            }
            let notes: Vec<yinhe_types::Note> = bucket.iter().copied().collect();
            let orig_len = notes.len();
            let mut kept: Vec<yinhe_types::Note> = Vec::with_capacity(orig_len);
            if cross_track {
                let mut last_end: Option<u32> = None;
                for n in notes {
                    if let Some(le) = last_end
                        && n.start_tick < le
                    {
                        removed.push((n, key));
                        continue;
                    }
                    kept.push(n);
                    last_end = Some(n.end_tick);
                }
            } else {
                use std::collections::HashMap;
                let mut last_per_track: HashMap<u16, u32> = HashMap::new();
                for n in notes {
                    if let Some(&le) = last_per_track.get(&n.track)
                        && n.start_tick < le
                    {
                        removed.push((n, key));
                        continue;
                    }
                    kept.push(n);
                    // 同轨下一个音符的起点必须 >= 本音符终点才保留
                    last_per_track.insert(n.track, n.end_tick);
                }
            }
            if kept.len() == orig_len {
                continue;
            }
            // 重建桶（kept 已有序，无需再排序）
            *bucket = yinhe_types::NoteBucket::from_sorted(kept);
            model.mark_dirty(key);
        }
        if removed.is_empty() {
            return None;
        }
        model.rebuild_dirty();
        self.data.bump_revision();
        // 清理选中：被删音符若在选区内，其 rect 已无对应音符，但几何选区本身保留，
        // 不自动缩选区（与删除选中音符的语义一致：选区清空由调用方或下次框选决定）。
        Some(UndoAction::Notes(NoteDelta {
            before: removed,
            after: vec![],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use yinhe_core::{ConductorData, TrackData, YinModel};
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
    fn duplicate_selected_to_preserves_original_and_offsets_copy() {
        let mut doc = make_doc_with_note();
        let action = doc
            .duplicate_selected_to(50, 12)
            .expect("should produce action");

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
        // key 60 + 200 半音 = 260, 应 clamp 到 MAX_KEY(255)
        let _ = doc.duplicate_selected_to(0, 200);
        assert_eq!(
            doc.data.model.notes[yinhe_types::MAX_KEY as usize].len(),
            1,
            "应 clamp 到 key MAX_KEY"
        );
    }

    /// Bug 7 回归：Alt 拖动复制后必须 bump 文档 revision，否则 GPU 层缓存不失效、画面不更新。
    #[test]
    fn duplicate_selected_to_bumps_revision() {
        let mut doc = make_doc_with_note();
        let rev_before = doc.data.revision;
        assert!(doc.duplicate_selected_to(50, 12).is_some(), "复制应成功");
        assert!(
            doc.data.revision > rev_before,
            "复制后文档 revision 必须前进（GPU 缓存失效依据）"
        );
        // per-key revision 也应前进（GPU cull 增量上传依据）
        assert!(
            doc.data.note_revisions()[72] > 0,
            "副本所在 key 的 revision 应前进"
        );
    }

    #[test]
    fn transpose_follows_visual_rects() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        let _ = doc.transpose_selected(2);
        // 数据选区 + PR 视觉选框的 key 一起平移
        assert_eq!(doc.edit.selected.rects[0].2, 62);
        assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 201.0, 62, 62));
    }

    #[test]
    fn duplicate_follows_visual_rects() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        let _ = doc.duplicate_selected();
        // 数据选区 + PR 视觉选框一起平移到副本（tick 偏移 100）
        assert_eq!(doc.edit.selected.rects[0].0, 200);
        assert_eq!(doc.edit.sel_rect.rects[0], (200.0, 301.0, 60, 60));
    }

    #[test]
    fn resize_selected_notes_right_extends_end_tick() {
        let mut doc = make_doc_with_note();
        // 原音符: tick 100~200, key 60
        let action = doc
            .resize_selected_notes(ResizeSide::Right, 50)
            .expect("应产生 UndoAction");
        let note = doc.data.model.notes[60][0];
        assert_eq!(note.start_tick, 100, "start_tick 不变");
        assert_eq!(note.end_tick, 250, "end_tick += 50");

        // 选区右边界同步偏移
        let (ts, te, _kl, _kh, _tl, _th) = doc.edit.selected.rects[0];
        assert_eq!(ts, 100, "选区 ts 不变");
        assert_eq!(te, 251, "选区 te += 50 (原 201)");

        // UndoAction
        match action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1);
                assert_eq!(delta.before[0].0.end_tick, 200);
                assert_eq!(delta.after.len(), 1);
                assert_eq!(delta.after[0].0.end_tick, 250);
            }
            _ => panic!("期望 UndoAction::Notes"),
        }
    }

    #[test]
    fn resize_selected_notes_left_shifts_start_tick() {
        let mut doc = make_doc_with_note();
        // 原音符: tick 100~200, key 60
        doc.resize_selected_notes(ResizeSide::Left, -30)
            .expect("应产生 UndoAction");
        let note = doc.data.model.notes[60][0];
        assert_eq!(note.start_tick, 70, "start_tick -= 30");
        assert_eq!(note.end_tick, 200, "end_tick 不变");

        // 选区左边界同步偏移
        let (ts, te, _kl, _kh, _tl, _th) = doc.edit.selected.rects[0];
        assert_eq!(ts, 70, "选区 ts -= 30");
        assert_eq!(te, 201, "选区 te 不变");
    }

    #[test]
    fn resize_selected_notes_right_clamps_to_min_length() {
        let mut doc = make_doc_with_note();
        // 原音符: tick 100~200 (长度 100)。dt = -200 会让 end < start，应 clamp 到 start+1
        doc.resize_selected_notes(ResizeSide::Right, -200)
            .expect("应产生 UndoAction");
        let note = doc.data.model.notes[60][0];
        assert_eq!(note.start_tick, 100);
        assert_eq!(note.end_tick, 101, "end_tick 应 clamp 到 start+1");
    }

    #[test]
    fn resize_selected_notes_left_clamps_to_min_length() {
        let mut doc = make_doc_with_note();
        // 原音符: tick 100~200。dt = 200 会让 start >= end，应 clamp 到 end-1
        doc.resize_selected_notes(ResizeSide::Left, 200)
            .expect("应产生 UndoAction");
        let note = doc.data.model.notes[60][0];
        assert_eq!(note.start_tick, 199, "start_tick 应 clamp 到 end-1");
        assert_eq!(note.end_tick, 200);
    }

    #[test]
    fn resize_selected_notes_zero_dt_returns_none() {
        let mut doc = make_doc_with_note();
        assert!(doc.resize_selected_notes(ResizeSide::Right, 0).is_none());
        assert!(doc.resize_selected_notes(ResizeSide::Left, 0).is_none());
    }

    #[test]
    fn resize_selected_notes_empty_selection_returns_none() {
        let mut doc = make_doc_with_note();
        doc.edit.selected.clear();
        assert!(doc.resize_selected_notes(ResizeSide::Right, 50).is_none());
    }

    #[test]
    fn apply_note_field_edit_velocity_add() {
        let mut doc = make_doc_with_note();
        let ops = crate::num_expr::parse_num_expr("+5").unwrap();
        let action = doc
            .apply_note_field_edit(NoteField::Velocity, &ops)
            .expect("should edit");
        assert_eq!(doc.data.model.notes[60][0].velocity, 105);
        match action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1);
                assert_eq!(delta.before[0].0.velocity, 100);
                assert_eq!(delta.after[0].0.velocity, 105);
            }
            _ => panic!("expected Notes"),
        }
    }

    #[test]
    fn apply_note_field_edit_velocity_clamp_127() {
        let mut doc = make_doc_with_note();
        let ops = crate::num_expr::parse_num_expr("x2").unwrap();
        doc.apply_note_field_edit(NoteField::Velocity, &ops);
        assert_eq!(doc.data.model.notes[60][0].velocity, 127);
    }

    #[test]
    fn apply_note_field_edit_gate_add_follows_rect() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        let ops = crate::num_expr::parse_num_expr("+10").unwrap();
        doc.apply_note_field_edit(NoteField::Gate, &ops);
        let n = doc.data.model.notes[60][0];
        assert_eq!(n.end_tick - n.start_tick, 110);
        // 选框 te 跟随 +10，ts 不动
        assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 211.0, 60, 60));
        assert_eq!(doc.edit.selected.rects[0].1, 211);
    }

    #[test]
    fn apply_note_field_edit_tick_add_moves_rects() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        doc.edit.arr_sel_rect = vec![(100.0, 201.0, 0, 0)];
        let ops = crate::num_expr::parse_num_expr("+50").unwrap();
        doc.apply_note_field_edit(NoteField::Tick, &ops);
        let n = doc.data.model.notes[60][0];
        assert_eq!(n.start_tick, 150);
        assert_eq!(n.end_tick, 250);
        // PR + AR 选框 tick 平移
        assert_eq!(doc.edit.sel_rect.rects[0], (150.0, 251.0, 60, 60));
        assert_eq!(doc.edit.arr_sel_rect[0], (150.0, 251.0, 0, 0));
        assert_eq!(doc.edit.selected.rects[0].0, 150);
    }

    #[test]
    fn apply_note_field_edit_key_add_moves_bucket_and_rect() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        let ops = crate::num_expr::parse_num_expr("+2").unwrap();
        doc.apply_note_field_edit(NoteField::Key, &ops);
        assert!(doc.data.model.notes[60].is_empty());
        assert_eq!(doc.data.model.notes[62].len(), 1);
        // 选框 key 平移
        assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 201.0, 62, 62));
        assert_eq!(doc.edit.selected.rects[0].2, 62);
    }

    #[test]
    fn apply_note_field_edit_tick_mul_follows_single_uniform() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        let ops = crate::num_expr::parse_num_expr("x2").unwrap();
        doc.apply_note_field_edit(NoteField::Tick, &ops);
        let n = doc.data.model.notes[60][0];
        assert_eq!(n.start_tick, 200);
        assert_eq!(n.end_tick, 300);
        // 单音符 delta 一致（+100）→ 选框跟随
        assert_eq!(doc.edit.sel_rect.rects[0], (200.0, 301.0, 60, 60));
    }

    #[test]
    fn apply_note_field_edit_empty_selection_returns_none() {
        let mut doc = make_doc_with_note();
        doc.edit.selected.clear();
        let ops = crate::num_expr::parse_num_expr("+10").unwrap();
        assert!(
            doc.apply_note_field_edit(NoteField::Velocity, &ops)
                .is_none()
        );
    }

    #[test]
    fn rescale_selection_span_doubles_notes_and_rects() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        // 跨度 101 → 202（×2）：音符 start 100→100（起点不动），end 200→300
        let action = doc.rescale_selection_span(202).expect("should edit");
        let n = doc.data.model.notes[60][0];
        assert_eq!(n.start_tick, 100);
        assert_eq!(n.end_tick, 300);
        assert_eq!(doc.edit.selected.rects[0].1, 302);
        assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 302.0, 60, 60));
        match action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before[0].0.end_tick, 200);
                assert_eq!(delta.after[0].0.end_tick, 300);
            }
            _ => panic!("expected Notes"),
        }
    }

    #[test]
    fn rescale_selection_span_halves_notes() {
        let mut doc = make_doc_with_note();
        // 跨度 101 → 51（约 /2）：end 200 → 150
        doc.rescale_selection_span(51).expect("should edit");
        let n = doc.data.model.notes[60][0];
        assert_eq!(n.start_tick, 100);
        assert_eq!(n.end_tick, 150);
        assert_eq!(doc.edit.selected.rects[0].1, 151);
    }

    #[test]
    fn rescale_selection_span_same_span_returns_none() {
        let mut doc = make_doc_with_note();
        assert!(doc.rescale_selection_span(101).is_none());
    }

    #[test]
    fn rescale_selection_span_empty_selection_returns_none() {
        let mut doc = make_doc_with_note();
        doc.edit.selected.clear();
        assert!(doc.rescale_selection_span(200).is_none());
    }

    #[test]
    fn rescale_undo_restores_notes_and_rects() {
        let mut doc = make_doc_with_note();
        doc.edit.sel_rect.rects = vec![(100.0, 201.0, 60, 60)];
        let before = doc.capture_snapshot();
        let action = doc.rescale_selection_span(202).expect("should edit");
        doc.push_undo(action, "rescale", before);
        assert_eq!(doc.edit.selected.rects[0].1, 302, "编辑后选区跟随缩放");
        assert!(doc.undo(), "undo 应成功");
        // 音符与选区/选框都恢复编辑前状态
        assert_eq!(doc.data.model.notes[60][0].end_tick, 200);
        assert_eq!(doc.edit.selected.rects[0].1, 201);
        assert_eq!(doc.edit.sel_rect.rects[0], (100.0, 201.0, 60, 60));
        assert!(doc.redo(), "redo 应成功");
        assert_eq!(doc.data.model.notes[60][0].end_tick, 300);
        assert_eq!(doc.edit.selected.rects[0].1, 302);
    }

    #[test]
    fn flip_horizontal_mirrors_ticks() {
        let mut doc = make_doc_with_note();
        // 第二个音符 (150, 250) key 60
        doc.add_note(
            0,
            NoteEvent {
                id: 1,
                start_tick: 150,
                end_tick: 250,
                key: 60,
                velocity: 100,
            },
        );
        doc.edit.selected.add_rect_track(150, 251, 60, 60, 0, 0);
        let action = doc
            .flip_selected_notes(FlipAxis::Horizontal)
            .expect("should flip");
        // t0=100, t1=251：
        // n1 (100,200) → start' = 100 + (251-200) = 151, end' = 100 + (251-100) = 251
        // n2 (150,250) → start' = 100 + (251-250) = 101, end' = 100 + (251-150) = 201
        // 桶内按 start 排序：[(101,201), (151,251)]
        assert_eq!(doc.data.model.notes[60][0].start_tick, 101);
        assert_eq!(doc.data.model.notes[60][0].end_tick, 201);
        assert_eq!(doc.data.model.notes[60][1].start_tick, 151);
        assert_eq!(doc.data.model.notes[60][1].end_tick, 251);
        match action {
            UndoAction::FlipNotes { rects, bounds, .. } => {
                assert!(!rects.is_empty());
                assert_eq!(bounds, (100, 251, 60, 60));
            }
            other => panic!("expected FlipNotes, got {other:?}"),
        }
    }

    #[test]
    fn flip_vertical_mirrors_keys() {
        let mut doc = make_doc_with_note(); // key 60（alloc id 1）
        doc.add_note(
            0,
            NoteEvent {
                id: 2,
                start_tick: 100,
                end_tick: 200,
                key: 64,
                velocity: 100,
            },
        );
        doc.edit.selected.add_rect_track(100, 201, 64, 64, 0, 0);
        doc.flip_selected_notes(FlipAxis::Vertical)
            .expect("should flip");
        // kl=60, kh=64：60↔64（add_note 忽略传入 id，按 alloc 顺序为 1、2）
        assert!(
            doc.data.model.notes[64].iter().any(|n| n.id == 1),
            "key 60 的音符应镜像到 64"
        );
        assert!(
            doc.data.model.notes[60].iter().any(|n| n.id == 2),
            "key 64 的音符应镜像到 60"
        );
    }

    #[test]
    fn flip_undo_restores_notes() {
        let mut doc = make_doc_with_note();
        let before = doc.capture_snapshot();
        let action = doc
            .flip_selected_notes(FlipAxis::Horizontal)
            .expect("should flip");
        doc.push_undo(action, "flip", before);
        assert!(doc.undo(), "undo 应成功");
        let n = doc.data.model.notes[60][0];
        assert_eq!(n.start_tick, 100);
        assert_eq!(n.end_tick, 200);
    }

    #[test]
    fn apply_note_field_edit_velocity_remembers_latest_tick() {
        let mut doc = make_doc_with_note();
        doc.add_note(
            0,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 200,
                end_tick: 300,
                key: 60,
                velocity: 90,
            },
        );
        // 选区覆盖 t100 与 t200 两个音符
        doc.edit.selected.add_rect_track(100, 300, 60, 60, 0, 0);
        let ops = crate::num_expr::parse_num_expr("60").unwrap(); // 赋值 60
        assert!(
            doc.apply_note_field_edit(NoteField::Velocity, &ops)
                .is_some()
        );
        // 记录时间最晚（t200）的 60
        assert_eq!(doc.edit.default_velocity(0), 60);
    }

    #[test]
    fn set_note_end_tick_updates_gate_and_undo() {
        let mut doc = make_doc_with_note();
        let action = doc
            .add_note(
                0,
                yinhe_core::NoteEvent {
                    id: 0,
                    start_tick: 1000,
                    end_tick: 1001,
                    key: 62,
                    velocity: 80,
                },
            )
            .expect("add_note 应成功");
        let note_id = match &action {
            crate::history::UndoAction::Notes(d) => d.after[0].0.id,
            other => panic!("unexpected action {other:?}"),
        };

        // NoteOff 闭合 gate
        let before = doc.capture_snapshot();
        let update = doc
            .set_note_end_tick(62, note_id, 1480)
            .expect("set_note_end_tick 应成功");
        doc.push_undo(update, "record", before);
        let n = doc.data.model.notes[62][0];
        assert_eq!(n.end_tick, 1480);

        // undo 恢复原 gate
        assert!(doc.undo(), "undo 应成功");
        let n = doc.data.model.notes[62][0];
        assert_eq!(n.end_tick, 1001);
    }

    #[test]
    fn set_note_end_tick_clamps_below_start() {
        let mut doc = make_doc_with_note();
        let action = doc
            .add_note(
                0,
                yinhe_core::NoteEvent {
                    id: 0,
                    start_tick: 1000,
                    end_tick: 2000,
                    key: 62,
                    velocity: 80,
                },
            )
            .expect("add_note 应成功");
        let note_id = match &action {
            crate::history::UndoAction::Notes(d) => d.after[0].0.id,
            other => panic!("unexpected action {other:?}"),
        };
        // 快速弹放：end_tick == start_tick → 钳制为 start+1（仍返回 Some）
        assert!(doc.set_note_end_tick(62, note_id, 1000).is_some());
        let n = doc.data.model.notes[62][0];
        assert_eq!(n.end_tick, 1001);
    }

    fn ev(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
        }
    }

    /// 「允许新重叠音符」默认开：重叠新音符照常添加（保持现状行为）。
    #[test]
    fn add_note_allows_overlap_by_default() {
        let mut doc = make_doc_with_note(); // 已有 [100,200) k60
        assert!(
            doc.add_note(0, ev(150, 250, 60)).is_some(),
            "默认应允许重叠"
        );
        assert_eq!(doc.data.model.notes[60].len(), 2);
    }

    /// 开关关闭时 add_note 整个拒绝重叠新音符；相接/跨 key 不算重叠。
    #[test]
    fn add_note_rejects_overlap_when_disallowed() {
        let mut doc = make_doc_with_note(); // 已有 [100,200) k60
        doc.edit.allow_overlapping_notes = false;

        // 相交 → 无视（返回 None，模型不变，也不消耗 id）
        assert!(
            doc.add_note(0, ev(150, 250, 60)).is_none(),
            "重叠新音符应被无视"
        );
        assert_eq!(doc.data.model.notes[60].len(), 1);

        // 首尾相接（左闭右开区间）→ 不算重叠，放行
        assert!(doc.add_note(0, ev(200, 300, 60)).is_some(), "相接不算重叠");
        // 跨 key → 不同桶，放行
        assert!(
            doc.add_note(0, ev(150, 250, 61)).is_some(),
            "跨 key 不算重叠"
        );
        // 不重叠 → 放行
        assert!(doc.add_note(0, ev(400, 500, 60)).is_some());
    }

    /// move_selected_notes：目标被非移动集合的已有音符占据的音符留在原处，
    /// 其余正常移动；undo 回退副本制且只含真正移动的音符。
    #[test]
    fn move_selected_notes_partially_blocked_when_disallowed() {
        let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
        doc.add_note(0, ev(100, 150, 62)); // B k62（待选中）
        doc.add_note(0, ev(500, 600, 60)); // C k60 占位（不选中）
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0); // 选中 A、B
        doc.edit.allow_overlapping_notes = false;
        doc.edit.overlap_blocked_behavior =
            crate::audio_settings::OverlapBlockedBehavior::KeepOriginal;

        // +400 tick：A 目标 [500,600) 与 C 重叠 → 留原处；B 目标 k62 [500,550) 无阻挡 → 移动
        let before_snap = doc.capture_snapshot();
        let action = doc
            .move_selected_notes(400, 0)
            .expect("部分移动应产生 undo");
        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1, "delta 只含真正移动的 B");
                assert_eq!(delta.after.len(), 1);
                assert_eq!(delta.after[0].1, 62);
                assert_eq!(delta.after[0].0.start_tick, 500);
            }
            other => panic!("有被拦音符时应回退副本制 Notes，实际 {other:?}"),
        }
        // A 留原处、C 不动
        assert_eq!(doc.data.model.notes[60].len(), 2);
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200),
            "A 应留在原处"
        );
        // B 已移动
        let b = doc.data.model.notes[62]
            .iter()
            .find(|n| n.start_tick == 500)
            .copied();
        assert_eq!(b.map(|n| n.end_tick), Some(550), "B 应移到 [500,550)");

        // undo 回放不受开关拦截：B 回到原位置
        doc.push_undo(action, "move", before_snap);
        assert!(doc.undo(), "undo 应成功");
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 150),
            "undo 后 B 应回到 [100,150)"
        );
        // redo 同样不受拦截
        assert!(doc.redo(), "redo 应成功");
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 500 && n.end_tick == 550),
            "redo 后 B 应再次位于 [500,550)"
        );
    }

    /// move_selected_notes：全部被拦时整体无视（None，选区与模型不变）。
    #[test]
    fn move_selected_notes_all_blocked_returns_none() {
        let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
        doc.add_note(0, ev(500, 600, 60)); // C 占位
        doc.edit.allow_overlapping_notes = false;
        doc.edit.overlap_blocked_behavior =
            crate::audio_settings::OverlapBlockedBehavior::KeepOriginal;

        assert!(
            doc.move_selected_notes(400, 0).is_none(),
            "目标全被占据时应整体无视"
        );
        assert_eq!(doc.data.model.notes[60].len(), 2);
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200),
            "A 应留在原处"
        );
        // 选区未偏移
        assert_eq!(doc.edit.selected.rects[0].0, 100);
    }

    /// duplicate_selected：与已有音符重叠的副本跳过，其余正常插入。
    #[test]
    fn duplicate_selected_skips_overlapping_copy() {
        let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
        doc.add_note(0, ev(100, 150, 62)); // B k62（待选中）
        // C k60 [250,350)：在选区外，但与 A 的副本目标 [200,300) 相交
        doc.add_note(0, ev(250, 350, 60));
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0); // 选中 A、B
        doc.edit.allow_overlapping_notes = false;

        // offset = 100：A 副本 [200,300) 被 C 拦下；B 副本 k62 [200,250) 插入
        let action = doc.duplicate_selected().expect("应有部分副本插入");
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

    /// duplicate_selected：副本全被拦时返回 None。
    #[test]
    fn duplicate_selected_all_blocked_returns_none() {
        let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
        doc.add_note(0, ev(250, 350, 60)); // C 拦下副本目标 [200,300)
        doc.edit.allow_overlapping_notes = false;
        assert!(
            doc.duplicate_selected().is_none(),
            "副本全被拦时应返回 None"
        );
        assert_eq!(doc.data.model.notes[60].len(), 2);
    }

    /// duplicate_selected_to：与已有音符重叠的副本跳过。
    #[test]
    fn duplicate_selected_to_skips_overlapping_copy() {
        let mut doc = make_doc_with_note(); // A [100,200) k60（选中）
        doc.add_note(0, ev(500, 600, 60)); // C k60 占位
        doc.edit.allow_overlapping_notes = false;

        // 副本目标 [500,600) 与 C 重叠 → 全部跳过
        assert!(doc.duplicate_selected_to(400, 0).is_none());
        assert_eq!(doc.data.model.notes[60].len(), 2);

        // 换 key 方向（k62）无阻挡 → 正常插入
        assert!(doc.duplicate_selected_to(400, 2).is_some());
        assert_eq!(doc.data.model.notes[62].len(), 1);
    }

    /// undo/redo 回放路径（apply NoteDelta）绝不被重叠检查拦截：
    /// 开关开着时制造重叠，关掉后 undo/redo 照常重放。
    #[test]
    fn undo_redo_replay_not_blocked_by_overlap_check() {
        let mut doc = make_doc_with_note(); // A [100,200) k60，默认允许重叠
        let before_snap = doc.capture_snapshot();
        let action = doc.add_note(0, ev(150, 250, 60)).expect("允许重叠时应成功");
        doc.push_undo(action, "add", before_snap);
        assert_eq!(doc.data.model.notes[60].len(), 2);

        doc.edit.allow_overlapping_notes = false;
        assert!(doc.undo(), "undo 应成功");
        assert_eq!(doc.data.model.notes[60].len(), 1);
        assert!(doc.redo(), "redo 应成功（即使重新制造重叠）");
        assert_eq!(doc.data.model.notes[60].len(), 2);
    }
}
