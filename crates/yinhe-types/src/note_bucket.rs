//! 单个 key 的音符存储：按 `start_tick` 排序的 65536 音符/块 分块序列。
//!
//! # 为什么分块
//!
//! 黑乐谱单 key 可能堆上千万音符，单 `Vec` 的 `insert` 是 O(桶) memmove
//! （10M 音符 ≈ 80MB 内存搬移，数毫秒卡顿）。分块后插入只在块内搬移：
//! - 块大小 65536 音符 = 1MB（`Note` 16B），最坏插入 memmove 512KB ≈ 0.05ms
//! - 块间有序 + 块内有序：查询 = 块级二分（`starts`）+ 块内二分
//! - 每块 `Arc<Vec<Note>>`：模型被共享时（音频线程持有），编辑只深拷贝
//!   触达块（1MB），而不是整个桶
//!
//! # 不变量
//!
//! - `chunks[i]` 的任意元素 `start_tick <= chunks[i+1]` 的任意元素
//! - 每块非空（删除路径会移除空块）
//! - `starts[i]` = `chunks[i][0].start_tick`
//! - 写路径自己维护有序性（`rebuild_dirty` 不再排序）；`sort()` 是
//!   加载/结构变化后的全量重建入口

use std::sync::Arc;

use crate::Note;

/// 65536 音符/块 = 1MB/块。2 的幂：分配器友好；u16 块内偏移恰好够用
/// （索引 0..=65535 是 u16 完整定义域）。
pub const BUCKET_CHUNK_CAP: usize = 65536;

#[derive(Clone, Debug, Default)]
pub struct NoteBucket {
    /// 块间有序（见模块不变量）。
    chunks: Vec<Arc<Vec<Note>>>,
    /// 每块首元素的 `start_tick`，块级二分定位用。
    starts: Vec<u32>,
}

impl NoteBucket {
    /// 从已按 `start_tick` 排序的音符构建（O(N) 逐元素移动，无额外复制）。
    pub fn from_sorted(notes: Vec<Note>) -> Self {
        let mut chunks = Vec::new();
        let mut starts = Vec::new();
        let iter = notes.into_iter();
        let mut current: Vec<Note> = Vec::with_capacity(BUCKET_CHUNK_CAP);
        for n in iter {
            if current.is_empty() {
                starts.push(n.start_tick);
            }
            current.push(n);
            if current.len() == BUCKET_CHUNK_CAP {
                chunks.push(Arc::new(current));
                current = Vec::with_capacity(BUCKET_CHUNK_CAP);
            }
        }
        if !current.is_empty() {
            chunks.push(Arc::new(current));
        }
        Self { chunks, starts }
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// 清空全部音符。
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.starts.clear();
    }

    pub fn len(&self) -> usize {
        self.chunks.iter().map(|c| c.len()).sum()
    }

    pub fn first(&self) -> Option<&Note> {
        self.chunks.first().and_then(|c| c.first())
    }

    pub fn last(&self) -> Option<&Note> {
        self.chunks.last().and_then(|c| c.last())
    }

    /// 第 `index` 个音符（跨块线性定位，O(块数)）。测试/诊断用。
    pub fn get(&self, index: usize) -> Option<&Note> {
        let mut i = index;
        for c in &self.chunks {
            if i < c.len() {
                return c.get(i);
            }
            i -= c.len();
        }
        None
    }

    /// 是否满足块内/块间有序不变量（O(N) 比较）。
    pub fn is_sorted(&self) -> bool {
        if self
            .chunks
            .iter()
            .any(|c| c.windows(2).any(|w| w[0].start_tick > w[1].start_tick))
        {
            return false;
        }
        self.chunks
            .windows(2)
            .all(|w| w[0].last().unwrap().start_tick <= w[1].first().unwrap().start_tick)
    }

