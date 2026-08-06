//! YinModel 的统计缓存与重建逻辑。
//!
//! 从 model.rs 拆分出来，避免 model.rs 过长。所有方法都是
//! `impl YinModel` 的扩展，访问相同的字段。

use std::collections::HashMap;
use std::sync::Arc;

use yinhe_types::{Note, NoteBucket};

use crate::events::{BucketNote, NoteEvent};

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
        let mut per_key_count = [0u32; 128];
        for notes in per_track_notes.iter() {
            for note in notes {
                per_key_count[note.key as usize] += 1;
            }
        }

        let mut loader = NoteLoader::new(self.tracks.len(), self.next_note_id, per_key_count);
        for (track_idx, notes) in per_track_notes.into_iter().enumerate() {
            for note in notes {
                loader.feed(
                    note.key as usize,
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

    /// `.yin` 加载路径：直接按 key 桶填（桶内已按 start_tick 排序）。
    ///
    /// 与 `load_track_notes` 的区别：输入是 128 个 key 桶而非 per-track 列表，
    /// 省去“按 track 分组存 → 加载再分桶”的多余转换（`.yin` 新格式直接按桶存）。
    /// 每个音符的 `track` 取自 `BucketNote.track`；id 一律重新分配（`id` 不落盘）。
    pub fn load_bucket_notes(&mut self, bucket_notes: Vec<Vec<BucketNote>>) {
        let mut loader =
            NoteLoader::with_capacity(self.tracks.len(), self.next_note_id, &bucket_notes);
        for (key, notes) in bucket_notes.into_iter().enumerate() {
            for note in notes {
                loader.feed(
                    key,
                    note.track,
                    note.start_tick,
                    note.end_tick,
                    note.velocity,
                    0, // id 不落盘，一律重新分配
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
        // Sort + re-chunk all 128 buckets in parallel.
        use rayon::prelude::*;
        self.notes.par_iter_mut().for_each(|bucket| {
            Arc::make_mut(bucket).sort();
        });

        // Bump all note_revisions (full rebuild = all keys changed).
        for r in &mut self.note_revisions {
            *r = r.wrapping_add(1);
        }

        // Recompute note_count, max_tick, track_note_count, track_audible_count
        // (may have changed after edits or track insertions).
        let mut note_count: u64 = 0;
        let mut max_tick: u64 = 0;
        let mut max_len: u32 = 0;
        let mut track_counts: Vec<u64> = vec![0u64; self.tracks.len()];
        let mut track_audible: Vec<u64> = vec![0u64; self.tracks.len()];
        // Per-bucket per-track stats — recomputed in the same pass so
        // rebuild_dirty() can do incremental updates later.
        let mut bucket_stats: [HashMap<u16, (u64, u64)>; 128] =
            core::array::from_fn(|_| HashMap::new());
        let mut bucket_max_end: [u64; 128] = [0; 128];
        for (k, bucket) in self.notes.iter().enumerate() {
            note_count += bucket.len() as u64;
            for n in bucket.iter() {
                let end = n.end_tick as u64;
                if end > max_tick {
                    max_tick = end;
                }
                max_len = max_len.max(n.end_tick.saturating_sub(n.start_tick));
                if end > bucket_max_end[k] {
                    bucket_max_end[k] = end;
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
        self.max_note_len = max_len;
        self.track_note_count = track_counts;
        self.track_audible_count = track_audible;
        for k in 0..128 {
            self.bucket_note_count[k] = self.notes[k].len() as u64;
            self.bucket_max_end_tick[k] = bucket_max_end[k];
            self.bucket_track_stats[k] = std::mem::take(&mut bucket_stats[k]);
        }

        // Rebuild tempo_map (depends on tick_length we just computed).
        self.tempo_map = Arc::new(self.build_tempo_map());
    }

    /// Rebuild statistics for the dirty buckets incrementally.
    ///
    /// Cost: O(sum of dirty bucket sizes) for stats + O(D) bookkeeping.
    /// For a 30M-note song where only 10 buckets were touched, this is
    /// ~O(10 bucket scans) instead of O(128 bucket sorts + clones).
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
        let dirty_indices: Vec<usize> = (0..128).filter(|&k| self.dirty_keys[k]).collect();
        if dirty_indices.is_empty() {
            return;
        }

        let prev_tick_length = self.tick_length;
        self.dirty_keys = [false; 128];
        let mut delta_note_count: i64 = 0;
        for &k in &dirty_indices {
            let old = self.bucket_note_count[k] as i64;
            let new = self.notes[k].len() as i64;
            delta_note_count += new - old;
            self.bucket_note_count[k] = new as u64;

            // Recompute this dirty bucket's max end_tick.
            let mut bucket_max: u64 = 0;
            for n in self.notes[k].iter() {
                let end = n.end_tick as u64;
                if end > bucket_max {
                    bucket_max = end;
                }
                // max_note_len 只增不减：删除最长音符后保留旧值只会让视口
                // 查询左边界略偏左，绝不漏音符（见 NoteSource::max_note_len）。
                self.max_note_len = self
                    .max_note_len
                    .max(n.end_tick.saturating_sub(n.start_tick));
            }
            self.bucket_max_end_tick[k] = bucket_max;
        }
        self.note_count = (self.note_count as i64 + delta_note_count) as u64;
        // tick_length = max over all 128 buckets of bucket_max_end_tick.
        // O(128) scan — cheap, and correctly handles shrinkage when the
        // last note was deleted/shortened in a dirty bucket.
        let new_tick_length = self.bucket_max_end_tick.iter().copied().max().unwrap_or(0);
        self.tick_length = new_tick_length;

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
        }

        // 4. Rebuild tempo_map only if tick_length changed.
        //    rebuild_dirty 路径不动 conductor（tempo/time_sig），所以
        //    tempo_map 的 tempo_segments / time_sig_events 不变；
        //    只有 tick_length 字段可能需要同步。
        if new_tick_length != prev_tick_length {
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
        let scale = new_ppq as f64 / old_ppq as f64;
        let scale_tick = |t: u32| -> u32 {
            let v = (t as f64 * scale).round();
            // 防御性 clamp：避免极端输入溢出。u32::MAX 已经远超任何合理 tick。
            if v > u32::MAX as f64 {
                u32::MAX
            } else {
                v as u32
            }
        };

        // 1. Notes (128 buckets, parallel)
        use rayon::prelude::*;
        self.notes.par_iter_mut().for_each(|bucket| {
            let bucket = Arc::make_mut(bucket);
            for n in bucket.iter_mut() {
                n.start_tick = scale_tick(n.start_tick);
                n.end_tick = scale_tick(n.end_tick);
                // 维持 end >= start 的不变量（极端 round 情况下可能相等）
                if n.end_tick < n.start_tick {
                    n.end_tick = n.start_tick;
                }
            }
            // start_tick 是排序键：缩放后重新排序切块。
            bucket.sort();
        });

        // 2. Conductor: tempo events + time signature events
        let conductor = Arc::make_mut(&mut self.conductor);
        for ev in conductor.tempo.events.iter_mut() {
            ev.tick = scale_tick(ev.tick);
        }
        conductor.tempo.events.sort_by_key(|e| e.tick);
        for ts in conductor.time_sig.iter_mut() {
            ts.tick = scale_tick(ts.tick);
        }
        conductor.time_sig.sort_by_key(|e| e.tick);

        // 3. Track automation lanes + program changes
        for track in self.tracks.iter_mut() {
            let track = Arc::make_mut(track);
            for lane in track.automation_lanes.iter_mut() {
                for ev in lane.events.iter_mut() {
                    ev.tick = scale_tick(ev.tick);
                }
                lane.events.sort_by_key(|e| e.tick);
            }
            for pc in track.program_change.iter_mut() {
                pc.tick = scale_tick(pc.tick);
            }
            track.program_change.sort_by_key(|e| e.tick);
        }

        // 4. Update ppq + rebuild derived data
        self.meta.ppq = new_ppq;
        self.rebuild();
    }

    /// 异步版本 of [`Self::rescale_ppq`]：在子线程中执行，带进度报告和取消支持。
    ///
    /// 调用方传入 model 的 clone（不修改原 model），返回 rescale 后的新 model。
    /// 若 `cancel` 在处理过程中被设为 true，提前返回 `Err("已取消")`。
    ///
    /// 进度报告：
    /// - 阶段 1（0..90%）：缩放 128 个音符 bucket，每个 bucket 完成后更新进度
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
        let progress_clone = progress.clone();
        let cancel_clone = cancel.clone();
        model.notes.par_iter_mut().for_each(|bucket| {
            if cancel_clone.load(Ordering::Relaxed) {
                return;
            }
            let bucket = Arc::make_mut(bucket);
            for n in bucket.iter_mut() {
                n.start_tick = scale_tick(n.start_tick);
                n.end_tick = scale_tick(n.end_tick);
                if n.end_tick < n.start_tick {
                    n.end_tick = n.start_tick;
                }
            }
            bucket.sort();
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut p) = progress_clone.lock() {
                p.progress = d as f32 / 128.0 * 0.9;
                p.label = format!("缩放音符 {}/128", d);
            }
        });
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
        }
        if let Ok(mut p) = progress.lock() {
            p.progress = 0.95;
            p.label = "缩放 conductor".to_string();
        }

        // ── 阶段 3：缩放 track automation + PC（99%）──
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
            }
            for pc in track.program_change.iter_mut() {
                pc.tick = scale_tick(pc.tick);
            }
            track.program_change.sort_by_key(|e| e.tick);
        }

        // ── 阶段 4：更新 ppq + rebuild（100%）──
        model.meta.ppq = new_ppq;
        model.rebuild();
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
    /// 当前阶段标签（如 "缩放音符 42/128"）。
    pub label: String,
}

/// `load_track_notes` / `load_bucket_notes` 的共享装载状态：
/// 分配 id、入桶、统计（单趟，避免 rebuild 二次扫描）。
struct NoteLoader {
    key_notes: [Vec<Note>; 128],
    note_count: u64,
    max_tick: u64,
    max_len: u32,
    track_counts: Vec<u64>,
    track_audible: Vec<u64>,
    bucket_stats: [HashMap<u16, (u64, u64)>; 128],
    bucket_max_end: [u64; 128],
    max_id_seen: u32,
    next_note_id: u32,
}

impl NoteLoader {
    /// 按每 key 预估容量精确分配（MIDI 解析路径，先扫一遍数数）。
    fn new(track_count: usize, next_note_id: u32, per_key_count: [u32; 128]) -> Self {
        Self {
            key_notes: core::array::from_fn(|k| Vec::with_capacity(per_key_count[k] as usize)),
            note_count: 0,
            max_tick: 0,
            max_len: 0,
            track_counts: vec![0u64; track_count],
            track_audible: vec![0u64; track_count],
            bucket_stats: core::array::from_fn(|_| HashMap::new()),
            bucket_max_end: [0; 128],
            max_id_seen: 0,
            next_note_id,
        }
    }

    /// 容量直接取自各桶长度（.yin 加载路径，桶已就位）。
    fn with_capacity(
        track_count: usize,
        next_note_id: u32,
        bucket_notes: &[Vec<BucketNote>],
    ) -> Self {
        Self {
            key_notes: core::array::from_fn(|k| {
                Vec::with_capacity(bucket_notes.get(k).map_or(0, |b| b.len()))
            }),
            note_count: 0,
            max_tick: 0,
            max_len: 0,
            track_counts: vec![0u64; track_count],
            track_audible: vec![0u64; track_count],
            bucket_stats: core::array::from_fn(|_| HashMap::new()),
            bucket_max_end: [0; 128],
            max_id_seen: 0,
            next_note_id,
        }
    }

    /// 处理单个音符：发号（0=未分配）+ 入桶 + 统计。
    fn feed(
        &mut self,
        key: usize,
        track: u16,
        start_tick: u32,
        end_tick: u32,
        velocity: u8,
        id: u32,
    ) {
        let end = end_tick as u64;
        if end > self.max_tick {
            self.max_tick = end;
        }
        self.max_len = self.max_len.max(end_tick.saturating_sub(start_tick));
        self.note_count += 1;
        if (track as usize) < self.track_counts.len() {
            self.track_counts[track as usize] += 1;
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
        if end > self.bucket_max_end[key] {
            self.bucket_max_end[key] = end;
        }
        let e = self.bucket_stats[key].entry(track).or_insert((0, 0));
        e.0 += 1;
        if velocity > 1 {
            e.1 += 1;
        }
    }

    /// 写回模型：统计字段 + 发号器（保留 id 时推进到 max+1）。
    fn finish(mut self, model: &mut YinModel) {
        if self.max_id_seen + 1 > self.next_note_id {
            self.next_note_id = self.max_id_seen + 1;
        }
        model.next_note_id = self.next_note_id;

        *model.notes = self.key_notes.map(|mut v| {
            // 加载路径统一在此排序（乱序输入），随后按 65536 切块。
            v.sort_by_key(|n| n.start_tick);
            Arc::new(NoteBucket::from_sorted(v))
        });
        model.note_count = self.note_count;
        model.tick_length = self.max_tick;
        model.max_note_len = self.max_len;
        model.track_note_count = self.track_counts;
        model.track_audible_count = self.track_audible;
        for (k, bucket) in model.notes.iter().enumerate() {
            model.bucket_note_count[k] = bucket.len() as u64;
            model.bucket_max_end_tick[k] = self.bucket_max_end[k];
            model.bucket_track_stats[k] = std::mem::take(&mut self.bucket_stats[k]);
        }
    }
}
