//! YinModel 的统计缓存与重建逻辑。
//!
//! 从 model.rs 拆分出来，避免 model.rs 过长。所有方法都是
//! `impl YinModel` 的扩展，访问相同的字段。

use std::collections::HashMap;
use std::sync::Arc;

use yinhe_types::{KEY_COUNT, Note, NoteBucket};

use crate::events::NoteEvent;

use super::YinModel;

impl YinModel {
    /// Distribute per-track notes into the by-key store.
    ///
    /// Called once during parsing. After this, `TrackData` no longer
    /// holds notes — the by-key `self.notes` is the single source.
    /// Also computes `note_count`, `tick_length`, and `track_note_count`
    /// in the same pass (avoids a second full scan in `rebuild()`).
    ///
    /// 音符 id 分配：输入 NoteEvent.id == 0 表示未分配（MIDI 解析路径），
    /// 由本方法从 `next_note_id` 起顺序发号；非 0 表示外部已分配（旧 .yin 加载），
    /// 保留原 id 并推进 `next_note_id` 到 max+1。
    pub fn load_track_notes(&mut self, per_track_notes: Vec<Vec<NoteEvent>>) {
        // Count per key for exact allocation.
        let mut per_key_count = [0u32; KEY_COUNT];
        for notes in per_track_notes.iter() {
            for note in notes {
                per_key_count[note.key as usize] += 1;
            }
        }

        let mut loader = NoteLoader::new(self.tracks.len(), self.next_note_id, per_key_count);
        for (track_idx, notes) in per_track_notes.into_iter().enumerate() {
            for note in notes {
                loader.feed(
                    note.key,
                    track_idx as u16,
                    note.start_tick,
                    note.end_tick,
                    note.velocity,
                    note.id,
                );
            }
        }
        loader.finish(self);
    }

    /// 分配一个新的全局唯一音符 id。编辑路径（新增/粘贴/复制）调用。
    pub fn alloc_note_id(&mut self) -> u32 {
        let id = self.next_note_id;
        self.next_note_id = self.next_note_id.wrapping_add(1);
        id
    }

    /// 带上限检查的 id 分配。到 `u32::MAX` 返回 `None`，不再回绕。
    pub fn try_alloc_note_id(&mut self) -> Option<u32> {
        let id = self.next_note_id;
        if id == u32::MAX {
            return None;
        }
        self.next_note_id = id.checked_add(1)?;
        Some(id)
    }

    /// 单音符编辑的统计增量：用 before/after 直接更新计数与 tick_length，
    /// 不扫描桶、不设置 `dirty_keys`、不触发 `rebuild_dirty()`。
    ///
    /// `before` / `after` 必须是模型里真实存在的旧/新状态；调用方负责在
    /// bucket 已经改完后调用。跨 key 移动时两个参数可以带不同的 key。
    pub fn apply_note_stat_delta(
        &mut self,
        before: Option<(u8, &yinhe_types::Note)>,
        after: Option<(u8, &yinhe_types::Note)>,
    ) {
        let prev_tick_length = self.tick_length;

        if let Some((key, n)) = before {
            let k = key as usize;
            self.note_count = self.note_count.saturating_sub(1);
            self.bucket_note_count[k] = self.bucket_note_count[k].saturating_sub(1);
            let remove_entry = if let Some(e) = self.bucket_track_stats[k].get_mut(&n.track) {
                e.0 = e.0.saturating_sub(1);
                if n.velocity > 1 {
                    e.1 = e.1.saturating_sub(1);
                }
                e.0 == 0
            } else {
                false
            };
            if remove_entry {
                self.bucket_track_stats[k].remove(&n.track);
            }
            if let Some(t) = self.track_note_count.get_mut(n.track as usize) {
                *t = t.saturating_sub(1);
            }
            if n.velocity > 1
                && let Some(a) = self.track_audible_count.get_mut(n.track as usize)
            {
                *a = a.saturating_sub(1);
            }
        }

        if let Some((key, n)) = after {
            let k = key as usize;
            self.note_count = self.note_count.saturating_add(1);
            self.bucket_note_count[k] = self.bucket_note_count[k].saturating_add(1);
            let e = self.bucket_track_stats[k].entry(n.track).or_insert((0, 0));
            e.0 = e.0.saturating_add(1);
            if n.velocity > 1 {
                e.1 = e.1.saturating_add(1);
            }
            if let Some(t) = self.track_note_count.get_mut(n.track as usize) {
                *t = t.saturating_add(1);
            }
            if n.velocity > 1
                && let Some(a) = self.track_audible_count.get_mut(n.track as usize)
            {
                *a = a.saturating_add(1);
            }
        }

        // tick_length 只可能因「移除了当前最大值」或「新增了更大的 end」变化。
        let removed_max = before.is_some_and(|(_, n)| n.end_tick as u64 >= prev_tick_length);
        let added_max = after.is_some_and(|(_, n)| n.end_tick as u64 > prev_tick_length);
        if removed_max || added_max {
            self.tick_length = self
                .notes
                .iter()
                .map(|b| b.max_end_tick() as u64)
                .max()
                .unwrap_or(0);
            if self.tick_length != prev_tick_length {
                self.tempo_map = Arc::new(self.build_tempo_map());
            }
        }
    }