    /// 跨块顺序迭代。
    pub fn iter(&self) -> impl Iterator<Item = &Note> + '_ {
        self.chunks.iter().flat_map(|c| c.iter())
    }

    /// 跨块顺序可变迭代。逐块 `Arc::make_mut`：模型被共享时，每个块都会
    /// 深拷贝（1MB）。只适合"全局都要改"的操作（track remap、ppq 缩放）。
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Note> + '_ {
        self.chunks
            .iter_mut()
            .flat_map(|c| Arc::make_mut(c).iter_mut())
    }

    /// 范围迭代：`start_tick ∈ [lo, hi)` 的音符（按排序键左闭右开）。
    /// 从 `lo` 所在块开始，只扫命中块，块内二分。
    pub fn range(&self, lo: u32, hi: u32) -> NoteRangeIter<'_> {
        let start_idx = self.chunk_index_for(lo);
        NoteRangeIter::new(&self.chunks[start_idx..], &self.starts[start_idx..], lo, hi)
    }

    /// 按排序键单点插入：块级定位 + 块内插入（O(块内)），满则均分。
    /// 只深拷贝触达块。
    pub fn insert_sorted(&mut self, note: Note) {
        if self.chunks.is_empty() {
            self.chunks.push(Arc::new(vec![note]));
            self.starts.push(note.start_tick);
            return;
        }
        let idx = self.chunk_index_for(note.start_tick);
        let chunk = Arc::make_mut(&mut self.chunks[idx]);
        let pos = chunk.partition_point(|n| n.start_tick < note.start_tick);
        chunk.insert(pos, note);
        if pos == 0 {
            // 插到块头改变了块首元素，starts 必须同步（否则块级二分失序）。
            self.starts[idx] = chunk[0].start_tick;
        }
        if chunk.len() > BUCKET_CHUNK_CAP {
            let split_at = chunk.len() / 2;
            let right = chunk.split_off(split_at);
            self.starts.insert(idx + 1, right[0].start_tick);
            self.chunks.insert(idx + 1, Arc::new(right));
        }
    }

    /// 批量插入：新音符排序后按块切分，逐块双路归并（O(N + K)）。
    /// 归并结果按 65536 切块；块间有序性由切分保证。
    pub fn insert_batch_sorted(&mut self, mut new: Vec<Note>) {
        if new.is_empty() {
            return;
        }
        new.sort_by_key(|n| n.start_tick);
        if self.chunks.is_empty() {
            for part in new.chunks(BUCKET_CHUNK_CAP) {
                self.starts.push(part[0].start_tick);
                self.chunks.push(Arc::new(part.to_vec()));
            }
            return;
        }
        let mut result: Vec<Arc<Vec<Note>>> = Vec::with_capacity(self.chunks.len() + 1);
        let mut starts = Vec::with_capacity(result.capacity());
        let mut cursor = 0usize;
        for chunk in &self.chunks {
            // 首元素落在本块区间（<= 本块尾）的新音符归入本块一起归并。
            let end = chunk
                .last()
                .map(|last| {
                    new[cursor..].partition_point(|n| n.start_tick <= last.start_tick) + cursor
                })
                .unwrap_or(cursor);
            let merged = merge_runs(chunk, &new[cursor..end]);
            cursor = end;
            for part in merged.chunks(BUCKET_CHUNK_CAP) {
                starts.push(part[0].start_tick);
                result.push(Arc::new(part.to_vec()));
            }
        }
        // 超出旧块范围的新音符，直接成块。
        for part in new[cursor..].chunks(BUCKET_CHUNK_CAP) {
            starts.push(part[0].start_tick);
            result.push(Arc::new(part.to_vec()));
        }
        self.chunks = result;
        self.starts = starts;
    }

    /// 删除 `start_tick ∈ [lo, hi)` 的连续段（全 track 框选快路径），
    /// 返回被删音符。跨块逐段 `drain`（memmove），空块移除。
    pub fn drain_range(&mut self, lo: u32, hi: u32) -> Vec<Note> {
        let mut out = Vec::new();
        if self.chunks.is_empty() || lo >= hi {
            return out;
        }
        let lo_chunk = self.chunk_index_for(lo);
        let hi_chunk = self.chunk_index_for(hi.saturating_sub(1));
        for i in (lo_chunk..=hi_chunk).rev() {
            let (removed, empty) = {
                let chunk = Arc::make_mut(&mut self.chunks[i]);
                let a = chunk.partition_point(|n| n.start_tick < lo);
                let b = chunk.partition_point(|n| n.start_tick < hi);
                let removed: Vec<Note> = if a < b {
                    chunk.drain(a..b).collect()
                } else {
                    Vec::new()
                };
                (removed, chunk.is_empty())
            };
            out.extend(removed);
            if empty {
                self.chunks.remove(i);
            }
        }
        self.rebuild_starts();
        out
    }

    /// 删除 `start_tick ∈ [lo, hi)` 内满足 `pred` 的音符（track 过滤慢路径），
    /// 返回被删音符。每块只扫命中段（drain + 三明治合并），不扫段外。
    pub fn drain_range_filtered(
        &mut self,
        lo: u32,
        hi: u32,
        mut pred: impl FnMut(&Note) -> bool,
    ) -> Vec<Note> {
        let mut out = Vec::new();
        if self.chunks.is_empty() || lo >= hi {
            return out;
        }
        let lo_chunk = self.chunk_index_for(lo);
        let hi_chunk = self.chunk_index_for(hi.saturating_sub(1));
        let mut empty_blocks = Vec::new();
        for i in lo_chunk..=hi_chunk {
            let (removed, empty) = {
                let chunk = Arc::make_mut(&mut self.chunks[i]);
                let a = chunk.partition_point(|n| n.start_tick < lo);
                let b = chunk.partition_point(|n| n.start_tick < hi);
                let mut removed = Vec::new();
                if a < b {
                    let segment: Vec<Note> = chunk.drain(a..b).collect();
                    let mut keep = Vec::with_capacity(segment.len());
                    for n in segment {
                        if pred(&n) {
                            removed.push(n);
                        } else {
                            keep.push(n);
                        }
                    }
                    if !keep.is_empty() {
                        // keep 有序，且首 >= chunk[a-1]、尾 <= 原 chunk[b]（现 chunk[a]），
                        // 三明治合并保持有序。
                        let mut merged = Vec::with_capacity(chunk.len() + keep.len());
                        merged.extend_from_slice(&chunk[..a]);
                        merged.extend_from_slice(&keep);
                        merged.extend_from_slice(&chunk[a..]);
                        *chunk = merged;
                    }
                }
                (removed, chunk.is_empty())
            };
            out.extend(removed);
            if empty {
                empty_blocks.push(i);
            }
        }
        for &i in empty_blocks.iter().rev() {
            self.chunks.remove(i);
        }
        self.rebuild_starts();
        out
    }

    /// 逐块 `retain`，空块移除，重建 `starts`。
    /// 全局操作（track remap 等）用；单音符删除请用 [`Self::remove_by_id`]。
    pub fn retain(&mut self, mut f: impl FnMut(&Note) -> bool) {
        let mut i = 0;
        while i < self.chunks.len() {
            let chunk = Arc::make_mut(&mut self.chunks[i]);
            chunk.retain(&mut f);
            if chunk.is_empty() {
                self.chunks.remove(i);
            } else {
                i += 1;
            }
        }
        self.rebuild_starts();
    }

    /// 按全局唯一 `id` 删除单个音符，返回被删音符。
    /// 先只读定位所在块，再深拷贝该块（不碰其他块）。
    pub fn remove_by_id(&mut self, id: u32) -> Option<Note> {
        let idx = self
            .chunks
            .iter()
            .position(|c| c.iter().any(|n| n.id == id))?;
        let chunk = Arc::make_mut(&mut self.chunks[idx]);
        let pos = chunk.iter().position(|n| n.id == id)?;
        let note = chunk.remove(pos);
        if chunk.is_empty() {
            self.chunks.remove(idx);
        }
        self.rebuild_starts();
        Some(note)
    }

    /// 按 id 集合批量删除（undo/redo 恢复路径）。返回删除数量。
    pub fn remove_by_ids(&mut self, ids: &std::collections::HashSet<u32>) -> usize {
        if ids.is_empty() {
            return 0;
        }
        let mut removed = 0usize;
        let mut i = 0;
        while i < self.chunks.len() {
            let chunk = Arc::make_mut(&mut self.chunks[i]);
            let before = chunk.len();
            chunk.retain(|n| !ids.contains(&n.id));
            removed += before - chunk.len();
            if chunk.is_empty() {
                self.chunks.remove(i);
            } else {
                i += 1;
            }
        }
        if removed > 0 {
            self.rebuild_starts();
        }
        removed
    }

    /// 按全局唯一 `id` 定位可变引用。只深拷贝目标块。
    pub fn find_mut(&mut self, id: u32) -> Option<&mut Note> {
        let idx = self
            .chunks
            .iter()
            .position(|c| c.iter().any(|n| n.id == id))?;
        let chunk = Arc::make_mut(&mut self.chunks[idx]);
        chunk.iter_mut().find(|n| n.id == id)
    }

    /// 全量重建：排平排序后重新切块（O(N log N) + 一次 O(N) 拷贝）。
    /// 已有序时是 O(N) 检测，直接跳过。加载/rescale/结构变化后调用。
    pub fn sort(&mut self) {
        if self.is_sorted() {
            return;
        }
        let mut flat: Vec<Note> = self.chunks.iter().flat_map(|c| c.iter().copied()).collect();
        flat.sort_by_key(|n| n.start_tick);
        *self = Self::from_sorted(flat);
    }

    /// 块级二分定位：`start_tick <= tick` 的最后一个块。
    /// 空桶返回 0（调用方需先判空）。
    fn chunk_index_for(&self, tick: u32) -> usize {
        self.starts
            .partition_point(|&s| s <= tick)
            .saturating_sub(1)
    }

    /// 从当前块重算 `starts`（O(块数)）。块首元素可能变化的写路径后调用。
    fn rebuild_starts(&mut self) {
        self.starts.clear();
        self.starts
            .extend(self.chunks.iter().map(|c| c[0].start_tick));
    }
}

