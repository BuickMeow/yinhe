//! 黑乐谱清理：去重、框选填缝、框选合并（均为确定性整段修正）。

use std::sync::Arc;

use crate::document::Document;
use crate::history::{NoteDelta, UndoAction};

impl Document {
    /// 一键为整首歌去重重叠音符（用于黑乐谱叠音清理）。
    ///
    /// - `cross_track == false`：仅同一 `(track, key)` 内 `[start,end)` 相交的视为重叠，保留最早的，删后者；
    /// - `cross_track == true`：同一 `key` 下跨轨也视为重叠（全局 per-key 去重）。
    ///
    /// 已按 `start_tick` 有序的桶用 `retain_mut` 单趟扫描 `O(N)`；无重叠的桶
    /// 不重建、不标脏。对 1 亿音符零临时 `Vec`（旧实现每桶物化一份全桶）。
    pub fn dedup_overlapping_notes(&mut self, cross_track: bool) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let mut removed: Vec<(yinhe_types::Note, u8)> = Vec::new();
        for key in 0u8..=yinhe_types::MAX_KEY {
            let bucket = Arc::make_mut(&mut model.notes[key as usize]);
            if bucket.is_empty() {
                continue;
            }
            let len_before = bucket.len();
            if cross_track {
                let mut last_end: Option<u32> = None;
                bucket.retain_mut(|n| {
                    if let Some(le) = last_end
                        && n.start_tick < le
                    {
                        removed.push((*n, key));
                        return false;
                    }
                    last_end = Some(n.end_tick);
                    true
                });
            } else {
                use std::collections::HashMap;
                let mut last_per_track: HashMap<u16, u32> = HashMap::new();
                bucket.retain_mut(|n| {
                    if let Some(&le) = last_per_track.get(&n.track)
                        && n.start_tick < le
                    {
                        removed.push((*n, key));
                        return false;
                    }
                    // 同轨下一个音符的起点必须 >= 本音符终点才保留
                    last_per_track.insert(n.track, n.end_tick);
                    true
                });
            }
            if bucket.len() != len_before {
                model.mark_dirty(key);
            }
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

    /// 框选填缝：按**轨道**分组，把选中音符的终点贴合到该轨道「下一个不同的起点」。
    ///
    /// - 同一轨道内所有选中音符的起点去重升序为 `s₁ < … < sₖ`：起点为 `sᵢ`
    ///   （i < k）的音符 `end_tick → sᵢ₊₁`；起点为 `sₖ` 的音符保持原终点；
    /// - 只改终点（起点/key/轨道/力度/id 不动），顺带消除「起始不同却重叠」
    ///   与「同起点终点不一致」；框外音符不参与（既不被改、也不作为贴合目标）；
    /// - 被属性筛选排除的音符视同未选中；
    /// - 详情见 `docs/spec-note-fill-merge.md`。返回 `None` 表示无变化。
    pub fn fill_gaps_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }

        // Phase 1（只读）：按轨道收集选中音符并计算新终点，同时生成 undo
        // delta 与「id → 新终点」写回计划（Phase 2 不再读模型）。
        let mut by_track: std::collections::HashMap<u16, Vec<(yinhe_types::Note, u8)>> =
            std::collections::HashMap::new();
        {
            let model = &self.data.model;
            let selection = &self.edit.selected;
            for (k, ranges) in selection.merged_tick_ranges_by_key().iter().enumerate() {
                if ranges.is_empty() {
                    continue;
                }
                let key = k as u8;
                for &(lo, hi) in ranges {
                    for n in model.notes[k].range(lo, hi) {
                        if !selection.accepts_note(n, key) {
                            continue;
                        }
                        by_track.entry(n.track).or_default().push((*n, key));
                    }
                }
            }
        }

        let mut before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        // key → (note id → 新终点)
        let mut updates: std::collections::HashMap<u8, std::collections::HashMap<u32, u32>> =
            std::collections::HashMap::new();
        for notes in by_track.values_mut() {
            notes.sort_unstable_by_key(|(n, _)| n.start_tick);
            // 去重起点序列（notes 已按起点升序）。
            let mut starts: Vec<u32> = Vec::with_capacity(notes.len());
            for (n, _) in notes.iter() {
                if starts.last() != Some(&n.start_tick) {
                    starts.push(n.start_tick);
                }
            }
            if starts.len() < 2 {
                continue; // 只有一个起点：没有「下一个起点」可贴合
            }
            let mut idx = 0usize;
            for (n, key) in notes.iter() {
                while idx < starts.len() && starts[idx] < n.start_tick {
                    idx += 1;
                }
                let Some(&next) = starts.get(idx + 1) else {
                    continue; // 最后一个起点：保持原终点
                };
                if n.end_tick == next {
                    continue;
                }
                let mut updated = *n;
                updated.end_tick = next;
                before.push((*n, *key));
                after.push((updated, *key));
                updates.entry(*key).or_default().insert(n.id, next);
            }
        }
        if after.is_empty() {
            return None;
        }

