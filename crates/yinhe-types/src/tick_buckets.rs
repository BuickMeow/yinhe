use crate::Note;

/// Coarse tick buckets for fast visible-note range queries.
///
/// Each key's notes (already sorted by `start_tick`) are logically divided into
/// fixed-size tick blocks.  For every block we store the index range into
/// `key_notes[key]` and the maximum `end_tick` of any note whose *start* falls
/// in that block.  This lets renderers skip huge swaths of off-screen notes
/// without scanning from the beginning of the key.
///
/// The bucket size is intentionally much larger than the `NoteScanIndex` block
/// size (256 ticks): it is meant to cull at the "screenful" level, not to
/// replace the fine-grained scan index.
#[derive(Clone, Debug)]
pub struct TickBuckets {
    pub block_size: u32,
    pub key_blocks: [Vec<Bucket>; 128],
}

#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    /// Index into `key_notes[key]` of the first note whose `start_tick` falls
    /// in this bucket.
    pub start_idx: usize,
    /// Index one past the last note whose `start_tick` falls in this bucket.
    pub end_idx: usize,
    /// Maximum `end_tick` among notes whose `start_tick` falls in this bucket.
    pub max_end: u32,
}

impl TickBuckets {
    /// Build buckets from per-key note lists.
    ///
    /// `key_notes` must be sorted by `start_tick` within each key, which is the
    /// invariant already maintained by `MidiFile`.
    pub fn build(key_notes: &[Vec<Note>; 128], max_tick: u64, block_size: u32) -> Self {
        assert!(block_size > 0, "bucket block_size must be > 0");
        let num_blocks = max_tick.div_ceil(block_size as u64).max(1) as usize;
        let mut key_blocks: [Vec<Bucket>; 128] = core::array::from_fn(|_| Vec::new());

        for key in 0..128usize {
            let notes = &key_notes[key];
            if notes.is_empty() {
                continue;
            }
            let buckets = &mut key_blocks[key];
            buckets.reserve_exact(num_blocks);

            let mut note_idx = 0usize;
            for block in 0..num_blocks {
                let block_end_tick = ((block as u64 + 1) * block_size as u64)
                    .min(u32::MAX as u64) as u32;
                let start_idx = note_idx;
                let mut max_end = 0u32;

                while note_idx < notes.len() && notes[note_idx].start_tick < block_end_tick {
                    max_end = max_end.max(notes[note_idx].end_tick);
                    note_idx += 1;
                }

                buckets.push(Bucket {
                    start_idx,
                    end_idx: note_idx,
                    max_end,
                });
            }
        }

        Self {
            block_size,
            key_blocks,
        }
    }

    /// Return the index range `[start, end)` into `key_notes[key]` that may
    /// intersect `[tick_start, tick_end]`.
    ///
    /// The range is conservative: it is guaranteed to contain every note whose
    /// visible interval overlaps the requested tick range.  It may also contain
    /// a small number of extra notes that start shortly before `tick_start` but
    /// end after it ("crossing" notes), and possibly a few notes whose
    /// `start_tick` is just past `tick_end`; callers must still do their own
    /// pixel/viewport culling.
    pub fn range_for(&self, key: u8, tick_start: u32, tick_end: u32) -> (usize, usize) {
        let buckets = &self.key_blocks[key as usize];
        if buckets.is_empty() {
            return (0, 0);
        }
        if tick_start > tick_end {
            return (0, 0);
        }

        let start_block = (tick_start / self.block_size) as usize;
        if start_block >= buckets.len() {
            let last = buckets.last().unwrap();
            return (last.end_idx, last.end_idx);
        }

        // Walk backwards from start_block to include notes that started in an
        // earlier bucket but are long enough to reach tick_start.
        let mut start_idx = buckets[start_block].start_idx;
        for block in (0..start_block).rev() {
            if buckets[block].max_end < tick_start {
                break;
            }
            start_idx = buckets[block].start_idx;
        }

        let end_block = ((tick_end / self.block_size) as usize).min(buckets.len() - 1);
        let end_idx = buckets[end_block].end_idx;

        (start_idx, end_idx)
    }
}