    /// Mark a bucket as dirty (modified and needs sorting).
    /// Call this before or after modifying `self.notes[key]`.
    /// Also bumps `note_revisions[key]` for incremental GPU upload tracking.
    pub fn mark_dirty(&mut self, key: u8) {
        self.dirty_keys[key as usize] = true;
        self.note_revisions[key as usize] = self.note_revisions[key as usize].wrapping_add(1);
    }

    /// Rebuild all derived data from scratch.
    ///
    /// Call this after any mutation that changes notes, conductor, or
    /// track structure. O(N) where N = total note count.
    ///
    /// This operates on `self.notes` (the by-key store) directly — no
    /// longer reads from `TrackData.notes`.
    ///
    /// Note: `note_count` and `track_note_count` are maintained by
    /// `load_track_notes` and by edit operations. `rebuild()` only
    /// sorts buckets and rebuilds indices.
    pub fn rebuild(&mut self) {
        // Sort + re-chunk all KEY_COUNT buckets in parallel.
        use rayon::prelude::*;
        self.notes.par_iter_mut().for_each(|bucket| {
            Arc::make_mut(bucket).sort();
        });
        self.rebuild_stats_only();
    }

    /// 全量 bump 每个 key 的 revision，不做排序/统计。
    ///
    /// 用于 `NoteLoader::finish` 之后：loader 已经写入完整统计与有序桶，
    /// 只需要让渲染/缓存消费者知道所有 key 都换了内容。
    pub fn bump_all_note_revisions(&mut self) {
        for r in &mut self.note_revisions {
            *r = r.wrapping_add(1);
        }
    }

    /// 按当前 conductor 重建 tempo map（时间轴/跳转区间）。
    ///
    /// `NoteLoader::finish` 只写音符与统计，不碰 tempo map；加载路径
    /// 在统计已完整时调用本方法即可，不必跑全量 `rebuild_stats_only`。
    pub fn refresh_tempo_map(&mut self) {
        self.tempo_map = Arc::new(self.build_tempo_map());
    }

