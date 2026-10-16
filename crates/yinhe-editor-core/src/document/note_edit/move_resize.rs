//! 拖动与拉伸：几何平移/单边缩放，含快速路径与重叠回退双轨 undo。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::Document;
use crate::edit_state::ResizeSide;
use crate::history::{NoteDelta, UndoAction};

impl Document {
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
                    let dest_sel =
                        selection_before.dest_overlap_selection(delta_ticks, 0, 0, i32::MAX);
                    !batch_ops::compute_dest_overlap(model, &dest_sel, Some(&selection_before))
                }
            };
            if fast {
                let model = Arc::make_mut(&mut self.data.model);
                // 矩形态：顺手收集实际被移动的音符 id，提交后物化为精确成员集
                //（否则移动后矩形覆盖落点路人 → 被误高亮）。
                let needs_materialize = !selection_before.has_explicit_members();
                let mut moved_ids = yinhe_types::NoteBitsetBuilder::new();
                crate::batch_ops::update_selected_in_place(model, &selection_before, |n, _k| {
                    if needs_materialize {
                        moved_ids.insert(n.id);
                    }
                    let length = n.end_tick - n.start_tick;
                    n.start_tick = (n.start_tick as i64 + delta_ticks) as u32;
                    n.end_tick = n.start_tick + length;
                });
                model.rebuild_dirty();
                self.edit.selected.offset(delta_ticks, 0);
                if needs_materialize {
                    self.edit.selected.set_members_bitset(moved_ids.build());
                }
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
        let dest_sel =
            selection_before.dest_overlap_selection(delta_ticks, delta_keys, 0, i32::MAX);
        let has_dest_overlap = batch_ops::compute_dest_overlap(model, &dest_sel, None);
        let allow_overlap = self.edit.allow_overlapping_notes;
        let behavior = self.edit.overlap_blocked_behavior;
        let mut tally = batch_ops::MoveTally::default();
        // 被「禁止重叠」拦下的音符按 behavior 处理：
        // - KeepOriginal：留在原位（当前逻辑）
        // - DeleteOriginal：删除原音符（原消失，目标保留）
        // - ReplaceTarget：删除目标重叠音符并移动（原消失，目标被覆盖）
        for (note, old_key) in &originals {
            let new_key =
                ((*old_key as i32) + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
            let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
            let original = (*note, *old_key);
            // 「允许新重叠音符」关闭：目标位置与非本次移动的已有音符重叠
            // （移动集合已先移除，检查看不到它们）→ 按 behavior 处理。
            if !allow_overlap
                && batch_ops::move_one_note_blocked(
                    model, &original, new_key, new_tick, note.track, behavior, &mut tally,
                )
            {
                continue;
            }
            tally.push_moved(&original, new_key, new_tick, note.track);
        }
        let after = tally.flatten_after();
        batch_ops::insert_batch(model, std::mem::take(&mut tally.new_by_key));

        // 全部被拦且无其他变化（KeepOriginal 全拦）：音符已原样插回，选区不动，不产生 undo。
        // DeleteOriginal / ReplaceTarget 全拦时有删除/替换，需产生 delta。
        if tally.no_change() {
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
        let has_blocked_handling = tally.blocked_any
            || !tally.deleted_before.is_empty()
            || !tally.replaced_before.is_empty();
        if clamp_free && !has_blocked_handling && !has_dest_overlap {
            Some(UndoAction::MoveNotes {
                selection: selection_before,
                delta_ticks,
                delta_keys,
            })
        } else if tally.blocked_any
            && matches!(
                behavior,
                crate::audio_settings::OverlapBlockedBehavior::KeepOriginal
            )
        {
            // 退回：仅真正移动的音符进 delta
            Some(UndoAction::Notes(batch_ops::compose_move_delta(
                tally.moved_before,
                tally.deleted_before,
                tally.replaced_before,
                tally.moved_after,
            )))
        } else {
            // 删除原位 / 替换目标 或目标有重叠（allow=true）：用完整 before/after
            // before 含被替换的目标音符（B），after 仅含新位置的移动音符。
            Some(UndoAction::Notes(batch_ops::compose_move_delta(
                tally.moved_before,
                tally.deleted_before,
                tally.replaced_before,
                after,
            )))
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
                !batch_ops::compute_dest_overlap(model, &dest_sel, Some(&selection_before))
            }
        };
        if fast_ok {
            let model = Arc::make_mut(&mut self.data.model);
            // 矩形态：收集实际被拉伸的音符 id，提交后物化为精确成员集。
            let needs_materialize = !selection_before.has_explicit_members();
            let mut resized_ids = yinhe_types::NoteBitsetBuilder::new();
            crate::batch_ops::update_selected_in_place(model, &selection_before, |n, _k| {
                if needs_materialize {
                    resized_ids.insert(n.id);
                }
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
            if needs_materialize {
                self.edit.selected.set_members_bitset(resized_ids.build());
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

        let after = batch_ops::flatten_by_key(&new_by_key);
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
}
