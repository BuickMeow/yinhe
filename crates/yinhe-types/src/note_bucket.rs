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

/// 桶内游标：`chunk` 块下标 + `idx` 块内下标。
///
/// 音频线程顺序消费音符时只前进、不回退，因此不需要为整曲维护一份
/// 平铺副本（旧 `audible_notes`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoteCursor {
    pub chunk: u32,
    pub idx: u32,
}

#[derive(Clone, Debug, Default)]
pub struct NoteBucket {
    /// 块间有序（见模块不变量）。
    chunks: Vec<Arc<Vec<Note>>>,
    /// 每块首元素的 `start_tick`，块级二分定位用。
    starts: Vec<u32>,
    /// 每块内最大的 `end_tick`。
    ///
    /// 范围查询用它跳过整块：若 `max_ends[i] < tick_start`，该块所有音符
    /// 都在视口左侧结束，不可能相交。这样长音符只污染自己所在的块，
    /// 不再需要全局/每 key 的 `max_note_len` 回看。
    max_ends: Vec<u32>,
}

impl NoteBucket {
    /// 从已按 `start_tick` 排序的音符构建（O(N) 逐元素移动，无额外复制）。
    ///
    /// 尾块按实际长度缩容：满块预分配 65536 容量，若最后一个块只装少量音符，
    /// 不缩容会白占近 1MB（88 个 key 桶共 ~90MB）。
    pub fn from_sorted(notes: Vec<Note>) -> Self {
        let mut chunks = Vec::with_capacity(notes.len().div_ceil(BUCKET_CHUNK_CAP));
        let mut starts = Vec::with_capacity(chunks.capacity());
        let mut max_ends = Vec::with_capacity(chunks.capacity());
        let iter = notes.into_iter();
        let mut current: Vec<Note> = Vec::with_capacity(BUCKET_CHUNK_CAP);
        let mut current_max_end = 0u32;
        for n in iter {
            if current.is_empty() {
                starts.push(n.start_tick);
            }
            current_max_end = current_max_end.max(n.end_tick);
            current.push(n);
            if current.len() == BUCKET_CHUNK_CAP {
                max_ends.push(current_max_end);
                current_max_end = 0;
                chunks.push(Arc::new(std::mem::replace(
                    &mut current,
                    Vec::with_capacity(BUCKET_CHUNK_CAP),
                )));
            }
        }
        if !current.is_empty() {
            current.shrink_to_fit();
            max_ends.push(current_max_end);
            chunks.push(Arc::new(current));
        }
        Self {
            chunks,
            starts,
            max_ends,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// 清空全部音符。
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.starts.clear();
        self.max_ends.clear();
    }

    pub fn len(&self) -> usize {
        self.chunks.iter().map(|c| c.len()).sum()
    }

    /// 桶内最大 `end_tick`（从块级 `max_ends` 求 max）。
    pub fn max_end_tick(&self) -> u32 {
        self.max_ends.iter().copied().max().unwrap_or(0)
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

    /// 游标处的音符（自动跨块、跳过空块），返回**规范化后**的游标。
    ///
    /// 音频 dispatch 用它顺序消费整曲音符：光标单调前进，每个音符只碰一次，
    /// 不需要预构建一份 `audible_notes` 副本。
    #[inline]
    pub fn peek(&self, mut c: NoteCursor) -> Option<(NoteCursor, &Note)> {
        loop {
            let chunk = self.chunks.get(c.chunk as usize)?;
            if let Some(note) = chunk.get(c.idx as usize) {
                return Some((c, note));
            }
            c.chunk += 1;
            c.idx = 0;
        }
    }

    /// 游标前进一个音符（越界由 [`Self::peek`] 负责跨块/收尾）。
    #[inline]
    pub fn advance(&self, c: &mut NoteCursor) {
        c.idx += 1;
    }

    /// 第一个 `start_tick >= tick` 的游标位置（块间 + 块内二分）。
    /// 第一个 `start_tick > tick` 的游标位置。
    ///
    /// 用于"`<= tick` 的音符已经派发完毕"的场景（块末重定位）：与
    /// [`Self::cursor_at`] 的区别是同一 tick 上的音符不会重复触发。
    #[inline]
    pub fn cursor_after(&self, tick: u32) -> NoteCursor {
        let next = self.starts.partition_point(|&s| s <= tick);
        let Some(ci) = next.checked_sub(1) else {
            return NoteCursor::default();
        };
        let idx = self.chunks[ci].partition_point(|n| n.start_tick <= tick);
        NoteCursor {
            chunk: ci as u32,
            idx: idx as u32,
        }
    }

    #[inline]
    pub fn cursor_at(&self, tick: u32) -> NoteCursor {
        let next = self.starts.partition_point(|&s| s < tick);
        let Some(ci) = next.checked_sub(1) else {
            return NoteCursor::default();
        };
        let idx = self.chunks[ci].partition_point(|n| n.start_tick < tick);
        NoteCursor {
            chunk: ci as u32,
            idx: idx as u32,
        }
    }

    /// 跨块顺序可变迭代。逐块 `Arc::make_mut`：模型被共享时，每个块都会
    /// 深拷贝（1MB）。只适合"全局都要改"的操作（track remap、ppq 缩放）。
    ///
    /// 若修改了 `end_tick`，调用方必须随后 [`rebuild_max_ends`](Self::rebuild_max_ends)；
    /// 只改 `track` 等不影响 end 的字段时不需要。
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

    /// 可能与 `[tick_start, tick_end]` 相交的范围迭代。
    ///
    /// 用每块 `max_end_tick` 找第一个可能相交的块，再从该块首音符开始扫；
    /// 块内仍可能包含 `end_tick < tick_start` 的音符（end 不参与排序），
    /// 调用方按既有契约自行精确过滤。整块在视口左侧结束的会被直接跳过。
    pub fn range_including_long(&self, tick_start: u32, tick_end: u32) -> NoteRangeIter<'_> {
        match self.first_intersecting_chunk(tick_start, tick_end) {
            Some(idx) => self.range(self.starts[idx], tick_end),
            None => self.range(tick_end, tick_end),
        }
    }

    /// 与 `[tick_start, tick_end]` 可能相交的块切片（CPU cull 用）。
    ///
    /// 只返回块级候选，块内是否真的相交由调用方按 `end_tick` 过滤。
    /// 不分配、不复制，直接借用内部 chunks。
    pub fn chunks_in_range(&self, tick_start: u32, tick_end: u32) -> &[Arc<Vec<Note>>] {
        let Some(first) = self.first_intersecting_chunk(tick_start, tick_end) else {
            return &[];
        };
        let end = self.starts.partition_point(|&s| s <= tick_end);
        if first >= end {
            &[]
        } else {
            &self.chunks[first..end]
        }
    }

    /// 第一个可能相交的块下标：块首 <= tick_end 且块内 max_end >= tick_start。
    fn first_intersecting_chunk(&self, tick_start: u32, tick_end: u32) -> Option<usize> {
        let end = self.starts.partition_point(|&s| s <= tick_end);
        (0..end).find(|&i| self.max_ends[i] >= tick_start)
    }

    /// 同 [`drain_range_filtered`](Self::drain_range_filtered)，但左界按块级
    /// `max_end_tick` 自动外扩：起点远早于 `[lo, hi)`、但终点仍可能落入
    /// 区间的长音符不会被漏掉，也不需要全局 `max_note_len`。
    pub fn drain_range_including_long(
        &mut self,
        lo: u32,
        hi: u32,
        pred: impl FnMut(&Note) -> bool,
    ) -> Vec<Note> {
        let Some(first) = self.first_intersecting_chunk(lo, hi) else {
            return Vec::new();
        };
        if hi == 0 || self.starts[first] >= hi {
            return Vec::new();
        }
        self.drain_range_filtered(self.starts[first], hi, pred)
    }

    /// 重算每块 `max_end_tick`。`iter_mut` / 外部直接改 end 的路径必须调用。
    pub fn rebuild_max_ends(&mut self) {
        self.max_ends.clear();
        self.max_ends.extend(
            self.chunks
                .iter()
                .map(|c| c.iter().map(|n| n.end_tick).max().unwrap_or(0)),
        );
    }

    /// 按排序键单点插入：块级定位 + 块内插入（O(块内)），满则均分。
    /// 只深拷贝触达块。
    pub fn insert_sorted(&mut self, note: Note) {
        if self.chunks.is_empty() {
            self.chunks.push(Arc::new(vec![note]));
            self.starts.push(note.start_tick);
            self.max_ends.push(note.end_tick);
            return;
        }
        let idx = self.chunk_index_for(note.start_tick);
        let chunk = Arc::make_mut(&mut self.chunks[idx]);
        let pos = chunk.partition_point(|n| n.start_tick < note.start_tick);
        chunk.insert(pos, note);
        self.max_ends[idx] = self.max_ends[idx].max(note.end_tick);
        if pos == 0 {
            // 插到块头改变了块首元素，starts 必须同步（否则块级二分失序）。
            self.starts[idx] = chunk[0].start_tick;
        }
        if chunk.len() > BUCKET_CHUNK_CAP {
            let split_at = chunk.len() / 2;
            let right = chunk.split_off(split_at);
            self.max_ends[idx] = chunk.iter().map(|n| n.end_tick).max().unwrap_or(0);
            let right_max = right.iter().map(|n| n.end_tick).max().unwrap_or(0);
            self.starts.insert(idx + 1, right[0].start_tick);
            self.max_ends.insert(idx + 1, right_max);
            self.chunks.insert(idx + 1, Arc::new(right));
        }
    }

    /// 按 `(start_tick, track)` 定位一个音符；`id` 仅在同 `(start, track)`
    /// 有多个音符时消歧。
    ///
    /// 注意块切分可能把同一 `start_tick` 的音符放进相邻块：
    /// `chunk_index_for` 返回最后一个 `starts <= start` 的块，目标可能在前一块，
    /// 所以先向前回退，再向后扫描到块首 > start 为止。
    fn locate_at(&self, start: u32, track: u16, id: Option<u32>) -> Option<(usize, usize)> {
        if self.chunks.is_empty() {
            return None;
        }
        let mut ci = self.chunk_index_for(start);
        while ci > 0
            && self.chunks[ci - 1]
                .last()
                .is_some_and(|n| n.start_tick >= start)
        {
            ci -= 1;
        }
        for (i, chunk) in self.chunks.iter().enumerate().skip(ci) {
            let Some(first) = chunk.first() else {
                continue;
            };
            if first.start_tick > start {
                break;
            }
            let a = chunk.partition_point(|n| n.start_tick < start);
            let b = chunk.partition_point(|n| n.start_tick <= start);
            for (offset, n) in chunk[a..b].iter().enumerate() {
                if n.track == track && id.is_none_or(|id| n.id == id) {
                    return Some((i, a + offset));
                }
            }
        }
        None
    }

    /// 重算单块的 `starts` / `max_ends`。调用前块必须非空。
    fn refresh_chunk_meta(&mut self, ci: usize) {
        let chunk = &self.chunks[ci];
        self.starts[ci] = chunk[0].start_tick;
        self.max_ends[ci] = chunk.iter().map(|n| n.end_tick).max().unwrap_or(0);
    }

    /// 单点修改：`f` 可以改 end/velocity/track，也可以改 start。
    /// start 改变时只搬移该音符（必要时跨块），不全局 `sort()`。
    ///
    /// 返回 `(before, after)`，供模型层做增量统计和 undo。
    pub fn update_at(
        &mut self,
        start: u32,
        track: u16,
        id: u32,
        f: impl FnOnce(&mut Note),
    ) -> Option<(Note, Note)> {
        let (ci, ni) = self.locate_at(start, track, Some(id))?;
        let old = self.chunks[ci][ni];
        let (new, start_changed, end_changed) = {
            let chunk = Arc::make_mut(&mut self.chunks[ci]);
            f(&mut chunk[ni]);
            let new = chunk[ni];
            (
                new,
                new.start_tick != old.start_tick,
                new.end_tick != old.end_tick,
            )
        };

        if start_changed {
            let empty = {
                let chunk = Arc::make_mut(&mut self.chunks[ci]);
                let removed = chunk.remove(ni);
                debug_assert_eq!(removed.id, id);
                chunk.is_empty()
            };
            if empty {
                self.chunks.remove(ci);
                self.starts.remove(ci);
                self.max_ends.remove(ci);
            } else {
                if ni == 0 {
                    self.starts[ci] = self.chunks[ci][0].start_tick;
                }
                if old.end_tick == self.max_ends[ci] {
                    self.refresh_chunk_meta(ci);
                }
            }
            // insert_sorted 会同步目标块的 starts / max_ends，并处理块分裂。
            self.insert_sorted(new);
            return Some((old, new));
        }

        if end_changed {
            if new.end_tick > self.max_ends[ci] {
                self.max_ends[ci] = new.end_tick;
            } else if old.end_tick == self.max_ends[ci] && new.end_tick < old.end_tick {
                self.refresh_chunk_meta(ci);
            }
        }
        Some((old, new))
    }

    /// 单点删除：只重算受影响块的元数据。
    ///
    /// 返回被删除的音符；找不到返回 `None`。
    pub fn remove_at(&mut self, start: u32, track: u16, id: u32) -> Option<Note> {
        let (ci, ni) = self.locate_at(start, track, Some(id))?;
        let (note, empty) = {
            let chunk = Arc::make_mut(&mut self.chunks[ci]);
            let note = chunk.remove(ni);
            (note, chunk.is_empty())
        };
        if empty {
            self.chunks.remove(ci);
            self.starts.remove(ci);
            self.max_ends.remove(ci);
        } else {
            if ni == 0 {
                self.starts[ci] = self.chunks[ci][0].start_tick;
            }
            if note.end_tick == self.max_ends[ci] {
                self.refresh_chunk_meta(ci);
            }
        }
        Some(note)
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
                self.max_ends
                    .push(part.iter().map(|n| n.end_tick).max().unwrap_or(0));
                self.chunks.push(Arc::new(part.to_vec()));
            }
            return;
        }
        let mut result: Vec<Arc<Vec<Note>>> = Vec::with_capacity(self.chunks.len() + 1);
        let mut starts = Vec::with_capacity(result.capacity());
        let mut max_ends = Vec::with_capacity(result.capacity());
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
                max_ends.push(part.iter().map(|n| n.end_tick).max().unwrap_or(0));
                result.push(Arc::new(part.to_vec()));
            }
        }
        // 超出旧块范围的新音符，直接成块。
        for part in new[cursor..].chunks(BUCKET_CHUNK_CAP) {
            starts.push(part[0].start_tick);
            max_ends.push(part.iter().map(|n| n.end_tick).max().unwrap_or(0));
            result.push(Arc::new(part.to_vec()));
        }
        self.chunks = result;
        self.starts = starts;
        self.max_ends = max_ends;
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
        self.rebuild_max_ends();
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
        self.rebuild_max_ends();
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
        self.rebuild_max_ends();
    }

    /// 可变 retain：`f` 返回 false 的音符被移除，返回 true 的可就地修改。
    ///
    /// 与 [`NoteBucket::retain`] 相同语义，但省一遍遍历（轨号 remap 这类
    /// "删除 + 重编号"场景一趟完成）。调用方需保证修改不破坏 start_tick 序。
    pub fn retain_mut(&mut self, mut f: impl FnMut(&mut Note) -> bool) {
        let mut i = 0;
        while i < self.chunks.len() {
            let chunk = Arc::make_mut(&mut self.chunks[i]);
            chunk.retain_mut(&mut f);
            if chunk.is_empty() {
                self.chunks.remove(i);
            } else {
                i += 1;
            }
        }
        self.rebuild_starts();
        self.rebuild_max_ends();
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
        self.rebuild_max_ends();
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
            self.rebuild_max_ends();
        }
        removed
    }

    /// 按全局唯一 `id` 更新：只深拷贝目标块，修改后重算该块 max_end。
    /// 返回更新后的音符副本。若 `f` 改了 `start_tick`，调用方仍需 `sort()`。
    pub fn update_by_id(&mut self, id: u32, f: impl FnOnce(&mut Note)) -> Option<Note> {
        let idx = self
            .chunks
            .iter()
            .position(|c| c.iter().any(|n| n.id == id))?;
        let chunk = Arc::make_mut(&mut self.chunks[idx]);
        let note = chunk.iter_mut().find(|n| n.id == id)?;
        f(note);
        let updated = *note;
        self.max_ends[idx] = chunk.iter().map(|n| n.end_tick).max().unwrap_or(0);
        Some(updated)
    }

    /// 对匹配 `pred` 的音符原地应用 `f`，**只深拷贝含匹配音符的块**。
    /// 返回是否有改动。修改可能破坏排序，调用方负责随后 [`sort`](Self::sort)。
    ///
    /// 与 `iter_mut()`（无条件深拷贝全部块）的区别：小选区编辑时只触达
    /// 少数块；大选区时与 iter_mut 等价。`pred` 会被调用两遍
    ///（块级预检 + 逐音符应用），只读扫描成本远低于深拷贝。
    pub fn update_matching(
        &mut self,
        mut pred: impl FnMut(&Note) -> bool,
        mut f: impl FnMut(&mut Note),
    ) -> bool {
        let mut touched = false;
        let mut touched_chunks = Vec::new();
        for (idx, chunk) in self.chunks.iter_mut().enumerate() {
            if !chunk.iter().any(&mut pred) {
                continue;
            }
            for n in Arc::make_mut(chunk).iter_mut() {
                if pred(n) {
                    f(n);
                    touched = true;
                }
            }
            touched_chunks.push(idx);
        }
        for idx in touched_chunks {
            self.max_ends[idx] = self.chunks[idx]
                .iter()
                .map(|n| n.end_tick)
                .max()
                .unwrap_or(0);
        }
        touched
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

    /// retain_mut：删除与就地重编号一趟完成，空块被移除且 starts 重算。
    #[test]
    fn retain_mut_remaps_and_removes_empty_chunks() {
        let mut b =
            NoteBucket::from_sorted((0..3u32).map(|i| note(i, i * 100)).collect::<Vec<_>>());
        b.retain_mut(|n| {
            if n.id == 1 {
                return false; // 删除
            }
            n.track = 7; // 重编号
            true
        });
        assert_eq!(b.len(), 2);
        assert!(b.iter().all(|n| n.track == 7));
        assert!(b.iter().all(|n| n.id != 1));
        assert_sorted(&b);
        assert_eq!(b.first().unwrap().start_tick, 0);
    }

    #[test]
    fn max_ends_track_insert_update_and_removal() {
        let mut b = NoteBucket::from_sorted(vec![note(1, 0)]);
        assert_eq!(b.max_ends, vec![10]);

        b.insert_sorted(Note {
            id: 2,
            start_tick: 5,
            end_tick: 1_000,
            velocity: 100,
            track: 0,
        });
        assert_eq!(b.max_ends, vec![1_000]);

        b.update_by_id(1, |n| n.end_tick = 50);
        assert_eq!(b.max_ends, vec![1_000]);

        b.remove_by_id(2);
        assert_eq!(b.max_ends, vec![50]);
    }

    /// 块级 max_end：前面整块都在视口左侧结束时直接跳过，长音符只影响自己的块。
    #[test]
    fn range_including_long_skips_chunks_ending_before_viewport() {
        let mut notes: Vec<Note> = (0..BUCKET_CHUNK_CAP as u32).map(|i| note(i, i)).collect();
        // 第二个块从 1_000_000 开始。
        notes.extend((0..10u32).map(|i| note(1_000_000 + i, 1_000_000 + i)));
        let b = NoteBucket::from_sorted(notes);

        let first = b
            .range_including_long(1_000_000, 1_000_100)
            .next()
            .expect("应命中第二块");
        assert_eq!(first.start_tick, 1_000_000, "第一块应被整块跳过");
        assert_eq!(b.chunks_in_range(1_000_000, 1_000_100).len(), 1);
        assert!(b.chunks_in_range(900_000, 900_100).is_empty());
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
    fn drain_range_including_long_keeps_note_starting_far_before_window() {
        let mut b = NoteBucket::from_sorted(vec![
            Note {
                end_tick: 10_000,
                ..note(1, 0)
            },
            note(2, 9_000),
            note(3, 20_000),
        ]);
        let removed = b.drain_range_including_long(9_500, 9_600, |n| n.end_tick > 9_500);
        assert_eq!(removed.len(), 1, "长音符起点远早于窗口也必须命中");
        assert_eq!(removed[0].id, 1);
        assert_eq!(b.len(), 2);
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

    /// 游标顺序遍历：整曲每个音符恰好访问一次，跨块/空块都安全。
    #[test]
    fn cursor_walks_every_note_once() {
        let notes: Vec<Note> = (0..(BUCKET_CHUNK_CAP * 2 + 7) as u32)
            .map(|i| note(i, i * 3))
            .collect();
        let total = notes.len();
        let b = NoteBucket::from_sorted(notes);
        let mut c = NoteCursor::default();
        let mut seen = 0usize;
        while let Some((nc, n)) = b.peek(c) {
            assert_eq!(n.id as usize, seen);
            c = nc;
            b.advance(&mut c);
            seen += 1;
        }
        assert_eq!(seen, total);

        // 全空桶 / 空桶在中间：peek 必须跳过。
        let empty = NoteBucket::default();
        assert!(empty.peek(NoteCursor::default()).is_none());
        let mut b = NoteBucket::from_sorted(vec![note(1, 10)]);
        b.chunks.push(Arc::new(Vec::new()));
        b.starts.push(20);
        b.chunks.push(Arc::new(vec![note(2, 30)]));
        b.starts.push(30);
        let mut c = NoteCursor::default();
        let mut ids = Vec::new();
        while let Some((nc, n)) = b.peek(c) {
            ids.push(n.id);
            c = nc;
            b.advance(&mut c);
        }
        assert_eq!(ids, vec![1, 2]);
    }

    /// `cursor_at(tick)` = 第一个 `start_tick >= tick` 的位置。
    #[test]
    fn cursor_at_is_first_not_before_tick() {
        let notes: Vec<Note> = [0u32, 5, 5, 9, 30, 1000]
            .iter()
            .enumerate()
            .map(|(i, &t)| note(i as u32, t))
            .collect();
        let b = NoteBucket::from_sorted(notes);
        for (tick, want) in [
            (0u32, 0u32),
            (1, 1),
            (5, 1),
            (6, 3),
            (9, 3),
            (10, 4),
            (1000, 5),
            (1001, 6),
        ] {
            let (_, n) = match b.peek(b.cursor_at(tick)) {
                Some(v) => v,
                None => {
                    assert_eq!(want, 6, "tick={tick} 应到桶尾");
                    continue;
                }
            };
            assert_eq!(
                n.start_tick,
                [0, 5, 5, 9, 30, 1000][want as usize],
                "tick={tick}"
            );
        }
    }

    /// `cursor_after` 跳过同一 tick 的全部音符（块末重定位不重复派发）。
    #[test]
    fn cursor_after_skips_notes_on_the_same_tick() {
        let notes: Vec<Note> = [0u32, 5, 5, 9, 30]
            .iter()
            .enumerate()
            .map(|(i, &t)| note(i as u32, t))
            .collect();
        let b = NoteBucket::from_sorted(notes);
        let mut c = b.cursor_after(5);
        let (_, n) = b.peek(c).unwrap();
        assert_eq!(n.start_tick, 9);
        c = b.cursor_after(30);
        assert!(b.peek(c).is_none(), "越过最后一个音符");
        c = b.cursor_after(100);
        assert!(b.peek(c).is_none());
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

    fn note_at(id: u32, start: u32, end: u32, track: u16) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: end,
            velocity: 100,
            track,
        }
    }

    /// 同 start 被块切分到两个块时，两个目标都能被 `locate_at` 命中。
    #[test]
    fn locate_at_handles_equal_start_split() {
        let notes: Vec<Note> = (0..BUCKET_CHUNK_CAP + 1)
            .map(|i| note_at(i as u32, 0, 10, 0))
            .collect();
        let b = NoteBucket::from_sorted(notes);
        assert_eq!(b.chunks.len(), 2, "应切成两个块");
        assert_eq!(b.locate_at(0, 0, Some(0)), Some((0, 0)));
        assert_eq!(
            b.locate_at(0, 0, Some(BUCKET_CHUNK_CAP as u32)),
            Some((1, 0)),
            "第二块的目标不能被 chunk_index_for 漏掉"
        );
        assert!(b.locate_at(0, 1, None).is_none());
    }

    #[test]
    fn update_at_keeps_max_ends_exact() {
        let mut b = NoteBucket::from_sorted(vec![note(1, 0), note_at(2, 5, 1000, 0)]);
        assert_eq!(b.max_ends, vec![1000]);

        let (old, new) = b.update_at(0, 0, 1, |n| n.end_tick = 2000).expect("应命中");
        assert_eq!(old.end_tick, 10);
        assert_eq!(new.end_tick, 2000);
        assert_eq!(b.max_ends, vec![2000]);

        b.update_at(0, 0, 1, |n| n.end_tick = 1).expect("应命中");
        assert_eq!(b.max_ends, vec![1000], "缩小非最大 end 不应重算");

        b.update_at(5, 0, 2, |n| n.end_tick = 1).expect("应命中");
        assert_eq!(b.max_ends, vec![1], "删掉最大 end 后应精确收缩");
    }

    #[test]
    fn update_at_start_crosses_chunk() {
        let notes: Vec<Note> = (0..BUCKET_CHUNK_CAP + 10)
            .map(|i| note_at(i as u32, i as u32, i as u32 + 10, 0))
            .collect();
        let mut b = NoteBucket::from_sorted(notes);
        assert!(b.chunks.len() > 1);

        let (old, new) = b
            .update_at(0, 0, 0, |n| {
                n.start_tick = 70_000;
                n.end_tick = 70_010;
            })
            .expect("应命中");
        assert_eq!(old.start_tick, 0);
        assert_eq!(new.start_tick, 70_000);
        assert!(b.is_sorted(), "块间/块内必须保持有序");
        assert_eq!(b.get(0).unwrap().id, 1, "原块首应变成 id 1");
        assert_eq!(b.iter().find(|n| n.id == 0).unwrap().start_tick, 70_000);
        assert_eq!(b.max_end_tick(), 70_010);
    }

    #[test]
    fn remove_at_updates_starts_and_removes_empty_chunk() {
        let mut b = NoteBucket::from_sorted(vec![note(1, 10), note(2, 20), note(3, 30)]);
        let removed = b.remove_at(10, 0, 1).expect("应命中");
        assert_eq!(removed.id, 1);
        assert_eq!(b.starts, vec![20]);
        assert!(b.is_sorted());

        let mut one = NoteBucket::from_sorted(vec![note(9, 100)]);
        assert_eq!(one.remove_at(100, 0, 9).unwrap().id, 9);
        assert!(one.is_empty());
        assert!(one.starts.is_empty());
        assert!(one.max_ends.is_empty());
    }

    /// 单点修改只深拷贝触达块，未触达块保持 Arc 共享。
    #[test]
    fn update_at_copies_only_touched_chunk() {
        let notes: Vec<Note> = (0..BUCKET_CHUNK_CAP * 2)
            .map(|i| note_at(i as u32, i as u32, i as u32 + 10, 0))
            .collect();
        let mut b = NoteBucket::from_sorted(notes);
        let snapshot = b.clone();
        assert_eq!(b.chunks.len(), 2);

        b.update_at(0, 0, 0, |n| n.end_tick = 999_999)
            .expect("应命中");
        assert!(
            !Arc::ptr_eq(&b.chunks[0], &snapshot.chunks[0]),
            "触达块应被复制"
        );
        assert!(
            Arc::ptr_eq(&b.chunks[1], &snapshot.chunks[1]),
            "未触达块必须保持共享"
        );
    }

    #[test]
    fn update_at_and_remove_at_random_oracle() {
        let mut rng = 0x1234_5678_9ABC_DEF0u64;
        let mut next = || {
            rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };

        let mut notes: Vec<Note> = (0..2000u32).map(|i| note_at(i, i, i + 10, 0)).collect();
        notes.sort_by_key(|n| n.start_tick);
        let mut b = NoteBucket::from_sorted(notes);

        for _ in 0..300 {
            let id = (next() % 2000) as u32;
            let Some(n) = b.iter().find(|n| n.id == id).copied() else {
                continue;
            };
            if next() % 3 == 0 {
                let removed = b.remove_at(n.start_tick, n.track, id).expect("应命中");
                assert_eq!(removed.id, id);
            } else {
                let new_start = (next() % 3000) as u32;
                let new_end = new_start + 10;
                b.update_at(n.start_tick, n.track, id, |x| {
                    x.start_tick = new_start;
                    x.end_tick = new_end;
                })
                .expect("应命中");
            }
            assert!(b.is_sorted(), "随机操作后必须有序");
            assert_eq!(b.len(), b.iter().count());
            let max_end = b.iter().map(|n| n.end_tick).max().unwrap_or(0);
            assert_eq!(b.max_end_tick(), max_end, "max_end 必须精确");
        }
    }
}