    /// 只重建统计与 revision，不排序。
    ///
    /// 用于不破坏桶内 `start_tick` 顺序的结构变更（轨道重映射、轨号平移等：
    /// track 不是排序键，`retain_mut` 保持有序）。比 [`YinModel::rebuild`]
    /// 省掉 1.64 亿音符级别的全桶排序。
    pub fn rebuild_stats_only(&mut self) {
        // Bump all note_revisions (full rebuild = all keys changed).
        for r in &mut self.note_revisions {
            *r = r.wrapping_add(1);
        }

        // Recompute note_count, max_tick, track_note_count, track_audible_count
        // (may have changed after edits or track insertions).
        let mut note_count: u64 = 0;
        let mut max_tick: u64 = 0;
        let mut track_counts: Vec<u64> = vec![0u64; self.tracks.len()];
        let mut track_audible: Vec<u64> = vec![0u64; self.tracks.len()];
        // Per-bucket per-track stats — recomputed in the same pass so
        // rebuild_dirty() can do incremental updates later.
        let mut bucket_stats: [HashMap<u16, (u64, u64)>; KEY_COUNT] =
            core::array::from_fn(|_| HashMap::new());
        for (k, bucket) in self.notes.iter().enumerate() {
            note_count += bucket.len() as u64;
            for n in bucket.iter() {
                let end = n.end_tick as u64;
                if end > max_tick {
                    max_tick = end;
                }
                if (n.track as usize) < track_counts.len() {
                    track_counts[n.track as usize] += 1;
                    if n.velocity > 1 {
                        track_audible[n.track as usize] += 1;
                    }
                }
                let e = bucket_stats[k].entry(n.track).or_insert((0, 0));
                e.0 += 1;
                if n.velocity > 1 {
                    e.1 += 1;
                }
            }
        }
        self.note_count = note_count;
        self.tick_length = max_tick;
        self.track_note_count = track_counts;
        self.track_audible_count = track_audible;
        let notes = &self.notes;
        for (k, (count, stats)) in self
            .bucket_note_count
            .iter_mut()
            .zip(self.bucket_track_stats.iter_mut())
            .enumerate()
        {
            *count = notes[k].len() as u64;
            *stats = std::mem::take(&mut bucket_stats[k]);
        }

        // Rebuild tempo_map (depends on tick_length we just computed).
        self.tempo_map = Arc::new(self.build_tempo_map());
    }

    /// 按 `remap` 搬移轨道级统计（`remap[旧轨] = 新轨`，`u16::MAX` = 删除）。
    ///
    /// 仅适用于**纯重排/中间插入**场景：音符总数、`tick_length`、
    /// 每桶计数与块级 max_end 都不变（remap 不改音符的 tick）。零音符遍历，
    /// 成本 O(轨道数 + KEY_COUNT × 桶内轨道数)。
    ///
    /// 删除轨（remap 含 `u16::MAX`）不要用本方法，请走
    /// [`YinModel::rebuild_stats_only`]（note_count/tick_length 需要重算）。
    pub fn remap_track_stats(&mut self, remap: &[u16]) {
        // 新轨数 = 最大新轨号 + 1（中间插入时新轨槽位在 remap 中无旧轨对应）。
        let new_len = remap
            .iter()
            .filter(|&&v| v != u16::MAX)
            .max()
            .map_or(0, |&m| m as usize + 1);
        let mut counts = vec![0u64; new_len];
        let mut audible = vec![0u64; new_len];
        for (old, &new) in remap.iter().enumerate() {
            if new == u16::MAX {
                continue;
            }
            counts[new as usize] += self.track_note_count.get(old).copied().unwrap_or(0);
            audible[new as usize] += self.track_audible_count.get(old).copied().unwrap_or(0);
        }
        self.track_note_count = counts;
        self.track_audible_count = audible;
        for k in 0..KEY_COUNT {
            let mut new_map = HashMap::with_capacity(self.bucket_track_stats[k].len());
            for (&old, &(count, aud)) in &self.bucket_track_stats[k] {
                let Some(&new) = remap.get(old as usize) else {
                    continue;
                };
                if new == u16::MAX {
                    continue;
                }
                let e = new_map.entry(new).or_insert((0, 0));
                e.0 += count;
                e.1 += aud;
            }
            self.bucket_track_stats[k] = new_map;
        }
    }

    /// 末尾追加 `n` 条空轨的统计槽位（新轨无音符，无需任何音符遍历）。
    ///
    /// 调用方必须已把轨道 push 到 `tracks` 末尾（`add_tracks_batch` 场景）。
    pub fn push_empty_track_stats(&mut self, n: usize) {
        self.track_note_count
            .resize(self.track_note_count.len() + n, 0);
        self.track_audible_count
            .resize(self.track_audible_count.len() + n, 0);
    }

