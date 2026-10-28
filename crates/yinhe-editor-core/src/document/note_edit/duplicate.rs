//! 复制：原地复制（源保留）与 Alt 拖动复制（选区跟随副本）。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

/// Riff 的纵向轴：PR 复制到不同 key，AR 复制到不同 track。
#[derive(Clone, Copy)]
enum FillAxis {
    Key,
    Track,
}

/// 带符号的 `[0, ±1, .., ±N]` 步序列。
///
/// 0 必须保留：二维 Riff 要铺满整个矩形（含只沿另一个轴移动的边），
/// 调用方跳过 `(0, 0)` 原位置即可。
fn step_offsets(steps: i64) -> Vec<i64> {
    let mut out = vec![0];
    if steps != 0 {
        out.extend((1..=steps.unsigned_abs()).map(|i| i as i64 * steps.signum()));
    }
    out
}

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
        self.repeat_selected_impl(width_ticks, steps, 1, 0, FillAxis::Key)
    }

    /// PR 二维 Riff：横向按 `width_ticks` 重复，纵向按 `key_span` 个 key 重复。
    ///
    /// `steps_x`/`steps_y` 带符号：x 正向右、负向左；y 正向上（key 增大）、
    /// 负向下（key 减小）。超出 0..=MAX_KEY 的副本按 key 截断，仍保留同批
    /// 其它有效 key 的副本。一次操作一个 undo。
    pub fn repeat_selected_keys(
        &mut self,
        width_ticks: i64,
        steps_x: i64,
        key_span: u16,
        steps_y: i64,
    ) -> Option<UndoAction> {
        self.repeat_selected_impl(width_ticks, steps_x, key_span, steps_y, FillAxis::Key)
    }

    /// AR 二维 Riff：横向按 `width_ticks` 重复，纵向按 `row_span` 条轨道重复。
    ///
    /// `steps_y` 正向下（track 下标增大）、负向上。目标轨不是 MIDI 轨
    ///（Conductor / 音频轨）时跳过该条副本；超出轨道范围同样截断。
    pub fn repeat_selected_tracks(
        &mut self,
        width_ticks: i64,
        steps_x: i64,
        row_span: u16,
        steps_y: i64,
    ) -> Option<UndoAction> {
        self.repeat_selected_impl(width_ticks, steps_x, row_span, steps_y, FillAxis::Track)
    }

    fn repeat_selected_impl(
        &mut self,
        width_ticks: i64,
        steps_x: i64,
        span: u16,
        steps_y: i64,
        axis: FillAxis,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        if steps_x == 0 && steps_y == 0 {
            return None;
        }
        if steps_x != 0 && width_ticks <= 0 {
            return None;
        }
        if steps_y != 0 && span == 0 {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let num_tracks = self.data.model.tracks.len();
        let xs = step_offsets(steps_x);
        let ys = step_offsets(steps_y);
        let after = {
            let model = &self.data.model;
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            batch_ops::for_each_selected(model, &self.edit.selected, |note, old_key| {
                for &sx in &xs {
                    for &sy in &ys {
                        if sx == 0 && sy == 0 {
                            continue;
                        }
                        let delta = sx.saturating_mul(width_ticks);
                        let new_start = (i64::from(note.start_tick) + delta).max(0) as u32;
                        let length = note.end_tick - note.start_tick;
                        let (new_key, new_track) = match axis {
                            FillAxis::Key => {
                                let k = i64::from(old_key) + sy.saturating_mul(i64::from(span));
                                if !(0..=i64::from(yinhe_types::MAX_KEY)).contains(&k) {
                                    continue;
                                }
                                (k as u8, note.track)
                            }
                            FillAxis::Track => {
                                let t = i64::from(note.track) + sy.saturating_mul(i64::from(span));
                                if t < 0 || t as usize >= num_tracks {
                                    continue;
                                }
                                let track = &model.tracks[t as usize];
                                if track.kind != yinhe_core::TrackKind::Midi {
                                    continue;
                                }
                                (old_key, t as u16)
                            }
                        };
                        if !allow_overlap
                            && batch_ops::has_overlapping_note(
                                model,
                                new_track,
                                new_key,
                                new_start,
                                new_start + length,
                            )
                        {
                            continue;
                        }
                        new_by_key
                            .entry(new_key)
                            .or_default()
                            .push(yinhe_types::Note {
                                id: 0,
                                start_tick: new_start,
                                end_tick: new_start + length,
                                velocity: note.velocity,
                                track: new_track,
                            });
                    }
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
