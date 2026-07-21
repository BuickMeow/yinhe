//! YinModel 的统计缓存与重建逻辑。
//!
//! 从 model.rs 拆分出来，避免 model.rs 过长。所有方法都是
//! `impl YinModel` 的扩展，访问相同的字段。

use std::collections::HashMap;
use std::sync::Arc;

use yinhe_types::Note;

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
    /// 由本方法从 `next_note_id` 起顺序发号；非 0 表示外部已分配（.yin 加载），
    /// 保留原 id 并推进 `next_note_id` 到 max+1。
    pub fn load_track_notes(&mut self, per_track_notes: Vec<Vec<NoteEvent>>) {
        // Count per key for exact allocation.
        let mut per_key_count = [0u32; 128];
        for notes in per_track_notes.iter() {
            for note in notes {
                per_key_count[note.key as usize] += 1;
            }
        }

        // Allocate and fill each bucket, counting in one pass.
        let mut key_notes: [Vec<Note>; 128] =
            core::array::from_fn(|k| Vec::with_capacity(per_key_count[k] as usize));

        let mut note_count: u64 = 0;
        let mut max_tick: u64 = 0;
        let mut track_counts: Vec<u64> = vec![0u64; self.tracks.len()];
        let mut track_audible: Vec<u64> = vec![0u64; self.tracks.len()];
        let mut bucket_stats: [HashMap<u16, (u64, u64)>; 128] =
            core::array::from_fn(|_| HashMap::new());
        let mut max_id_seen: u32 = 0;

        for (track_idx, notes) in per_track_notes.into_iter().enumerate() {
            for note in notes {
                let end = note.end_tick as u64;
                if end > max_tick {
                    max_tick = end;
                }
                note_count += 1;
                if (track_idx as usize) < track_counts.len() {
                    track_counts[track_idx as usize] += 1;
                    if note.velocity > 1 {
                        track_audible[track_idx as usize] += 1;
                    }
                }
                // id 分配：0 = 未分配，从发号器取；非 0 = 外部分配，保留并跟踪 max。
                let id = if note.id == 0 {
                    let id = self.next_note_id;
                    self.next_note_id = self.next_note_id.wrapping_add(1);
                    id
                } else {
                    if note.id > max_id_seen {
                        max_id_seen = note.id;
                    }
                    note.id
                };
                let key = note.key as usize;
                key_notes[key].push(Note {
                    id,
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    track: track_idx as u16,
                });
                let e = bucket_stats[key].entry(track_idx as u16).or_insert((0, 0));
                e.0 += 1;
                if note.velocity > 1 {
                    e.1 += 1;
                }
            }
        }

        // 若加载了外部分配的 id，确保发号器在 max+1 之上，避免后续冲突。
        if max_id_seen + 1 > self.next_note_id {
            self.next_note_id = max_id_seen + 1;
        }

        self.notes = Box::new(key_notes.map(|v| Arc::new(v)));
        self.note_count = note_count;
        self.tick_length = max_tick;
        self.track_note_count = track_counts;
        self.track_audible_count = track_audible;
        for (k, bucket) in self.notes.iter().enumerate() {
            self.bucket_note_count[k] = bucket.len() as u64;
            self.bucket_track_stats[k] = std::mem::take(&mut bucket_stats[k]);
        }
        self.recompute_channel_counts();
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

    /// 从 `track_audible_count` + `tracks` 重新派生 per-channel 计数。
    ///
    /// 成本 O(tracks)，通常 17-100 个 track，几乎免费。在 `rebuild` /
    /// `rebuild_dirty` 末尾调用，保持 `channel_note_count` /
    /// `channel_ctrl_count` 与 `track_audible_count` 一致。
    ///
    /// 激活语义与 `ChannelLayout::from_model` 完全对齐：
    /// - note_count[ch] > 0 ⟺ ch 上至少一个 vel > 1 的音符
    /// - ctrl_count[ch] > 0 ⟺ ch 上至少一个 track 有 automation / PC
    fn recompute_channel_counts(&mut self) {
        let mut note_counts = [0u32; 256];
        let mut ctrl_counts = [0u32; 256];
        for (track_idx, track) in self.tracks.iter().enumerate() {
            let ch = track.global_channel() as usize;
            // channel_note_count: 累加该 track 的发声音符数（与 ChannelLayout::from_model 的 saturating_add 一致）
            if let Some(&audible) = self.track_audible_count.get(track_idx) {
                if audible > 0 {
                    note_counts[ch] = note_counts[ch].saturating_add(audible as u32);
                }
            }
            // channel_ctrl_count: 该 track 有 automation_lanes 或 program_change 就 +1
            if !track.automation_lanes.is_empty() || !track.program_change.is_empty() {
                ctrl_counts[ch] = ctrl_counts[ch].saturating_add(1);
            }
        }
        self.channel_note_count = Box::new(note_counts);
        self.channel_ctrl_count = Box::new(ctrl_counts);
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
        // Sort all 128 buckets in parallel.
        use rayon::prelude::*;
        self.notes
            .par_iter_mut()
            .for_each(|bucket| Arc::make_mut(bucket).sort_by_key(|n| n.start_tick));

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
        let mut bucket_stats: [HashMap<u16, (u64, u64)>; 128] =
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
        for k in 0..128 {
            self.bucket_note_count[k] = self.notes[k].len() as u64;
            self.bucket_track_stats[k] = std::mem::take(&mut bucket_stats[k]);
        }

        // Rebuild tempo_map (depends on tick_length we just computed).
        self.tempo_map = Arc::new(self.build_tempo_map());

        // 派生 per-channel 计数（用于音频引擎检测 ChannelLayout 翻转）。
        self.recompute_channel_counts();
    }

    /// Rebuild only the dirty buckets and update statistics incrementally.
    ///
    /// Cost: O(sum of dirty bucket sizes) for sorting + O(D) for stats.
    /// For a 30M-note song where only 10 buckets were touched, this is
    /// ~O(10 bucket scans) instead of O(128 bucket sorts + clones).
    ///
    /// The sorting step only calls `Arc::make_mut` on dirty buckets,
    /// so clean buckets that share Arc data with an undo snapshot are
    /// never deep-cloned — the key performance win over `rebuild()`.
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
        let dirty_indices: Vec<usize> = (0..128)
            .filter(|&k| self.dirty_keys[k])
            .collect();
        if dirty_indices.is_empty() {
            return;
        }

        let prev_tick_length = self.tick_length;

        // 1. Sort only dirty buckets in parallel (Arc::make_mut only for these).
        use rayon::prelude::*;
        let dirty = self.dirty_keys; // Copy 128 bools
        self.notes
            .par_iter_mut()
            .enumerate()
            .for_each(|(k, bucket)| {
                if dirty[k] {
                    Arc::make_mut(bucket).sort_by_key(|n| n.start_tick);
                }
            });
        self.dirty_keys = [false; 128];

        // 2. Incremental stats: subtract old per-bucket counts, add new ones.
        let mut delta_note_count: i64 = 0;
        let mut new_tick_length = self.tick_length;
        for &k in &dirty_indices {
            let old = self.bucket_note_count[k] as i64;
            let new = self.notes[k].len() as i64;
            delta_note_count += new - old;
            self.bucket_note_count[k] = new as u64;

            // Update tick_length: scan dirty buckets for max end_tick.
            for n in self.notes[k].iter() {
                let end = n.end_tick as u64;
                if end > new_tick_length {
                    new_tick_length = end;
                }
            }
        }
        self.note_count = (self.note_count as i64 + delta_note_count) as u64;
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

        // 5. 派生 per-channel 计数（与 rebuild() 末尾一致）。
        self.recompute_channel_counts();
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
    pub fn rescale_ppq(&mut self, new_ppq: u32) {
        let old_ppq = self.meta.ppq;
        if old_ppq == new_ppq || old_ppq == 0 {
            return;
        }
        let scale = new_ppq as f64 / old_ppq as f64;
        let scale_tick = |t: u32| -> u32 {
            let v = (t as f64 * scale).round();
            // 防御性 clamp：避免极端输入溢出。u32::MAX 已经远超任何合理 tick。
            if v > u32::MAX as f64 { u32::MAX } else { v as u32 }
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
            bucket.sort_by_key(|n| n.start_tick);
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
}
