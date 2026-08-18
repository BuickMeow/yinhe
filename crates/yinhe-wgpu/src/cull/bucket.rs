use crate::vertex::NoteInstance;

/// Per-key tick-chunk index over the key's notes (sorted by start_tick).
///
/// Chunk c covers notes [c*256, min((c+1)*256, count)) — contiguous, so the
/// shader computes its input range directly from (c_lo, chunk id) without
/// any GPU lookup table.
///
/// A chunk can intersect the viewport tick range [ts, te] iff
/// chunk_start[c] <= te AND chunk_max_end[c] >= ts. chunk_max_end is not
/// monotonic (a long note anywhere inflates it), so we store **block-level
/// prefix/suffix max** (one entry per BLOCK_CHUNKS chunks):
///   - left bound: block prefix max is non-decreasing → binary search finds
///     the first block whose prefix max >= ts; every earlier chunk's max_end
///     < ts. A forward scan of that block's ≤64 chunks then finds the exact
///     first chunk with max_end >= ts.
///   - right bound: block suffix max is non-increasing → binary search finds
///     the first block whose suffix max < ts; every chunk from there on ends
///     < ts. A backward scan of the previous block finds the exact last chunk
///     with max_end >= ts.
///
/// Long notes only pollute their own block (≤64 chunk comparisons), so the
/// dispatched range is O(viewport) instead of O(song prefix): a viewport in
/// the middle of a 100M-note song no longer scans every note from the song
/// start every frame (that caused multi-second-per-frame stalls at the
/// song's tail).
#[derive(Clone)]
pub(crate) struct KeyBucketIndex {
    /// start_tick of each chunk's first note (monotonic non-decreasing).
    pub(crate) chunk_start: Vec<u32>,
    /// max end_tick within each chunk (not monotonic).
    pub(crate) chunk_max_end: Vec<u32>,
    /// prefix max of chunk_max_end, one entry per BLOCK_CHUNKS chunks
    /// (monotonic non-decreasing).
    pub(crate) block_prefix_max: Vec<u32>,
    /// suffix max of chunk_max_end, one entry per BLOCK_CHUNKS chunks
    /// (monotonic non-increasing).
    pub(crate) block_suffix_max: Vec<u32>,
    /// Total chunk count = ceil(note_count / 256).
    pub(crate) chunk_total: u32,
}

/// Chunks per index block; per-block scan cost ≤ BLOCK_CHUNKS comparisons.
pub(crate) const BLOCK_CHUNKS: usize = 64;

impl KeyBucketIndex {
    /// 编辑场景（upload_one_key）每帧重建，chunk 级计算用 rayon 并行：
    /// 每 chunk 的 max_end 互相独立（256 音符取 max），只有 block 级
    /// 前缀/后缀数组依赖顺序。370 万音符的 key 从 ~2ms 降到 ~0.3ms。
    pub(crate) fn build(notes: &[NoteInstance]) -> Self {
        use rayon::prelude::*;
        let chunk_total = notes.len().div_ceil(256);
        // chunk_start: 每 chunk 首音符的 start_tick（顺序，O(chunk_total) 很快）。
        let mut chunk_start = Vec::with_capacity(chunk_total);
        chunk_start.extend(notes.chunks(256).map(|c| c[0].start_tick));
        // chunk_max_end: 并行（大头）。
        let mut chunk_max_end = vec![0u32; chunk_total];
        chunk_max_end
            .par_iter_mut()
            .enumerate()
            .for_each(|(ci, m)| {
                let start = ci * 256;
                let end = (start + 256).min(notes.len());
                *m = notes[start..end]
                    .iter()
                    .map(|n| n.end_tick)
                    .max()
                    .unwrap_or(0);
            });
        // Block prefix max (non-decreasing).
        let mut block_prefix_max = Vec::new();
        let mut cur = 0;
        for (bi, m) in chunk_max_end.iter().enumerate() {
            cur = cur.max(*m);
            if bi % BLOCK_CHUNKS == BLOCK_CHUNKS - 1 || bi == chunk_max_end.len() - 1 {
                block_prefix_max.push(cur);
            }
        }
        // Block suffix max (non-increasing): max over [block..] of per-block max.
        let n_blocks = chunk_max_end.len().div_ceil(BLOCK_CHUNKS);
        let mut per_block_max = vec![0u32; n_blocks];
        for (bi, m) in chunk_max_end.iter().enumerate() {
            let b = bi / BLOCK_CHUNKS;
            per_block_max[b] = per_block_max[b].max(*m);
        }
        let mut block_suffix_max = Vec::with_capacity(n_blocks);
        let mut cur = 0;
        for &m in per_block_max.iter().rev() {
            cur = cur.max(m);
            block_suffix_max.push(cur);
        }
        block_suffix_max.reverse();
        KeyBucketIndex {
            chunk_total: chunk_max_end.len() as u32,
            chunk_start,
            chunk_max_end,
            block_prefix_max,
            block_suffix_max,
        }
    }

    /// Chunk range [c_lo, c_hi) that can intersect
    /// [tick_start, tick_end]. Conservative: may include chunks that the
    /// shader's exact AABB test then culls. Returns None when nothing can
    /// intersect.
    ///
    /// Correctness: any visible chunk v has chunk_max_end[v] >= ts, so v lies
    /// between the first chunk with max_end >= ts (found by block prefix
    /// search + forward scan) and the last chunk with max_end >= ts (found by
    /// block suffix search + backward scan). Combined with the chunk_start
    /// <= te bound, the interval [c_lo, c_hi) covers every visible chunk.
    pub(crate) fn visible_chunk_range(&self, tick_start: u32, tick_end: u32) -> Option<(u32, u32)> {
        if self.chunk_total == 0 || tick_start > tick_end {
            return None;
        }
        let c_hi_bound = self.chunk_start.partition_point(|&s| s <= tick_end);
        if c_hi_bound == 0 {
            return None;
        }

        // Left bound: first chunk with max_end >= ts.
        let block_lo = self
            .block_prefix_max
            .partition_point(|&m| m < tick_start)
            .saturating_mul(BLOCK_CHUNKS)
            .min(c_hi_bound);
        let scan_end = (block_lo + BLOCK_CHUNKS).min(c_hi_bound);
        let mut c_lo = scan_end;
        for c in block_lo..scan_end {
            if self.chunk_max_end[c] >= tick_start {
                c_lo = c;
                break;
            }
        }

        // Right bound: last chunk with max_end >= ts. The block-level suffix
        // search may point at a block whose max_end >= ts comes from chunks
        // with start > te (past the viewport) — those must still be scanned
        // (they clip the bound), so the backward scan runs over the full
        // block, not just [0, c_hi_bound).
        let block_tail = self.block_suffix_max.partition_point(|&m| m >= tick_start);
        let mut c_hi = c_lo + 1;
        if block_tail > 0 {
            let back_start = (block_tail - 1).saturating_mul(BLOCK_CHUNKS).max(c_lo);
            let back_end = (block_tail * BLOCK_CHUNKS).min(self.chunk_total as usize);
            for c in (back_start..back_end).rev() {
                if self.chunk_max_end[c] >= tick_start {
                    c_hi = c + 1;
                    break;
                }
            }
        }
        let c_hi = c_hi.min(c_hi_bound);
        if c_lo >= c_hi || c_lo >= c_hi_bound {
            return None;
        }
        Some((c_lo as u32, c_hi as u32))
    }
}