    /// Rebuild statistics for the dirty buckets incrementally.
    ///
    /// Cost: O(sum of dirty bucket sizes) for stats + O(D) bookkeeping.
    /// For a 30M-note song where only 10 buckets were touched, this is
    /// ~O(10 bucket scans) instead of O(KEY_COUNT bucket sorts + clones).
    ///
    /// **No sorting happens here.** All write paths guarantee the
    /// per-key bucket stays sorted by `start_tick` themselves:
    /// - single-note paths insert at the `partition_point` position
    /// - `batch_ops::append_notes_ordered` merges batch inserts
    /// - in-place field edits never touch the sort key (velocity, end_tick)
    ///
    /// `rebuild()` still sorts — it is the full-rebuild fallback for loads,
    /// track-structure changes, and PPQ rescale. If a bucket ever becomes
    /// unsorted, every `partition_point` consumer silently misbehaves, so
    /// keep the invariant in mind when adding new write paths.
    ///
    /// Statistics are updated incrementally using `bucket_note_count`:
    /// subtract old counts for dirty buckets, then rescan only dirty
    /// buckets and add back new counts. Track-level stats still do a
    /// full scan of all buckets — this is a future optimization.
    ///
    /// tempo_map 重建：仅当 `tick_length` 变化时重建。
    /// conductor 变更由 `rebuild_tempo_map` 单独处理（不经过 rebuild_dirty），
    /// 所以 rebuild_dirty 路径只需关心 tick_length 是否变了——tempo_map
    /// 内部缓存了 tick_length 字段，需要同步。
    pub fn rebuild_dirty(&mut self) {
        self.rebuild_dirty_with_progress(|_, _| {});
    }

    /// 同 [`rebuild_dirty`](Self::rebuild_dirty)，按脏 key 内**当前音符数**
    /// 回报 `(done, total)`。批量导入一次弄脏所有 key 时，UI 不再在
    /// 100% 后继续静默重扫。
    pub fn rebuild_dirty_with_progress(&mut self, mut on_notes: impl FnMut(u64, u64)) {
        let dirty_indices: Vec<usize> = (0..KEY_COUNT).filter(|&k| self.dirty_keys[k]).collect();
        if dirty_indices.is_empty() {
            return;
        }
        let dirty_total: u64 = dirty_indices
            .iter()
            .map(|&k| self.notes[k].len() as u64)
            .sum();
        let mut dirty_done = 0u64;

        let prev_tick_length = self.tick_length;
        self.dirty_keys = [false; KEY_COUNT];
        let mut delta_note_count: i64 = 0;
        for &k in &dirty_indices {
            let old = self.bucket_note_count[k] as i64;
            let new = self.notes[k].len() as i64;
            delta_note_count += new - old;
            self.bucket_note_count[k] = new as u64;
        }
        self.note_count = (self.note_count as i64 + delta_note_count) as u64;
        // tick_length 直接取每桶块级 max_ends 的最大值；不维护第二份
        // per-key 缓存。O(总块数)，且删除/缩短最后一个音符时能正确收缩。
        self.tick_length = self
            .notes
            .iter()
            .map(|b| b.max_end_tick() as u64)
            .max()
            .unwrap_or(0);

        // 3. Incremental track stats: subtract old per-track contributions
        //    for each dirty bucket, recompute the bucket's stats, and add
        //    the new contributions back. O(dirty bucket size) per edit
        //    instead of O(total notes).
        for &k in &dirty_indices {
            // Subtract old contributions from per-track totals.
            for (&track, &(total, audible)) in &self.bucket_track_stats[k] {
                if let Some(t) = self.track_note_count.get_mut(track as usize) {
                    *t = t.saturating_sub(total);
                }
                if let Some(a) = self.track_audible_count.get_mut(track as usize) {
                    *a = a.saturating_sub(audible);
                }
            }

            // Recompute this bucket's per-track stats from current notes.
            let mut new_stats: HashMap<u16, (u64, u64)> = HashMap::new();
            for n in self.notes[k].iter() {
                let e = new_stats.entry(n.track).or_insert((0, 0));
                e.0 += 1;
                if n.velocity > 1 {
                    e.1 += 1;
                }
            }

            // Add new contributions back to per-track totals.
            for (&track, &(total, audible)) in &new_stats {
                if let Some(t) = self.track_note_count.get_mut(track as usize) {
                    *t += total;
                }
                if let Some(a) = self.track_audible_count.get_mut(track as usize) {
                    *a += audible;
                }
            }
            self.bucket_track_stats[k] = new_stats;
            dirty_done += self.notes[k].len() as u64;
            on_notes(dirty_done, dirty_total);
        }

        // 4. Rebuild tempo_map only if tick_length changed.
        //    rebuild_dirty 路径不动 conductor（tempo/time_sig），所以
        //    tempo_map 的 tempo_segments / time_sig_events 不变；
        //    只有 tick_length 字段可能需要同步。
        if self.tick_length != prev_tick_length {
            self.tempo_map = Arc::new(self.build_tempo_map());
        }
    }

