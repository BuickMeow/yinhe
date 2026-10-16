//! 工具批量操作：网格切割、直线/画笔生成、剪刀切割。

use std::sync::Arc;

use yinhe_core::NoteEvent;

use crate::batch_ops;
use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

impl Document {
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

    /// 网格切割（操作栏按钮）：把选框内（含边界）的音符按当前量化网格切开。
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
        if Some(track) == self.edit.track_cache.conductor_idx {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let after: Vec<(yinhe_types::Note, u8)>;
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
            after = batch_ops::flatten_by_key(&by_key);
            batch_ops::insert_batch(model, by_key);
        }
        self.data.rebuild_model_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// 钢笔工具确认：曲线每经过一个整数音高行生成一个音符（gate = 一个量化
    /// 间隔），非单调曲线同一行可出现多个音符。
    ///
    /// start = 曲线与该行交点的 tick 吸附量化（含小节感知）；力度 = 该轨记忆
    /// 力度。目标轨 = 主音轨，无主音轨/Conductor 时返回 None。
    pub fn generate_pen_notes(&mut self) -> Option<UndoAction> {
        let path = self.edit.pen_path.clone()?;
        let track = self.edit.main_track()?;
        if Some(track) == self.edit.track_cache.conductor_idx {
            return None;
        }
        let ppq = self.data.model.meta.ppq;
        let quantize = self.edit.quantize_pianoroll;
        let (tpb, num, den, events) = pr_bar_line_data(&self.data.model);
        let bar = Some((tpb, num, den, events.as_slice()));
        let velocity = self.edit.default_velocity(track);
        let notes: Vec<NoteEvent> = crate::pen::note_slots(&path, quantize, ppq, bar)
            .into_iter()
            .map(|(key, start, end)| NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: end,
                key,
                velocity,
            })
            .collect();
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
