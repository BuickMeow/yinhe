use super::*;

impl Document {
    /// 当前撤销栈的状态标记（发起异步保存时取，完成后原样传回）。
    pub fn undo_marker(&self) -> Option<u64> {
        self.history.marker()
    }

    /// 当前撤销栈长度（异步保存的版本快照用）。
    pub fn undo_past_len(&self) -> usize {
        self.history.past_len()
    }

    /// 捕获当前编辑状态快照（undo/redo 恢复用）。编辑**前**调用。
    pub fn capture_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            selected: self.edit.selected.clone(),
            track_selected: self.edit.track_selected.clone(),
            sel_rect: self.edit.sel_rect.clone(),
            arr_sel_rect: self.edit.arr_sel_rect.clone(),
            chan_ar_sel: self.edit.chan_ar_sel.clone(),
            anchor_sel_rects: self
                .edit
                .controller_panels
                .iter()
                .map(|p| p.anchor_sel_rects.clone())
                .collect(),
            anchor_members: self
                .edit
                .controller_panels
                .iter()
                .map(|p| p.anchor_members.clone())
                .collect(),
            arr_am_views: self.edit.arr_am_views.clone(),
            ppq: self.data.model.meta.ppq,
        }
    }

    /// 用编辑**前**快照 push 一个 undo entry。
    pub fn push_undo(&mut self, action: UndoAction, label: &str, before: EditSnapshot) {
        self.history.push(UndoEntry {
            action,
            label: label.to_string(),
            snapshot: before,
        });
    }

    /// 把快照恢复到 edit 状态（undo/redo 共用）。
    /// AM 选框/成员按面板索引对齐恢复，面板数量变化时防御跳过。
    /// PPQ 不同（undo/redo 跨过 PPQ rescale）时先按比例缩放快照里的 tick。
    fn restore_snapshot(edit: &mut EditState, snapshot: &EditSnapshot, current_ppq: u32) {
        let scaled;
        let snapshot = if snapshot.ppq != 0 && current_ppq != 0 && snapshot.ppq != current_ppq {
            scaled = scale_snapshot_ticks(snapshot, current_ppq as f64 / snapshot.ppq as f64);
            &scaled
        } else {
            snapshot
        };
        edit.selected = snapshot.selected.clone();
        edit.track_selected = snapshot.track_selected.clone();
        edit.sel_rect = snapshot.sel_rect.clone();
        edit.arr_sel_rect = snapshot.arr_sel_rect.clone();
        edit.chan_ar_sel = snapshot.chan_ar_sel.clone();
        for (i, panel) in edit.controller_panels.iter_mut().enumerate() {
            if let Some(rects) = snapshot.anchor_sel_rects.get(i) {
                panel.anchor_sel_rects = rects.clone();
            }
            if let Some(members) = snapshot.anchor_members.get(i) {
                panel.anchor_members = members.clone();
                // 成员由快照精确恢复，视为全部 rects 已物化。
                panel.materialized_anchor_rects = if members.is_some() {
                    panel.anchor_sel_rects.len()
                } else {
                    0
                };
            }
            if !panel.anchor_sel_rects.is_empty() {
                panel.dirty = true;
            }
        }
        edit.arr_am_views = snapshot.arr_am_views.clone();
    }

    /// PPQ rescale 后把当前编辑态里的 tick 按比例缩放（光标、各种选框、
    /// gate 筛选、循环区间）。临时绘制路径（钢笔/剪刀）直接清空。
    pub fn rescale_edit_ticks(&mut self, old_ppq: u32, new_ppq: u32) {
        if old_ppq == 0 || new_ppq == 0 || old_ppq == new_ppq {
            return;
        }
        let factor = new_ppq as f64 / old_ppq as f64;
        let tick_u32 = |v: u32| (v as f64 * factor).round() as u32;

        if let Some(t) = self.edit.cursor_tick {
            self.edit.cursor_tick = Some(t * factor);
        }
        for r in &mut self.edit.sel_rect.rects {
            r.0 *= factor;
            r.1 *= factor;
        }
        for r in &mut self.edit.selected.rects {
            r.0 = tick_u32(r.0);
            r.1 = tick_u32(r.1);
        }
        if let Some((g0, g1)) = self.edit.selected.filter.gate {
            self.edit.selected.filter.gate = Some((tick_u32(g0), tick_u32(g1)));
        }
        for r in &mut self.edit.arr_sel_rect {
            r.0 *= factor;
            r.1 *= factor;
        }
        for r in &mut self.edit.chan_ar_sel {
            r.t_start *= factor;
            r.t_end *= factor;
        }
        for panel in &mut self.edit.controller_panels {
            for r in &mut panel.anchor_sel_rects {
                r.tick_start *= factor;
                r.tick_end *= factor;
            }
            panel.dirty = true;
        }
        for view in self.edit.arr_am_views.values_mut() {
            for r in &mut view.anchor_sel_rects {
                r.tick_start *= factor;
                r.tick_end *= factor;
            }
            view.dirty = true;
        }
        // 临时绘制路径的 tick 语义已变，直接丢弃，避免落笔到错误位置。
        self.edit.pen_path = None;
        self.edit.scissors_line = None;
        self.edit.playback.rescale_ticks(factor);
    }

    /// 撤销栈内每步操作的标签（旧→新），供历史记录面板显示。
    pub fn undo_labels(&self) -> Vec<&str> {
        self.history.past_labels()
    }

    /// 重做栈内每步操作的标签（旧→新）。
    pub fn redo_labels(&self) -> Vec<&str> {
        self.history.future_labels()
    }

    /// Undo the most recent operation. Returns true if something was undone.
    pub fn undo(&mut self) -> bool {
        let Some((id, entry)) = self.history.take_past() else {
            return false;
        };

        // Save current selection so redo can restore it.
        let current = self.capture_snapshot();

        // 导入不保存源数据（1 亿音符不存两份），undo 后清空 redo 栈。
        let irreversible = matches!(&entry.action, UndoAction::ImportMidi { .. });

        // 反转 action（消耗 entry.action，零克隆），apply 一次后再 move 进 redo 栈。
        let reversed = entry.action.reversed();
        reversed.redo(self);

        // Restore selection from the undo entry.
        let current_ppq = self.data.model.meta.ppq;
        Self::restore_snapshot(&mut self.edit, &entry.snapshot, current_ppq);

        // Push reversed action onto the redo stack（沿用原状态 id：redo 后回到该状态）。
        self.history.put_future(
            id,
            UndoEntry {
                action: reversed,
                label: entry.label,
                snapshot: current,
            },
        );
        if irreversible {
            self.history.clear_future();
        }

        true
    }

    /// Redo the most recently undone operation. Returns true if something was redone.
    pub fn redo(&mut self) -> bool {
        let Some((id, entry)) = self.history.take_future() else {
            return false;
        };

        let current = self.capture_snapshot();

        // future 栈里存的是 reversed action，再反转一次回到原始方向并 apply。
        let reversed = entry.action.reversed();
        reversed.redo(self);

        // Restore selection from the redo entry.
        let current_ppq = self.data.model.meta.ppq;
        Self::restore_snapshot(&mut self.edit, &entry.snapshot, current_ppq);

        self.history.put_past(
            id,
            UndoEntry {
                action: reversed,
                label: entry.label,
                snapshot: current,
            },
        );

        true
    }

    /// 跳到撤销栈的指定长度（0 = 最初状态）：
    /// 向前/向后连续 undo / redo，**保留** future 栈，便于再点其它历史行跳转。
    /// 之后只要有新编辑，`push` 会照常清空 future，旧分支被覆盖。
    ///
    /// 返回是否有状态变化。不可逆操作（导入 MIDI）undo 时会清空 future，
    /// 此时循环自然停在能到达的位置。
    pub fn jump_history_to(&mut self, target_past_len: usize) -> bool {
        let mut changed = false;
        while self.history.past_len() > target_past_len {
            if !self.undo() {
                break;
            }
            changed = true;
        }
        while self.history.past_len() < target_past_len {
            if !self.redo() {
                break;
            }
            changed = true;
        }
        changed
    }
}