    /// Change PPQ and rescale all tick data to preserve absolute timing.
    ///
    /// Scales every tick-bearing field (notes, automation events, tempo events,
    /// time signature events, program changes) by `new_ppq / old_ppq`.
    /// For integer ratios (e.g. 480→960, ×2) the result is exact; for
    /// non-integer ratios rounding may introduce sub-tick discrepancies.
    ///
    /// Sets `meta.ppq = new_ppq` and calls `rebuild()` to recompute derived
    /// data (tempo_map, tick_length, statistics).
    ///
    /// O(N) where N = total notes + automation events. Triggers `Arc::make_mut`
    /// deep-clones on every bucket that is shared with an undo snapshot.
    ///
    /// 同步版本：在主线程调用，无进度报告。适用于音符数较少或已知不会卡顿的场景。
    /// 大工程（百万级音符）应使用 [`Self::rescale_ppq_with_progress`] 在子线程执行。
    pub fn rescale_ppq(&mut self, new_ppq: u32) {
        let old_ppq = self.meta.ppq;
        if old_ppq == new_ppq || old_ppq == 0 {
            return;
        }
        // 唯一一份 tick 字段实现（含 key_sig / markers / lyrics / chord），
        // 见 `model_transform.rs`。这里只负责最后重建派生数据。
        self.rescale_and_offset_ticks(new_ppq, 0);
        self.rebuild();
    }

