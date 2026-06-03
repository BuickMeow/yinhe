use crate::NoteSource;

/// A time signature event at a specific tick position.
#[derive(Clone, Debug)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    /// Denominator as power of 2: 2 means 4 (2^2).
    pub denominator: u8,
}

/// Non-note MIDI events (CC, Program Change, Pitch Bend) stored for audio synthesis.
#[derive(Clone, Debug)]
pub enum MidiControlEvent {
    ControlChange {
        tick: u32,
        channel: u8,
        controller: u8,
        value: u8,
        track: u16,
    },
    ProgramChange {
        tick: u32,
        channel: u8,
        program: u8,
        track: u16,
    },
    PitchBend {
        tick: u32,
        channel: u8,
        value: i16,
        track: u16,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Note {
    pub key: u8,
    pub start: f64,
    pub end: f64,
    pub start_tick: u32,
    pub end_tick: u32,
    pub velocity: u8,
    pub channel: u8,
    pub track: u16,
}

// ── Scan index for fast seeking ────────────────────────────────────

/// Default scan index block size in ticks.
const DEFAULT_BLOCK_SIZE: u32 = 256;

/// One block in the scan index.
#[derive(Clone, Debug)]
pub struct ScanBlock {
    /// Index into `key_notes[key]` where notes whose `start_tick` falls
    /// in this block begin.
    pub block_start_note: usize,
    /// Maximum `end_tick` among all notes whose `start_tick` falls in
    /// blocks 0 ..= current_block (cumulative).
    pub cumulative_max_end: u32,
}

/// Block-based scan index that accelerates searching for notes visible
/// within a tick range.  Built once when a MIDI file is loaded and reused
/// every frame.
#[derive(Clone, Debug)]
pub struct NoteScanIndex {
    pub block_size: u32,
    /// Per key 0..127: one `ScanBlock` per tick-block.  Empty Vec if the
    /// key has no notes.
    pub key_blocks: [Vec<ScanBlock>; 128],
}

impl NoteScanIndex {
    /// Build a scan index from the per-key note lists.
    ///
    /// `key_notes` must be sorted by `start_tick` within each key.
    /// `max_tick` is the last tick of any note (used to size the index).
    pub fn build(key_notes: &[Vec<Note>; 128], max_tick: u64) -> Self {
        let block_size = DEFAULT_BLOCK_SIZE;
        let num_blocks = ((max_tick + block_size as u64 - 1) / block_size as u64).max(1) as usize;

        let mut key_blocks: [Vec<ScanBlock>; 128] = core::array::from_fn(|_| Vec::new());

        // Build per-key block indices in one pass
        for key in 0..128u8 {
            let notes = &key_notes[key as usize];
            if notes.is_empty() {
                continue;
            }
            let blocks = &mut key_blocks[key as usize];
            blocks.reserve_exact(num_blocks);

            let mut note_idx = 0usize;
            let mut cumulative_max = 0u32;
            for block in 0..num_blocks {
                let block_tick = (block as u32) * block_size;
                // Record the first note index for this block
                let block_start_note = note_idx;

                // Include all notes whose start_tick falls in this block
                while note_idx < notes.len() && notes[note_idx].start_tick < block_tick + block_size {
                    cumulative_max = cumulative_max.max(notes[note_idx].end_tick);
                    note_idx += 1;
                }

                blocks.push(ScanBlock {
                    block_start_note,
                    cumulative_max_end: cumulative_max,
                });
            }
        }

        Self {
            block_size,
            key_blocks,
        }
    }
}

/// Find the index of the first note whose `end_tick >= min_tick` for a
/// given key, using the scan index when available.
///
/// Uses binary search on the monotonically non-decreasing
/// `cumulative_max_end` field to correctly handle "crossing" notes —
/// notes that start before `min_tick` but extend far enough to be visible.
///
/// Falls back to 0 when no scan index is available (backwards-compatible).
pub fn seek_first_note(key: u8, source: &dyn NoteSource, min_tick: u32) -> usize {
    let notes = source.key_notes(key);
    if notes.is_empty() {
        return 0;
    }
    let Some(idx) = source.scan_index() else {
        return 0; // no index → scan from start
    };
    let blocks = &idx.key_blocks[key as usize];
    if blocks.is_empty() {
        return 0;
    }

    // Quick check: if no note reaches min_tick, skip entirely.
    if blocks.last().unwrap().cumulative_max_end < min_tick {
        return notes.len();
    }

    // Binary search for the first block where cumulative_max_end >= min_tick.
    let mut lo = 0usize;
    let mut hi = blocks.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if blocks[mid].cumulative_max_end < min_tick {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    blocks[lo].block_start_note
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock NoteSource for testing.
    struct MockSource {
        notes: [Vec<Note>; 128],
        index: Option<NoteScanIndex>,
    }

    impl NoteSource for MockSource {
        fn key_notes(&self, key: u8) -> &[Note] { &self.notes[key as usize] }
        fn duration(&self) -> f64 { 10.0 }
        fn scan_index(&self) -> Option<&NoteScanIndex> { self.index.as_ref() }
    }

    fn make_note(key: u8, start_tick: u32, end_tick: u32) -> Note {
        Note {
            key,
            start_tick,
            end_tick,
            velocity: 100,
            channel: 0,
            track: 0,
            start: 0.0,
            end: 0.0,
        }
    }

    #[test]
    fn test_scan_index_build_and_seek() {
        let mut notes: [Vec<Note>; 128] = core::array::from_fn(|_| Vec::new());
        // Key 60: three notes at ticks 0-480, 480-960, 1000-1500
        notes[60] = vec![
            make_note(60, 0, 480),
            make_note(60, 480, 960),
            make_note(60, 1000, 1500),
        ];
        let index = NoteScanIndex::build(&notes, 1500);
        let source = MockSource { notes, index: Some(index) };

        // seek_first_note(60, min_tick=500) should skip the first note
        let idx = seek_first_note(60, &source, 500);
        assert!(idx <= 1, "Should skip note ending at 480, got {}", idx);
    }

    #[test]
    fn test_scan_index_seek_empty_key() {
        let notes: [Vec<Note>; 128] = core::array::from_fn(|_| Vec::new());
        let source = MockSource { notes, index: None };
        let idx = seek_first_note(60, &source, 0);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_scan_index_seek_past_all_notes() {
        let mut notes: [Vec<Note>; 128] = core::array::from_fn(|_| Vec::new());
        notes[60] = vec![make_note(60, 0, 100)];
        let index = NoteScanIndex::build(&notes, 100);
        let source = MockSource { notes, index: Some(index) };

        // min_tick far beyond the last note
        let idx = seek_first_note(60, &source, 9999);
        assert_eq!(idx, 1, "Should return notes.len() when past all notes");
    }
}
