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
            self.edit
                .selected
                .set_members(after.iter().map(|(n, _)| n.id));
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

            // 检测目标位置是否已有非选中音符：若目标选框内已有音符，
            // 操作式 undo 会误搬 B，需回退副本制。
            let has_dest_overlap = {
                let dest_rects: Vec<(u32, u32, u8, u8, u16, u16)> = selection_before
                    .rects
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
                let mut dest_sel = selection_before.clone();
                dest_sel.rects = dest_rects;
                dest_sel.drop_members(); // 纯几何查询：重叠检测不认成员位图
                batch_ops::any_selected(model, &dest_sel)
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
                selection: selection_before,
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

        // ── 快速路径：同 key、无 clamp、目标无重叠 → 逐桶原地改 ──
        // 判定全部流式只读（不物化）；任一不满足回退下方副本路径。
        // 1.64 亿全选移动的峰值从 ~6.5GB（remove+副本+insert）降到桶级。
        if delta_keys == 0 {
            let selection_before = self.edit.selected.clone();
            let allow_overlap = self.edit.allow_overlapping_notes;
            let fast = {
                let model = &self.data.model;
                let mut clamp_free = true;
                batch_ops::for_each_selected(model, &selection_before, |n, _| {
                    let ns = n.start_tick as i64 + delta_ticks;
                    let ne = n.end_tick as i64 + delta_ticks;
                    if ns < 0 || ne > u32::MAX as i64 {
                        clamp_free = false;
                    }
                });
                if !clamp_free {
                    false
                } else if allow_overlap {
                    true
                } else {
                    // 目标区域是否有「不属于本次移动集合」的音符（否则撤销会误搬）。
                    let mut dest_sel = selection_before.clone();
                    for r in &mut dest_sel.rects {
                        r.0 = (r.0 as i64 + delta_ticks).max(0) as u32;
                        r.1 = (r.1 as i64 + delta_ticks).max(0) as u32;
                    }
                    dest_sel.drop_members(); // 纯几何查询
                    let mut overlap = false;
                    batch_ops::for_each_selected(model, &dest_sel, |n, k| {
                        let in_original =
                            selection_before
                                .rects
                                .iter()
                                .any(|&(ts, te, kl, kh, tl, th)| {
                                    n.start_tick >= ts
                                        && n.start_tick < te
                                        && k >= kl
                                        && k <= kh
                                        && n.track >= tl
                                        && n.track <= th
                                })
                                && selection_before.accepts_note(n, k);
                        if !in_original {
                            overlap = true;
                        }
                    });
                    !overlap
                }
            };
            if fast {
                let model = Arc::make_mut(&mut self.data.model);
                crate::batch_ops::update_selected_in_place(model, &selection_before, |n, _k| {
                    let length = n.end_tick - n.start_tick;
                    n.start_tick = (n.start_tick as i64 + delta_ticks) as u32;
                    n.end_tick = n.start_tick + length;
                });
                model.rebuild_dirty();
                self.edit.selected.offset(delta_ticks, 0);
                self.data.bump_revision();
                return Some(UndoAction::MoveNotes {
                    selection: selection_before,
                    delta_ticks,
                    delta_keys: 0,
                });
            }
        }

        let model = Arc::make_mut(&mut self.data.model);

        // Batch removal + collect removed notes.
        let originals = batch_ops::remove_selected(model, &self.edit.selected);
        // 操作式 undo 需要操作**前**的选区矩形（offset 之前捕获）。
        let selection_before = self.edit.selected.clone();
        // 检测目标位置是否已有非选中音符（重叠允许时也会产生）：
        // 若目标选框内已有音符，操作式 undo（按选框收集）在撤销时会把
        // 这些原本不在选区的 B 音符也一起平移回 A，造成“B 被搬到 A”的 bug。
        // 此时必须回退到副本制（NoteDelta），仅精确撤销被移动的音符。
        // 必须在插入新音符前检测（此时模型仅含 B 等非选中音符）。
        let has_dest_overlap = {
            let dest_rects: Vec<(u32, u32, u8, u8, u16, u16)> = selection_before
                .rects
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
            let mut dest_sel = selection_before.clone();
            dest_sel.rects = dest_rects;
            dest_sel.drop_members(); // 纯几何查询：重叠检测不认成员位图
            batch_ops::any_selected(model, &dest_sel)
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
                selection: selection_before,
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

        // ── 快速路径：无 clamp、目标无重叠 → 逐桶原地改（操作式 undo）──
        // 判定全部流式只读（不物化）；target 区域用平移后的选框近似
        //（命中区间的超集，宁可回退也不漏报重叠）。
        let selection_before = self.edit.selected.clone();
        let allow_overlap = self.edit.allow_overlapping_notes;
        let mut hit_any = false;
        let fast_ok = {
            let model = &self.data.model;
            let mut ok = true;
            batch_ops::for_each_selected(model, &selection_before, |n, _| {
                hit_any = true;
                match side {
                    ResizeSide::Left => {
                        let ns = n.start_tick as i64 + dt;
                        if ns < 0 || ns > n.end_tick as i64 - 1 {
                            ok = false;
                        }
                    }
                    ResizeSide::Right => {
                        let ne = n.end_tick as i64 + dt;
                        if ne < n.start_tick as i64 + 1 || ne > u32::MAX as i64 {
                            ok = false;
                        }
                    }
                }
            });
            if !ok {
                false
            } else if allow_overlap {
                true
            } else {
                let mut dest_sel = selection_before.clone();
                for r in &mut dest_sel.rects {
                    match side {
                        ResizeSide::Left => {
                            r.0 = (r.0 as i64 + dt).max(0) as u32;
                        }
                        ResizeSide::Right => {
                            r.1 = (r.1 as i64 + dt).max(0) as u32;
                        }
                    }
                }
                dest_sel.drop_members(); // 纯几何查询
                let mut overlap = false;
                batch_ops::for_each_selected(model, &dest_sel, |n, k| {
                    let in_original =
                        selection_before
                            .rects
                            .iter()
                            .any(|&(ts, te, kl, kh, tl, th)| {
                                n.start_tick >= ts
                                    && n.start_tick < te
                                    && k >= kl
                                    && k <= kh
                                    && n.track >= tl
                                    && n.track <= th
                            })
                            && selection_before.accepts_note(n, k);
                    if !in_original {
                        overlap = true;
                    }
                });
                !overlap
            }
        };
        if fast_ok {
            let model = Arc::make_mut(&mut self.data.model);
            crate::batch_ops::update_selected_in_place(model, &selection_before, |n, _k| {
                let old_start = n.start_tick;
                match side {
                    ResizeSide::Left => {
                        n.start_tick = (n.start_tick as i64 + dt) as u32;
                    }
                    ResizeSide::Right => {
                        n.end_tick = (n.end_tick as i64 + dt) as u32;
                    }
                }
                // 记录"最近修改长度"（同轨取时间最晚，左拉伸用原 start 比较）
                let gate = n.end_tick - n.start_tick;
                self.edit.remember_gate(n.track, old_start, gate);
            });
            // 选区单边跟随（与副本路径一致）
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
                        r.1 = (r.1 as i64 + dt).max(r.0 as i64 + 1) as u32;
                    }
                }
            }
            model.rebuild_dirty();
            self.data.bump_revision();
            if hit_any {
                return Some(UndoAction::ResizeNotes {
                    selection: selection_before,
                    side,
                    delta_ticks: dt,
                });
            }
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
        // 记录"最近修改长度"：新音符默认长度跟随最近一次修改（同轨取时间最晚的音符）
        // 左拉伸时 start_tick 变早，用原 start_tick 做"最晚时间"比较
        for ((before, _), (after, _)) in moved_before.iter().zip(moved_after.iter()) {
            let gate = after.end_tick - after.start_tick;
            self.edit
                .remember_gate(before.track, before.start_tick, gate);
        }

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
            let mut sel = self.edit.selected.clone();
            sel.drop_members(); // 纯几何查询：目标选框内是否有非选中音符
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

    /// 剪刀切割：对每个 `(key, cut)`，切开该行中跨过 `cut` 的音符。
    ///
    /// 只切 `track_selected`（空 = 全部）∩ `track_pianoroll_visible` 的轨道；
    /// 切点在音符边界上或音符外时不切。左半保留原 id，右半分配新 id。
    /// 返回副本制 undo（before = 原音符，after = 两半）。
    pub fn split_notes_at(&mut self, cuts: &[(u8, u32)]) -> Option<UndoAction> {
        if cuts.is_empty() {
            return None;
        }
        let mut before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        {
            let selected = &self.edit.track_selected;
            let visible = &self.edit.track_pianoroll_visible;
            let model = Arc::make_mut(&mut self.data.model);
            let max_len = model.max_note_len;
            for &(key, cut) in cuts {
                if key as usize >= yinhe_types::KEY_COUNT {
                    continue;
                }
                let targets = {
                    let bucket = Arc::make_mut(&mut model.notes[key as usize]);
                    // 左界用全曲最长音符收紧：start < cut 且 end > cut 的音符
                    // 必然落在 [cut - max_len, cut] 内。
                    bucket.drain_range_filtered(
                        cut.saturating_sub(max_len),
                        cut.saturating_add(1),
                        |n| {
                            n.start_tick < cut
                                && n.end_tick > cut
                                && (selected.is_empty() || selected.contains(&n.track))
                                && visible.get(n.track as usize).copied().unwrap_or(true)
                        },
                    )
                };
                if targets.is_empty() {
                    continue;
                }
                let new_ids: Vec<u32> = (0..targets.len()).map(|_| model.alloc_note_id()).collect();
                let mut halves = Vec::with_capacity(targets.len() * 2);
                for (n, new_id) in targets.iter().zip(new_ids) {
                    let left = yinhe_types::Note {
                        end_tick: cut,
                        ..*n
                    };
                    let right = yinhe_types::Note {
                        id: new_id,
                        start_tick: cut,
                        end_tick: n.end_tick,
                        velocity: n.velocity,
                        track: n.track,
                    };
                    before.push((*n, key));
                    after.push((left, key));
                    after.push((right, key));
                    halves.push(left);
                    halves.push(right);
                }
                Arc::make_mut(&mut model.notes[key as usize]).insert_batch_sorted(halves);
                model.mark_dirty(key);
            }
        }
        if after.is_empty() {
            return None;
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }

    /// 网格工具确认：把选框内（含边界）的音符按当前量化网格切开。
    ///
    /// - 只切选框与音符的交集区段：框外的音符头/尾保留；
    /// - 切点 = 落在 `[t0, t1]` 且严格处于音符内部的量化网格点；
    /// - 作用域与其它 PR 编辑一致：`track_selected`（空 = 全部）∩ PR 可见轨。
    ///
    /// 返回副本制 undo（before = 原音符，after = 各段）。
    pub fn split_selection_by_grid(&mut self) -> Option<UndoAction> {
        let interval = self
            .edit
            .quantize_pianoroll
            .tick_interval(self.data.model.meta.ppq);
        if interval == 0 || self.edit.sel_rect.rects.is_empty() {
            return None;
        }
        let rects: Vec<(f64, f64, u8, u8)> = self.edit.sel_rect.rects.clone();
        let interval = interval as u64;
        let mut before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        {
            let selected = &self.edit.track_selected;
            let visible = &self.edit.track_pianoroll_visible;
            let model = Arc::make_mut(&mut self.data.model);
            let max_len = model.max_note_len;
            for &(t0f, t1f, key_lo, key_hi) in &rects {
                let t0 = t0f.max(0.0) as u32;
                let t1 = t1f.max(0.0) as u32;
                if t1 <= t0 {
                    continue;
                }
                for key in key_lo..=key_hi {
                    let targets = {
                        let bucket = Arc::make_mut(&mut model.notes[key as usize]);
                        bucket.drain_range_filtered(
                            t0.saturating_sub(max_len),
                            t1.saturating_add(1),
                            |n| {
                                n.start_tick < t1
                                    && n.end_tick > t0
                                    && (selected.is_empty() || selected.contains(&n.track))
                                    && visible.get(n.track as usize).copied().unwrap_or(true)
                                    && !grid_cuts_in_note(n, t0, t1, interval).is_empty()
                            },
                        )
                    };
                    if targets.is_empty() {
                        continue;
                    }
                    let per_note: Vec<(yinhe_types::Note, Vec<u32>)> = targets
                        .into_iter()
                        .map(|n| {
                            let cuts = grid_cuts_in_note(&n, t0, t1, interval);
                            (n, cuts)
                        })
                        .collect();
                    // 首段沿用原 id，其余段各分配一个新 id。
                    let mut halves = Vec::new();
                    for (n, cuts) in &per_note {
                        before.push((*n, key));
                        let mut seg_start = n.start_tick;
                        for (i, &cut) in cuts.iter().enumerate() {
                            let id = if i == 0 { n.id } else { model.alloc_note_id() };
                            let seg = yinhe_types::Note {
                                id,
                                start_tick: seg_start,
                                end_tick: cut,
                                ..*n
                            };
                            seg_start = cut;
                            after.push((seg, key));
                            halves.push(seg);
                        }
                        let tail = yinhe_types::Note {
                            id: model.alloc_note_id(),
                            start_tick: seg_start,
                            end_tick: n.end_tick,
                            ..*n
                        };
                        after.push((tail, key));
                        halves.push(tail);
                    }
                    Arc::make_mut(&mut model.notes[key as usize]).insert_batch_sorted(halves);
                    model.mark_dirty(key);
                }
            }
        }
        if after.is_empty() {
            return None;
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }

    /// 批量添加音符到指定 track（直线/刷子工具）。
    ///
    /// 「允许新重叠音符」关闭时逐个过滤与已有音符重叠的新音符；
    /// 批次内部互不影响（与 `duplicate_selected` 语义一致）。
    /// 返回副本制 undo（before 空，after = 实际添加的音符）。
    pub fn add_notes_batch(&mut self, track: u16, notes: &[NoteEvent]) -> Option<UndoAction> {
        if notes.is_empty() || track as usize >= self.data.model.tracks.len() {
            return None;
        }
        if Some(track) == self.edit.conductor_track_idx {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        {
            let model = Arc::make_mut(&mut self.data.model);
            let mut by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for ev in notes {
                if !allow_overlap
                    && batch_ops::has_overlapping_note(
                        model,
                        track,
                        ev.key,
                        ev.start_tick,
                        ev.end_tick,
                    )
                {
                    continue;
                }
                let note = yinhe_types::Note {
                    id: model.alloc_note_id(),
                    start_tick: ev.start_tick,
                    end_tick: ev.end_tick.max(ev.start_tick.saturating_add(1)),
                    velocity: ev.velocity,
                    track,
                };
                by_key.entry(ev.key).or_default().push(note);
            }
            if by_key.is_empty() {
                return None;
            }
            for (key, group) in &by_key {
                after.extend(group.iter().map(|n| (*n, *key)));
            }
            batch_ops::insert_batch(model, by_key);
        }
        self.data.rebuild_model_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// 直线工具确认：沿音高行生成音符（每行一个，gate = 一个量化间隔）。
    ///
    /// start = 线在该行的 tick 吸附量化（含小节感知）；力度 = 该轨记忆力度。
    /// 目标轨 = 主音轨（与 PR 铅笔/选框一致），无主音轨/Conductor 时返回 None。
    pub fn generate_line_notes(&mut self) -> Option<UndoAction> {
        let line = self.edit.line_tool_line?;
        let track = self.edit.main_track()?;
        if Some(track) == self.edit.conductor_track_idx {
            return None;
        }
        let ppq = self.data.model.meta.ppq;
        let quantize = self.edit.quantize_pianoroll;
        let interval = quantize.tick_interval(ppq);
        if interval == 0 {
            return None;
        }
        let (tpb, num, den, events) = pr_bar_line_data(&self.data.model);
        let bar = Some((tpb, num, den, events.as_slice()));
        let (_, k1) = line.start;
        let (_, k2) = line.end;
        let (lo, hi) = (k1.min(k2), k1.max(k2));
        let velocity = self.edit.default_velocity(track);
        let mut notes = Vec::with_capacity(hi as usize - lo as usize + 1);
        for key in lo..=hi {
            let raw = crate::quantize::line_tick_at_key(line.start, line.end, key);
            let start = crate::quantize::snap_tick(raw, quantize, ppq, bar).max(0.0) as u32;
            notes.push(NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: start.saturating_add(interval),
                key,
                velocity,
            });
        }
        self.add_notes_batch(track, &notes)
    }

    /// 剪刀工具确认：按待确认锚点线切开音符（切完由调用方清空线）。
    pub fn split_scissors_line(&mut self) -> Option<UndoAction> {
        let line = self.edit.scissors_line?;
        let ppq = self.data.model.meta.ppq;
        let quantize = self.edit.quantize_pianoroll;
        let (tpb, num, den, events) = pr_bar_line_data(&self.data.model);
        let bar = Some((tpb, num, den, events.as_slice()));
        let cuts = crate::quantize::line_cuts(line.start, line.end, quantize, ppq, bar);
        self.split_notes_at(&cuts)
    }
}

/// 构造 PR 的小节线感知 snap 参数（与 content.rs 传给钢琴卷帘的一致）。
fn pr_bar_line_data(model: &yinhe_core::YinModel) -> (u32, u8, u8, Vec<yinhe_types::TimeSigEvent>) {
    let tpb = model.meta.ppq;
    let first = model.conductor.time_sig.first();
    let num = first.map(|t| t.numerator).unwrap_or(4);
    let den = first.map(|t| t.denominator).unwrap_or(2);
    (tpb, num, den, model.conductor.time_sig.clone())
}

/// 音符在 `[t0, t1]` 内、严格处于音符内部的量化网格切点（升序）。
///
/// 无切点时返回空。`interval` 已保证 > 0。
fn grid_cuts_in_note(n: &yinhe_types::Note, t0: u32, t1: u32, interval: u64) -> Vec<u32> {
    let lower = (n.start_tick as u64 + 1).max(t0 as u64);
    let upper = (n.end_tick as u64).saturating_sub(1).min(t1 as u64);
    let mut cuts = Vec::new();
    if lower > upper {
        return cuts;
    }
    let first = lower.div_ceil(interval) * interval;
    let mut t = first;
    while t <= upper {
        cuts.push(t as u32);
        t += interval;
    }
    cuts
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
                        id: 0,
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
            doc_id: 0,
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

    /// 回归：重叠选框下同一音符只移动一次（位移小、移动后仍落在另一 rect
    /// 内时，曾按 rect 重复命中导致两倍位移）。
    #[test]
    fn overlapping_rects_move_applies_once() {
        let mut doc = make_doc_with_note();
        // 追加一个与原选区重叠的 rect，两 rect 都覆盖 tick=100 的音符；
        // 移动 +10 后 110 仍落在 [50,150) 内（重复命中场景）。
        doc.edit.selected.add_rect_track(50, 150, 60, 60, 0, 0);
        doc.move_selected_notes(10, 0).expect("移动应产生 undo");
        let ticks: Vec<u32> = doc.data.model.notes[60]
            .iter()
            .map(|n| n.start_tick)
            .collect();
        assert_eq!(ticks, vec![110], "只应移动一次（+10），实际 {ticks:?}");
    }

    /// 回归：重叠选框下复制不产生重复副本。
    #[test]
    fn overlapping_rects_duplicate_once() {
        let mut doc = make_doc_with_note();
        doc.edit.selected.add_rect_track(50, 150, 60, 60, 0, 0);
        doc.duplicate_selected().expect("复制应产生 undo");
        let total: usize = (0..yinhe_types::KEY_COUNT)
            .map(|k| doc.data.model.notes[k].len())
            .sum();
        assert_eq!(total, 2, "原件 + 恰好一份副本");
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
        let snap = doc.capture_snapshot();
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

        // 无 clamp/无重叠 → 操作式（O(1) undo）
        match &action {
            UndoAction::ResizeNotes {
                side, delta_ticks, ..
            } => {
                assert_eq!(*side, ResizeSide::Right);
                assert_eq!(*delta_ticks, 50);
            }
            other => panic!("期望 UndoAction::ResizeNotes，实际 {other:?}"),
        }
        // undo/redo 精确
        doc.push_undo(action, "resize-right", snap);
        assert!(doc.undo());
        assert_eq!(doc.data.model.notes[60][0].end_tick, 200);
        assert!(doc.redo());
        assert_eq!(doc.data.model.notes[60][0].end_tick, 250);
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

    /// 左拉伸快速路径 round-trip：undo/redo 的音符与选区单边跟随都精确。
    #[test]
    fn resize_selected_notes_left_fast_path_roundtrips() {
        let mut doc = make_doc_with_note();
        let snap = doc.capture_snapshot();
        let action = doc
            .resize_selected_notes(ResizeSide::Left, -30)
            .expect("应产生 UndoAction");
        assert!(
            matches!(
                action,
                UndoAction::ResizeNotes {
                    side: ResizeSide::Left,
                    ..
                }
            ),
            "无 clamp/无重叠应走操作式"
        );
        doc.push_undo(action, "resize-left", snap);

        assert!(doc.undo());
        assert_eq!(doc.data.model.notes[60][0].start_tick, 100);
        assert_eq!(doc.edit.selected.rects[0].0, 100, "undo 后选区左边界回位");
        assert!(doc.redo());
        assert_eq!(doc.data.model.notes[60][0].start_tick, 70);
        assert_eq!(doc.edit.selected.rects[0].0, 70, "redo 后选区左边界跟随");
    }

    /// 拉伸目标与非选中音符重叠（禁止重叠）→ 回退副本路径并按 behavior 拦截
    ///（默认 KeepOriginal 且全拦 → 返回 None、音符保持原样）。
    #[test]
    fn resize_selected_notes_overlap_falls_back_to_blocked() {
        let mut doc = make_doc_with_note(); // t100~200, key 60, 选区 t100~201
        doc.edit.allow_overlapping_notes = false;
        doc.add_note(
            0,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 400,
                key: 60,
                velocity: 90,
            },
        );
        // 右拉伸 +150 → 目标 [100,350) 与 [300,400) 重叠 → 全拦。
        assert!(
            doc.resize_selected_notes(ResizeSide::Right, 150).is_none(),
            "全拦应返回 None"
        );
        assert_eq!(
            doc.data.model.notes[60][0].end_tick, 200,
            "被拦音符保持原样"
        );
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
            UndoAction::FlipNotes {
                selection, bounds, ..
            } => {
                assert!(!selection.rects.is_empty());
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

    /// 回归：多个重叠 rect 命中同一音符时，单趟原地路径不重复收集
    ///（旧实现按 rect×key 遍历会 push 重复项，undo 数据膨胀）。
    #[test]
    fn apply_note_field_edit_overlapping_rects_dedup() {
        let mut doc = make_doc_with_note(); // key 60, t100-200, vel 100
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(0, 300, 60, 60, 0, 0);
        doc.edit.selected.add_rect_track(100, 400, 60, 60, 0, 0);
        let ops = crate::num_expr::parse_num_expr("50").unwrap(); // 赋值 50
        let action = doc
            .apply_note_field_edit(NoteField::Velocity, &ops)
            .expect("应命中");
        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1, "重叠 rect 不应重复收集");
                assert_eq!(delta.after.len(), 1);
                assert_eq!(delta.after[0].0.velocity, 50);
            }
            other => panic!("expected Notes, got {other:?}"),
        }
        let snap = doc.capture_snapshot();
        doc.push_undo(action, "vel-dedup", snap);
        assert!(doc.undo());
        assert_eq!(doc.data.model.notes[60][0].velocity, 100);
        assert!(doc.redo());
        assert_eq!(doc.data.model.notes[60][0].velocity, 50);
    }

    #[test]
    fn apply_note_field_edit_gate_remembers_latest_tick() {
        let mut doc = make_doc_with_note(); // t100 gate 100
        doc.add_note(
            0,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 500,
                key: 60,
                velocity: 90,
            },
        ); // t300 gate 200
        // 选区覆盖两者
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(100, 501, 60, 60, 0, 0);
        // 批量改 gate：赋值 240
        let ops = crate::num_expr::parse_num_expr("240").unwrap();
        assert!(doc.apply_note_field_edit(NoteField::Gate, &ops).is_some());
        // 记录时间最晚（t300）的 240，且无记忆时回退 120
        assert_eq!(doc.edit.default_gate(0, 120), 240);
        assert_eq!(doc.edit.default_gate(1, 120), 120);
        // 未命中不覆盖
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(9999, 10000, 60, 60, 0, 0);
        let ops2 = crate::num_expr::parse_num_expr("10").unwrap();
        assert!(doc.apply_note_field_edit(NoteField::Gate, &ops2).is_none());
        assert_eq!(doc.edit.default_gate(0, 120), 240);
    }

    #[test]
    fn resize_selected_notes_remembers_gate() {
        let mut doc = make_doc_with_note(); // t100~200 gate 100
        doc.add_note(
            0,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 500,
                key: 60,
                velocity: 90,
            },
        ); // t300~500 gate 200
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(100, 501, 60, 60, 0, 0);
        doc.edit.sel_rect.clear();
        doc.edit.sel_rect.push_rect((100.0, 501.0, 60, 60), false);
        // 右拉 20 tick：gate 变 120 / 220，记时间最晚的 220
        assert!(
            doc.resize_selected_notes(crate::edit_state::ResizeSide::Right, 20)
                .is_some()
        );
        assert_eq!(doc.edit.default_gate(0, 120), 220);
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

    /// 筛选后移动只搬匹配音符；action 携带筛选边界，undo 精确恢复。
    #[test]
    fn move_with_velocity_filter_moves_subset_and_undo_restores() {
        let mut doc = make_doc_with_note(); // k60 [100,200) v100（已选中）
        doc.add_note(
            0,
            NoteEvent {
                id: 0,
                start_tick: 100,
                end_tick: 150,
                key: 62,
                velocity: 20,
            },
        );
        doc.edit.selected.clear();
        doc.edit.selected.add_rect_track(100, 201, 60, 62, 0, 0);
        doc.edit.selected.filter.velocity = Some((90, 127)); // 只筛 v100 的 k60

        let before_snap = doc.capture_snapshot();
        let action = doc.move_selected_notes(400, 0).expect("应移动");
        match &action {
            UndoAction::MoveNotes { selection, .. } => {
                assert_eq!(
                    selection.filter.velocity,
                    Some((90, 127)),
                    "操作式 undo 应携带筛选边界"
                );
            }
            other => panic!("应走操作式 MoveNotes，实际 {other:?}"),
        }
        // k60 已移动，k62（v20）原地不动
        assert!(doc.data.model.notes[60].iter().any(|n| n.start_tick == 500));
        assert!(doc.data.model.notes[62].iter().any(|n| n.start_tick == 100));

        doc.push_undo(action, "move", before_snap);
        assert!(doc.undo(), "undo 应成功");
        assert!(
            doc.data.model.notes[60].iter().any(|n| n.start_tick == 100),
            "k60 应回到原位"
        );
        assert!(
            doc.data.model.notes[62].iter().any(|n| n.start_tick == 100),
            "k62 从未移动"
        );
    }

    /// 剪刀切割：切点切开跨过它的音符（原 id 留左半、新 id 给右半），
    /// 边界/范围外不切，undo/redo 完整回放。
    #[test]
    fn split_notes_at_cuts_crossing_notes_and_undo_roundtrip() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        doc.add_note(0, ev(300, 400, 60)); // 不跨切点
        doc.add_note(0, ev(100, 400, 62)); // 另一行的跨切点音符
        let before_snap = doc.capture_snapshot();
        let action = doc.split_notes_at(&[(60, 150), (62, 150)]).expect("应切割");

        // k60: [100,150) + [150,200) + [300,400)
        assert_eq!(doc.data.model.notes[60].len(), 3);
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 150)
        );
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 150 && n.end_tick == 200)
        );
        // k62: [100,150) + [150,400)
        assert_eq!(doc.data.model.notes[62].len(), 2);
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 150 && n.end_tick == 400)
        );
        // 左半保留原 id
        let orig_id = doc.data.model.notes[60]
            .iter()
            .find(|n| n.start_tick == 100 && n.end_tick == 150)
            .map(|n| n.id);
        assert!(orig_id.is_some());

        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 2, "两个原音符");
                assert_eq!(delta.after.len(), 4, "切出四半");
            }
            other => panic!("期望副本制 Notes，实际 {other:?}"),
        }

        doc.push_undo(action, "split", before_snap);
        assert!(doc.undo(), "undo 应成功");
        assert_eq!(doc.data.model.notes[60].len(), 2);
        assert!(
            doc.data.model.notes[60]
                .iter()
                .any(|n| n.start_tick == 100 && n.end_tick == 200)
        );
        assert_eq!(doc.data.model.notes[62].len(), 1);
        assert!(doc.redo(), "redo 应成功");
        assert_eq!(doc.data.model.notes[60].len(), 3);
        assert_eq!(doc.data.model.notes[62].len(), 2);
    }

    /// 切点在音符边界（start/end）上或音符外：不切，返回 None。
    #[test]
    fn split_notes_at_skips_boundary_and_outside_cuts() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        assert!(
            doc.split_notes_at(&[(60, 100)]).is_none(),
            "cut == start 不切"
        );
        assert!(
            doc.split_notes_at(&[(60, 200)]).is_none(),
            "cut == end 不切"
        );
        assert!(doc.split_notes_at(&[(60, 50)]).is_none(), "音符左侧不切");
        assert!(doc.split_notes_at(&[(60, 500)]).is_none(), "音符右侧不切");
        assert!(doc.split_notes_at(&[]).is_none(), "空切点表");
        assert_eq!(doc.data.model.notes[60].len(), 1);
    }

    /// 作用域过滤：不可见轨、选中其他轨都不切；选中为空 = 全部可见轨。
    #[test]
    fn split_notes_at_respects_track_scope() {
        let mut doc = make_doc_with_note(); // track 0, k60 [100,200)
        doc.edit.track_pianoroll_visible = vec![false];
        assert!(doc.split_notes_at(&[(60, 150)]).is_none(), "不可见轨不切");
        doc.edit.track_pianoroll_visible = vec![true];
        doc.edit.track_selected.insert(5);
        assert!(
            doc.split_notes_at(&[(60, 150)]).is_none(),
            "选中其他轨时不切"
        );
        doc.edit.track_selected.clear();
        assert!(
            doc.split_notes_at(&[(60, 150)]).is_some(),
            "清空选中 = 全部可见轨"
        );
    }

    /// 网格切割：只切选框内的区段（含边界切点），框外头尾保留，
    /// 无切点的音符不动；undo/redo 完整回放。
    #[test]
    fn split_selection_by_grid_cuts_inside_box_only() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(480);
        doc.add_note(0, ev(0, 960, 62)); // 跨框边界的长音
        doc.add_note(0, ev(100, 300, 64)); // 不跨任何网格点
        doc.edit.sel_rect.rects = vec![(480.0, 960.0, 60, 64)];

        let before_snap = doc.capture_snapshot();
        let action = doc.split_selection_by_grid().expect("应切割");
        // k62 [0,960) → [0,480) + [480,960)：480 同时在框边界与音符内部
        assert_eq!(doc.data.model.notes[62].len(), 2);
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 0 && n.end_tick == 480)
        );
        assert!(
            doc.data.model.notes[62]
                .iter()
                .any(|n| n.start_tick == 480 && n.end_tick == 960)
        );
        // k60 [100,200)、k64 [100,300)：框内无网格点 → 不切
        assert_eq!(doc.data.model.notes[60].len(), 1);
        assert_eq!(doc.data.model.notes[64].len(), 1);

        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1);
                assert_eq!(delta.after.len(), 2);
            }
            other => panic!("期望副本制 Notes，实际 {other:?}"),
        }

        doc.push_undo(action, "grid", before_snap);
        assert!(doc.undo(), "undo 应成功");
        assert_eq!(doc.data.model.notes[62].len(), 1);
        assert!(doc.redo(), "redo 应成功");
        assert_eq!(doc.data.model.notes[62].len(), 2);
    }

    /// 网格切割：框完全包含音符时切成多段（首段保留原 id），
    /// 框外部分越出时只切到框边界。
    #[test]
    fn split_selection_by_grid_multi_segments() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(120);
        doc.add_note(0, ev(0, 600, 62));
        doc.edit.sel_rect.rects = vec![(120.0, 480.0, 62, 62)];
        // 切点：240, 360, 480 → [0,240) 跨框左界？不对：切点含框边界 120 吗？
        // 120 在音符内部 (0<120<600) 且 t0<=120<=t1 → 是切点；
        // 240、360、480 同理 → 段：[0,120) [120,240) [240,360) [360,480) [480,600)
        let action = doc.split_selection_by_grid().expect("应切割");
        assert_eq!(doc.data.model.notes[62].len(), 5);
        let starts: Vec<u32> = doc.data.model.notes[62]
            .iter()
            .map(|n| n.start_tick)
            .collect();
        assert_eq!(starts, vec![0, 120, 240, 360, 480]);
        let orig_id = doc.data.model.notes[62]
            .iter()
            .find(|n| n.start_tick == 0)
            .map(|n| n.id);
        assert!(orig_id.is_some(), "首段保留原音符 id");
        match &action {
            UndoAction::Notes(delta) => {
                assert_eq!(delta.before.len(), 1);
                assert_eq!(delta.after.len(), 5);
            }
            other => panic!("期望副本制 Notes，实际 {other:?}"),
        }
    }

    /// 网格切割：空选框 / 不可见轨 / interval 0 都返回 None。
    #[test]
    fn split_selection_by_grid_returns_none_when_scope_blocks() {
        let mut doc = make_doc_with_note();
        doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(480);
        assert!(doc.split_selection_by_grid().is_none(), "无选框");

        // 框与 k60 [100,200) 相交且含切点 120（边界）。
        doc.edit.sel_rect.rects = vec![(120.0, 480.0, 60, 60)];
        doc.edit.track_pianoroll_visible = vec![false];
        assert!(doc.split_selection_by_grid().is_none(), "不可见轨不切");

        doc.edit.track_pianoroll_visible = vec![true];
        doc.edit.track_selected.insert(5);
        assert!(doc.split_selection_by_grid().is_none(), "选中其他轨时不切");

        doc.edit.track_selected.clear();
        doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(0);
        assert!(doc.split_selection_by_grid().is_none(), "interval 0 不切");
        assert_eq!(doc.data.model.notes[60].len(), 1, "模型未被改动");
    }

    /// 直线生成：沿音高行逐行、gate=量化间隔、力度=记忆力度，undo/redo 回放。
    #[test]
    fn generate_line_notes_per_row_with_memory_velocity_and_undo() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        doc.edit.track_selected.insert(0);
        doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(120);
        doc.edit.remember_velocity(0, 0, 77);
        // 线 (0,62) → (480,64)：key 62→0、63→240、64→480
        doc.edit.line_tool_line = Some(crate::edit_state::AnchorLine {
            start: (0.0, 62),
            end: (480.0, 64),
        });

        let before_snap = doc.capture_snapshot();
        let action = doc.generate_line_notes().expect("应生成");
        assert_eq!(doc.data.model.notes[62].len(), 1);
        assert_eq!(doc.data.model.notes[63].len(), 1);
        assert_eq!(doc.data.model.notes[64].len(), 1);
        let n62 = doc.data.model.notes[62][0];
        assert_eq!((n62.start_tick, n62.end_tick, n62.velocity), (0, 120, 77));
        let n63 = doc.data.model.notes[63][0];
        assert_eq!((n63.start_tick, n63.end_tick), (240, 360));
        let n64 = doc.data.model.notes[64][0];
        assert_eq!((n64.start_tick, n64.end_tick), (480, 600));

        match &action {
            UndoAction::Notes(delta) => {
                assert!(delta.before.is_empty());
                assert_eq!(delta.after.len(), 3);
            }
            other => panic!("期望副本制 Notes，实际 {other:?}"),
        }
        doc.push_undo(action, "line", before_snap);
        assert!(doc.undo(), "undo 应成功");
        assert!(doc.data.model.notes[62].is_empty());
        assert!(doc.redo(), "redo 应成功");
        assert_eq!(doc.data.model.notes[62].len(), 1);
    }

    /// 直线生成：无主音轨时返回 None（与 PR 铅笔一致）。
    #[test]
    fn generate_line_notes_requires_main_track() {
        let mut doc = make_doc_with_note();
        doc.edit.line_tool_line = Some(crate::edit_state::AnchorLine {
            start: (0.0, 60),
            end: (0.0, 60),
        });
        assert!(doc.generate_line_notes().is_none(), "无主音轨不生成");
    }

    /// 批量添加：「允许新重叠音符」关闭时过滤与已有音符重叠的项。
    #[test]
    fn add_notes_batch_filters_overlap_when_disabled() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        doc.edit.allow_overlapping_notes = false;
        let notes = vec![
            NoteEvent {
                id: 0,
                start_tick: 150,
                end_tick: 250,
                key: 60,
                velocity: 100,
            },
            NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 400,
                key: 60,
                velocity: 100,
            },
        ];
        let action = doc.add_notes_batch(0, &notes).expect("应添加 1 个");
        assert_eq!(doc.data.model.notes[60].len(), 2);
        match action {
            UndoAction::Notes(delta) => assert_eq!(delta.after.len(), 1),
            other => panic!("期望副本制 Notes，实际 {other:?}"),
        }

        doc.edit.allow_overlapping_notes = true;
        assert!(doc.add_notes_batch(0, &notes).is_some());
        assert_eq!(doc.data.model.notes[60].len(), 4, "允许重叠时全部添加");
    }

    /// 剪刀确认：按锚点线逐行切割（复用 split_notes_at 的作用域与 undo）。
    #[test]
    fn split_scissors_line_cuts_via_anchor_line() {
        let mut doc = make_doc_with_note(); // k60 [100,200)
        doc.edit.quantize_pianoroll = crate::quantize::QuantizePreset::Absolute(120);
        doc.add_note(0, ev(0, 600, 62));
        doc.edit.scissors_line = Some(crate::edit_state::AnchorLine {
            start: (240.0, 60),
            end: (240.0, 62),
        });
        let action = doc.split_scissors_line().expect("应切割");
        assert_eq!(doc.data.model.notes[62].len(), 2, "k62 在 240 处切开");
        assert_eq!(doc.data.model.notes[60].len(), 1, "k60 切点在音符外不切");
        assert!(matches!(action, UndoAction::Notes(_)));

        doc.edit.scissors_line = None;
        assert!(doc.split_scissors_line().is_none(), "无线时返回 None");
    }
}