    /// 异步版本 of [`Self::rescale_ppq`]：在子线程中执行，带进度报告和取消支持。
    ///
    /// 调用方传入 model 的 clone（不修改原 model），返回 rescale 后的新 model。
    /// 若 `cancel` 在处理过程中被设为 true，提前返回 `Err("已取消")`。
    ///
    /// 进度报告：
    /// - 阶段 1（0..90%）：缩放 KEY_COUNT 个音符 bucket，每个 bucket 完成后更新进度
    /// - 阶段 2（90..95%）：缩放 conductor（tempo + time_sig）
    /// - 阶段 3（95..100%）：缩放 track automation + PC + rebuild
    ///
    /// rayon 并行处理音符 bucket，但 cancel 只能让未开始的任务跳过，
    /// 正在执行的 bucket 不能中断（最坏情况需等一个 bucket sort 完成）。
    pub fn rescale_ppq_with_progress(
        mut model: YinModel,
        new_ppq: u32,
        progress: std::sync::Arc<std::sync::Mutex<RescaleProgress>>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        pause: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<YinModel, String> {
        use std::sync::atomic::{AtomicU32, Ordering};

        let old_ppq = model.meta.ppq;
        if old_ppq == new_ppq || old_ppq == 0 {
            return Ok(model);
        }
        let scale = new_ppq as f64 / old_ppq as f64;
        let scale_tick = |t: u32| -> u32 {
            let v = (t as f64 * scale).round();
            if v > u32::MAX as f64 {
                u32::MAX
            } else {
                v as u32
            }
        };

        // ── 阶段 1：缩放音符（90%）──
        // rayon 并行 + AtomicU32 计数已完成 bucket 数。
        // cancel 检测在每个 bucket 开始前；正在跑的不能中断。
        use rayon::prelude::*;
        let done = std::sync::Arc::new(AtomicU32::new(0));
        let weights: Vec<u64> = model.notes.iter().map(|b| b.len() as u64).collect();
        let total_notes: u64 = weights.iter().sum();
        let notes_done = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let progress_clone = progress.clone();
        let cancel_clone = cancel.clone();
        let pause_clone = pause.clone();
        model
            .notes
            .par_iter_mut()
            .enumerate()
            .for_each(|(key, bucket)| {
                while pause_clone.load(Ordering::Relaxed) {
                    if cancel_clone.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if cancel_clone.load(Ordering::Relaxed) {
                    return;
                }
                let bucket = Arc::make_mut(bucket);
                for n in bucket.iter_mut() {
                    n.start_tick = scale_tick(n.start_tick);
                    n.end_tick = scale_tick(n.end_tick);
                    if n.end_tick <= n.start_tick {
                        n.end_tick = n.start_tick.saturating_add(1);
                    }
                }
                bucket.sort();
                bucket.rebuild_max_ends();
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                let n = notes_done.fetch_add(weights[key], Ordering::Relaxed) + weights[key];
                if let Ok(mut p) = progress_clone.lock() {
                    p.progress = n as f32 / total_notes.max(1) as f32 * 0.9;
                    p.label = format!(
                        "缩放音符 {}/{total_notes}（key {}/{KEY_COUNT}）",
                        n.min(total_notes),
                        d
                    );
                }
            });
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".to_string());
        }

        while pause.load(Ordering::Relaxed) {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".to_string());
        }

        // ── 阶段 2：缩放 conductor（95%）──
        {
            let conductor = Arc::make_mut(&mut model.conductor);
            for ev in conductor.tempo.events.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            conductor.tempo.events.sort_by_key(|e| e.tick);
            for ts in conductor.time_sig.iter_mut() {
                ts.tick = scale_tick(ts.tick);
            }
            conductor.time_sig.sort_by_key(|e| e.tick);
            for ev in conductor.key_sig.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            conductor.key_sig.sort_by_key(|e| e.tick);
            for ev in conductor.markers.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            conductor.markers.sort_by_key(|e| e.tick);
            for ev in conductor.lyrics.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            conductor.lyrics.sort_by_key(|e| e.tick);
            for ev in conductor.chord.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            conductor.chord.sort_by_key(|e| e.tick);
        }
        if let Ok(mut p) = progress.lock() {
            p.progress = 0.95;
            p.label = "缩放 conductor".to_string();
        }

        while pause.load(Ordering::Relaxed) {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".to_string());
        }

        // ── 阶段 3：缩放 track automation + PC（95..99%）──
        let total_events: u64 = model
            .tracks
            .iter()
            .map(|t| {
                t.automation_lanes
                    .iter()
                    .map(|l| l.events.len() as u64)
                    .sum::<u64>()
                    + t.program_change.len() as u64
                    + t.lyrics.len() as u64
                    + t.chord.len() as u64
            })
            .sum();
        let mut events_done = 0u64;
        for track in model.tracks.iter_mut() {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".to_string());
            }
            let track = Arc::make_mut(track);
            for lane in track.automation_lanes.iter_mut() {
                for ev in lane.events.iter_mut() {
                    ev.tick = scale_tick(ev.tick);
                }
                lane.events.sort_by_key(|e| e.tick);
                events_done += lane.events.len() as u64;
                if let Ok(mut p) = progress.lock() {
                    p.progress = 0.95 + events_done as f32 / total_events.max(1) as f32 * 0.04;
                    p.label = format!("缩放自动化 {events_done}/{total_events}");
                }
            }
            for pc in track.program_change.iter_mut() {
                pc.tick = scale_tick(pc.tick);
            }
            track.program_change.sort_by_key(|e| e.tick);
            events_done += track.program_change.len() as u64;
            for ev in track.lyrics.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            track.lyrics.sort_by_key(|e| e.tick);
            events_done += track.lyrics.len() as u64;
            for ev in track.chord.iter_mut() {
                ev.tick = scale_tick(ev.tick);
            }
            track.chord.sort_by_key(|e| e.tick);
            events_done += track.chord.len() as u64;
        }
        if let Ok(mut p) = progress.lock() {
            p.progress = 0.99;
            p.label = format!("缩放自动化 {events_done}/{total_events}");
        }

        // ── 阶段 4：更新 ppq + 重算统计（100%）──
        // tick 缩放是单调映射，桶内顺序不变；只需重算统计/tempo map，
        // 不必再跑一遍全量 is_sorted + sort。
        model.meta.ppq = new_ppq;
        if let Ok(mut p) = progress.lock() {
            p.progress = 0.99;
            p.label = "重算索引".to_string();
        }
        model.rebuild_stats_only();
        if let Ok(mut p) = progress.lock() {
            p.progress = 1.0;
            p.label = "完成".to_string();
        }

        Ok(model)
    }
}