        // Phase 2（写回）：只改终点、不换桶；`update_matching` 仅深拷贝命中块。
        let model = Arc::make_mut(&mut self.data.model);
        apply_end_ticks(model, &updates);
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }

    /// 框选合并：同一轨道 + 同一 key、首尾严格相接（`end == 下一个 start`）
    /// 的连续音符链压成一个音符。
    ///
    /// - 保留链首音符的 id/力度/轨道，终点取链尾终点，删除链内其余音符；
    /// - 1 tick 空隙或重叠都断开；链上所有音符都必须被框选（被属性筛选
    ///   排除的音符视同未选中会断开链）；框外音符不参与判定；
    /// - 详情见 `docs/spec-note-fill-merge.md`。返回 `None` 表示无可合并链。
    pub fn merge_adjacent_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }

        // Phase 1（只读）：逐 (key, track) 扫描选框内音符，找最大相邻链并
        // 生成 undo delta 与写回计划（Phase 2 不再读模型）。
        let mut before: Vec<(yinhe_types::Note, u8)> = Vec::new();
        let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
        // key → (链首 id → 合并后终点)
        let mut head_updates: std::collections::HashMap<u8, std::collections::HashMap<u32, u32>> =
            std::collections::HashMap::new();
        // key → 被合并掉的 id 集合
        let mut to_remove: std::collections::HashMap<u8, std::collections::HashSet<u32>> =
            std::collections::HashMap::new();
        {
            let model = &self.data.model;
            let selection = &self.edit.selected;
            for (k, ranges) in selection.merged_tick_ranges_by_key().iter().enumerate() {
                if ranges.is_empty() {
                    continue;
                }
                let key = k as u8;
                // 按轨道分组（同一 key 桶内跨轨音符交错；桶内已按起点排序）。
                let mut per_track: std::collections::HashMap<u16, Vec<(yinhe_types::Note, bool)>> =
                    std::collections::HashMap::new();
                for &(lo, hi) in ranges {
                    for n in model.notes[k].range(lo, hi) {
                        let selected = selection.accepts_note(n, key);
                        per_track.entry(n.track).or_default().push((*n, selected));
                    }
                }
                for notes in per_track.values() {
                    let mut run: Vec<yinhe_types::Note> = Vec::new();
                    for (n, selected) in notes {
                        let adjacent = run.last().is_some_and(|prev| prev.end_tick == n.start_tick);
                        if *selected && adjacent {
                            run.push(*n);
                            continue;
                        }
                        if run.len() >= 2 {
                            record_merge_chain(
                                &mut run,
                                key,
                                &mut before,
                                &mut after,
                                &mut head_updates,
                                &mut to_remove,
                            );
                        } else {
                            run.clear();
                        }
                        if *selected {
                            run.push(*n);
                        }
                    }
                    if run.len() >= 2 {
                        record_merge_chain(
                            &mut run,
                            key,
                            &mut before,
                            &mut after,
                            &mut head_updates,
                            &mut to_remove,
                        );
                    }
                }
            }
        }
        if after.is_empty() {
            return None;
        }

        // Phase 2（写回）：链首改终点 + 删除链内其余音符；只触达命中的 key 桶。
        let model = Arc::make_mut(&mut self.data.model);
        apply_end_ticks(model, &head_updates);
        for (key, ids) in to_remove {
            let bucket = Arc::make_mut(&mut model.notes[key as usize]);
            bucket.remove_by_ids(&ids);
            model.mark_dirty(key);
        }
        model.rebuild_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }
}

/// 批量改终点（按 key 分组的 `id → 新终点` 计划）：`update_matching` 只深拷贝
/// 命中块。`end_tick` 不参与排序键，改完无需重排。填缝/合并共用。
fn apply_end_ticks(
    model: &mut yinhe_core::YinModel,
    updates: &std::collections::HashMap<u8, std::collections::HashMap<u32, u32>>,
) {
    for (&key, plan) in updates {
        let bucket = Arc::make_mut(&mut model.notes[key as usize]);
        bucket.update_matching(
            |n| plan.contains_key(&n.id),
            |n| {
                if let Some(&new_end) = plan.get(&n.id) {
                    n.end_tick = new_end;
                }
            },
        );
        model.mark_dirty(key);
    }
}

/// 把一条（长度 ≥ 2 的）相邻音符链登记进合并计划：
/// undo delta（`before` = 链上全部原音符，`after` = 合并结果）+ 写回计划。
///
/// 合并结果保留链首 id/力度/轨道，终点取链尾 `end_tick`；链尾之后的音符全部删除。
fn record_merge_chain(
    chain: &mut Vec<yinhe_types::Note>,
    key: u8,
    before: &mut Vec<(yinhe_types::Note, u8)>,
    after: &mut Vec<(yinhe_types::Note, u8)>,
    head_updates: &mut std::collections::HashMap<u8, std::collections::HashMap<u32, u32>>,
    to_remove: &mut std::collections::HashMap<u8, std::collections::HashSet<u32>>,
) {
    if chain.len() < 2 {
        chain.clear();
        return;
    }
    let Some(last_end) = chain.last().map(|n| n.end_tick) else {
        chain.clear();
        return;
    };
    let mut merged = chain[0];
    merged.end_tick = last_end;
    head_updates
        .entry(key)
        .or_default()
        .insert(merged.id, last_end);
    let removals = to_remove.entry(key).or_default();
    for n in &chain[1..] {
        removals.insert(n.id);
    }
    before.extend(chain.iter().map(|n| (*n, key)));
    after.push((merged, key));
    chain.clear();
}