/// 双路归并两个按 `start_tick` 有序的切片（稳定：old 优先于 new）。
fn merge_runs(old: &[Note], new: &[Note]) -> Vec<Note> {
    let mut merged = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        if old[i].start_tick <= new[j].start_tick {
            merged.push(old[i]);
            i += 1;
        } else {
            merged.push(new[j]);
            j += 1;
        }
    }
    merged.extend_from_slice(&old[i..]);
    merged.extend_from_slice(&new[j..]);
    merged
}

impl std::ops::Index<usize> for NoteBucket {
    type Output = Note;

    /// 跨块索引（O(块数)）。测试/诊断用；越界 panic 语义与 `Vec` 一致。
    fn index(&self, index: usize) -> &Note {
        self.get(index).expect("NoteBucket index out of bounds")
    }
}

/// [`NoteBucket::range`] 返回的具体迭代器类型（具体类型，保持
/// `NoteSource` trait 的 dyn compatibility；块间有序保证提前终止）。
pub struct NoteRangeIter<'a> {
    /// 剩余块（从 `lo` 所在块开始）。
    chunks: std::slice::Iter<'a, Arc<Vec<Note>>>,
    /// 与 `chunks` 同步推进的块首 tick。
    starts: std::slice::Iter<'a, u32>,
    lo: u32,
    hi: u32,
    /// 当前块的段内迭代器。
    inner: Option<std::slice::Iter<'a, Note>>,
    ended: bool,
}