/// 异步 rescale 的进度报告结构。
///
/// 与 `yinhe_editor_core::progress::LoadProgress` 独立，
/// 因为 rescale 是单阶段任务，不需要多 stage 数组。
#[derive(Clone, Default)]
pub struct RescaleProgress {
    /// 0.0..1.0
    pub progress: f32,
    /// 当前阶段标签（如 "缩放音符 42/{KEY_COUNT}"）。
    pub label: String,
}

/// 音符装载器：分配 id、入桶、统计（单趟，避免 rebuild 二次扫描）。
///
/// `load_track_notes` 的底层状态；`.yin` 流式加载直接使用：
/// `feed` 逐音符喂入（乱序 OK），`finish` 时统一排序分块。
/// 传入 `per_key_count` 可精确预分配各 key 桶容量（避免 Vec 动态翻倍）。
pub struct NoteLoader {
    key_notes: [Vec<Note>; KEY_COUNT],
    note_count: u64,
    max_tick: u64,
    track_counts: Vec<u64>,
    track_audible: Vec<u64>,
    bucket_stats: [HashMap<u16, (u64, u64)>; KEY_COUNT],
    max_id_seen: u32,
    next_note_id: u32,
    /// 每轨第一个音符 start / 最后一个音符 end，用于初始化整轨 Clip。
    track_min_start: Vec<u32>,
    track_max_end: Vec<u64>,
}

impl NoteLoader {
    /// 按每 key 预估容量精确分配（MIDI 解析/流式加载路径，先扫一遍数数）。
    pub fn new(track_count: usize, next_note_id: u32, per_key_count: [u32; KEY_COUNT]) -> Self {
        Self {
            key_notes: core::array::from_fn(|k| Vec::with_capacity(per_key_count[k] as usize)),
            note_count: 0,
            max_tick: 0,
            track_counts: vec![0u64; track_count],
            track_audible: vec![0u64; track_count],
            bucket_stats: core::array::from_fn(|_| HashMap::new()),
            max_id_seen: 0,
            next_note_id,
            track_min_start: vec![u32::MAX; track_count],
            track_max_end: vec![0; track_count],
        }
    }