/// 按比例缩放快照里的所有 tick 字段（PPQ rescale 跨 undo/redo 用）。
fn scale_snapshot_ticks(snapshot: &EditSnapshot, factor: f64) -> EditSnapshot {
    fn tick_u32(v: u32, f: f64) -> u32 {
        (v as f64 * f).round() as u32
    }
    fn rect_f64(r: &mut (f64, f64, u8, u8), f: f64) {
        r.0 *= f;
        r.1 *= f;
    }
    let mut out = snapshot.clone();
    // 已换算成目标 PPQ，避免恢复时被再次缩放。
    out.ppq = 0;
    for r in &mut out.selected.rects {
        r.0 = tick_u32(r.0, factor);
        r.1 = tick_u32(r.1, factor);
    }
    if let Some((g0, g1)) = out.selected.filter.gate {
        out.selected.filter.gate = Some((tick_u32(g0, factor), tick_u32(g1, factor)));
    }
    for r in &mut out.sel_rect.rects {
        rect_f64(r, factor);
    }
    for r in &mut out.arr_sel_rect {
        r.0 *= factor;
        r.1 *= factor;
    }
    for r in &mut out.chan_ar_sel {
        r.t_start *= factor;
        r.t_end *= factor;
    }
    for rects in &mut out.anchor_sel_rects {
        for r in rects {
            r.tick_start *= factor;
            r.tick_end *= factor;
        }
    }
    for v in out.arr_am_views.values_mut() {
        for r in &mut v.anchor_sel_rects {
            r.tick_start *= factor;
            r.tick_end *= factor;
        }
    }
    out
}