impl<'a> NoteRangeIter<'a> {
    fn new(chunks: &'a [Arc<Vec<Note>>], starts: &'a [u32], lo: u32, hi: u32) -> Self {
        Self {
            chunks: chunks.iter(),
            starts: starts.iter(),
            lo,
            hi,
            inner: None,
            ended: false,
        }
    }
}

impl<'a> Iterator for NoteRangeIter<'a> {
    type Item = &'a Note;

    fn next(&mut self) -> Option<&'a Note> {
        loop {
            if self.ended {
                return None;
            }
            if let Some(inner) = &mut self.inner {
                if let Some(n) = inner.next() {
                    return Some(n);
                }
                self.inner = None; // 当前块段耗尽，进下一块
            }
            let chunk = match self.chunks.next() {
                Some(c) => c,
                None => {
                    self.ended = true;
                    return None;
                }
            };
            let start = self.starts.next();
            let c = chunk.as_slice();
            let a = c.partition_point(|n| n.start_tick < self.lo);
            let b = c.partition_point(|n| n.start_tick < self.hi);
            if a < b {
                self.inner = Some(c[a..b].iter());
            } else if start.is_none_or(|&s| s >= self.hi) {
                // 块首已 >= hi：块间有序，后续块首更大，不可能再命中。
                self.ended = true;
            }
            // 块首 < hi 但本块无命中：继续下一块。
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32, start: u32) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: start + 10,
            velocity: 100,
            track: 0,
        }
    }

    fn assert_sorted(bucket: &NoteBucket) {
        assert!(bucket.is_sorted(), "bucket 失序");
    }

    #[test]
    fn from_sorted_chunks_by_cap() {
        let notes: Vec<Note> = (0..BUCKET_CHUNK_CAP + 3)
            .map(|i| note(i as u32, i as u32))
            .collect();
        let b = NoteBucket::from_sorted(notes);
        assert_eq!(b.len(), BUCKET_CHUNK_CAP + 3);
        assert_eq!(b.get(BUCKET_CHUNK_CAP).unwrap().id, BUCKET_CHUNK_CAP as u32);
        assert_eq!(b.first().unwrap().id, 0);
        assert_eq!(b.last().unwrap().id, (BUCKET_CHUNK_CAP + 2) as u32);
    }

    #[test]
    fn insert_sorted_keeps_sorted_and_splits() {
        let mut b = NoteBucket::from_sorted(
            (0..BUCKET_CHUNK_CAP as u32)
                .map(|i| note(i, i * 2))
                .collect(),
        );
        // 插到中间，触发分裂
        b.insert_sorted(note(999_999, 42));
        assert_sorted(&b);
        assert_eq!(b.len(), BUCKET_CHUNK_CAP + 1);
        assert!(b.iter().any(|n| n.id == 999_999));
    }

    #[test]
    fn insert_sorted_into_empty() {
        let mut b = NoteBucket::default();
        b.insert_sorted(note(1, 100));
        assert_sorted(&b);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn insert_sorted_head_updates_starts() {
        let mut b = NoteBucket::from_sorted(vec![note(1, 100), note(2, 200)]);
        b.insert_sorted(note(3, 50));
        assert_sorted(&b);
        assert_eq!(b.first().unwrap().id, 3);
        assert_eq!(b.starts[0], 50);
    }

    #[test]
    fn insert_batch_merges_across_chunks() {
        let mut b =
            NoteBucket::from_sorted((0..BUCKET_CHUNK_CAP as u32).map(|i| note(i, i)).collect());
        let new: Vec<Note> = (0..10_000).map(|i| note(900_000 + i, 20_000 + i)).collect();
        b.insert_batch_sorted(new);
        assert_sorted(&b);
        assert_eq!(b.len(), BUCKET_CHUNK_CAP + 10_000);
    }

    #[test]
    fn drain_range_removes_span() {
        let mut b = NoteBucket::from_sorted((0..100u32).map(|i| note(i, i * 10)).collect());
        let removed = b.drain_range(200, 500);
        assert_eq!(removed.len(), 30);
        assert_sorted(&b);
        assert_eq!(b.len(), 70);
        assert_eq!(b.first().unwrap().start_tick, 0);
        assert_eq!(b.last().unwrap().start_tick, 990);
    }

    #[test]
    fn drain_range_filtered_respects_pred() {
        let mut b = NoteBucket::from_sorted((0..100u32).map(|i| note(i, i * 10)).collect());
        let removed = b.drain_range_filtered(0, 1000, |n| n.id % 2 == 0);
        assert_eq!(removed.len(), 50);
        assert_sorted(&b);
        assert_eq!(b.len(), 50);
        assert!(b.iter().all(|n| n.id % 2 == 1));
    }

    #[test]
    fn remove_by_id_only_removes_target() {
        let mut b = NoteBucket::from_sorted((0..10u32).map(|i| note(i, i)).collect());
        let n = b.remove_by_id(5).unwrap();
        assert_eq!(n.id, 5);
        assert_sorted(&b);
        assert_eq!(b.len(), 9);
        assert!(b.remove_by_id(999).is_none());
    }

    #[test]
    fn range_iterates_only_hits() {
        let b = NoteBucket::from_sorted((0..100u32).map(|i| note(i, i * 10)).collect());
        let hits: Vec<u32> = b.range(200, 350).map(|n| n.id).collect();
        assert_eq!(
            hits,
            vec![20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34]
        );
        assert_eq!(b.range(5000, 6000).count(), 0);
        assert_eq!(b.range(0, 0).count(), 0);
    }

    #[test]
    fn sort_rebuilds_unsorted_input() {
        let mut b = NoteBucket::from_sorted(vec![note(1, 500), note(2, 100), note(3, 300)]);
        b.sort(); // 已有序：快路径跳过
        assert_sorted(&b);
        // 手工破坏块内顺序后 sort 恢复
        let c = Arc::make_mut(&mut b.chunks[0]);
        c.swap(0, 1);
        b.sort();
        assert_sorted(&b);
        assert_eq!(b.first().unwrap().id, 2);
    }

    #[test]
    fn empty_bucket_ops_are_safe() {
        let mut b = NoteBucket::default();
        assert!(b.is_empty());
        assert!(b.range(0, 100).next().is_none());
        assert!(b.drain_range(0, 100).is_empty());
        assert!(b.remove_by_id(1).is_none());
        b.insert_sorted(note(1, 10));
        assert_eq!(b.len(), 1);
    }
}