    /// 处理单个音符：发号（0=未分配）+ 入桶 + 统计。
    pub fn feed(
        &mut self,
        key: u8,
        track: u16,
        start_tick: u32,
        end_tick: u32,
        velocity: u8,
        id: u32,
    ) {
        let key = key as usize;
        let end = end_tick as u64;
        if end > self.max_tick {
            self.max_tick = end;
        }
        self.note_count += 1;
        if (track as usize) < self.track_counts.len() {
            self.track_counts[track as usize] += 1;
            self.track_min_start[track as usize] =
                self.track_min_start[track as usize].min(start_tick);
            self.track_max_end[track as usize] =
                self.track_max_end[track as usize].max(end_tick as u64);
            if velocity > 1 {
                self.track_audible[track as usize] += 1;
            }
        }
        // id 分配：0 = 未分配，从发号器取；非 0 = 外部分配，保留并跟踪 max。
        let id = if id == 0 {
            let id = self.next_note_id;
            self.next_note_id = self.next_note_id.wrapping_add(1);
            id
        } else {
            if id > self.max_id_seen {
                self.max_id_seen = id;
            }
            id
        };
        self.key_notes[key].push(Note {
            id,
            start_tick,
            end_tick,
            velocity,
            track,
        });
        let e = self.bucket_stats[key].entry(track).or_insert((0, 0));
        e.0 += 1;
        if velocity > 1 {
            e.1 += 1;
        }
    }

    /// 写回模型：统计字段 + 发号器（保留 id 时推进到 max+1）。
    ///
    /// 逐桶 take → 排序 → 分块 → 立即写入模型：已处理桶的源 Vec 随即释放，
    /// 峰值内存 ≈ 单桶两份，而不是全部桶两份（1.64 亿音符省 ~2.6GB）。
    pub fn finish(self, model: &mut YinModel) {
        self.finish_with_progress(model, |_| {});
    }

    /// 同 [`finish`](Self::finish)，按**已写回音符数**回报进度。
    ///
    /// 排序本身是单次库调用，无法在桶内分片；因此以桶为采样点，
    /// 但 done/total 始终是真实音符数，不是 key 下标。
    pub fn finish_with_progress(mut self, model: &mut YinModel, mut on_notes: impl FnMut(u64)) {
        if self.max_id_seen + 1 > self.next_note_id {
            self.next_note_id = self.max_id_seen + 1;
        }
        model.next_note_id = self.next_note_id;

        let total = self.note_count;
        let mut done = 0u64;
        for key in 0..KEY_COUNT {
            let mut v = std::mem::take(&mut self.key_notes[key]);
            // 加载路径统一在此排序（乱序输入），随后按 65536 切块。
            // 同 start_tick 的音符顺序不影响音乐/渲染；不稳定排序避免为
            // 数千万音符再申请一份 O(N) 的稳定排序临时缓冲。
            done += v.len() as u64;
            v.sort_unstable_by_key(|n| n.start_tick);
            model.notes[key] = Arc::new(NoteBucket::from_sorted(v));
            on_notes(done.min(total));
        }
        model.note_count = self.note_count;
        model.tick_length = self.max_tick;
        // `track_count` 在构造 loader 时可能是上界（MIDI 分批解析时最终轨数
        // 要到解析结束才知道），只保留真实轨道对应的统计槽位。
        let track_count = model.tracks.len();
        self.track_counts.truncate(track_count);
        self.track_audible.truncate(track_count);
        model.track_note_count = self.track_counts;
        model.track_audible_count = self.track_audible;
        for (k, bucket) in model.notes.iter().enumerate() {
            model.bucket_note_count[k] = bucket.len() as u64;
            model.bucket_track_stats[k] = std::mem::take(&mut self.bucket_stats[k]);
        }
        // Clip v1：映射里已带 Clip 的轨道保留原值；其余非空轨生成整轨 Clip。
        let bounds: Vec<Option<(u32, u32)>> = self
            .track_min_start
            .iter()
            .zip(self.track_max_end.iter())
            .map(|(&s, &e)| (s != u32::MAX).then_some((s, e.min(u32::MAX as u64) as u32)))
            .collect();
        model.init_clips_from_bounds(&bounds);
        model.sync_next_clip_id();
    }
}
