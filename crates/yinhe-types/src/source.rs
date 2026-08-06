use crate::{NoteBucket, NoteRangeIter, TimeSigEvent};

pub trait NoteSource: Sync {
    /// All notes for `key`, stored as a chunked sorted sequence.
    ///
    /// Consumers iterate (`iter` / `range`) instead of slicing — the notes
    /// are not contiguous in memory (65536-note chunks).
    fn key_notes(&self, key: u8) -> &NoteBucket;
    fn duration(&self) -> f64;
    fn ticks_per_beat(&self) -> Option<u32> {
        None
    }
    fn tick_at_time(&self, _time: f64) -> Option<f64> {
        None
    }
    /// Total tick length (position of the last note end).
    fn tick_length(&self) -> Option<u64> {
        None
    }
    /// Default time signature (numerator, denominator-power).
    fn time_sig_default(&self) -> (u8, u8) {
        (4, 2) // default 4/4
    }
    /// Time signature change events, sorted by tick.
    fn time_sig_events(&self) -> &[TimeSigEvent] {
        &[]
    }
    /// Longest note length (in ticks) across the whole song.
    ///
    /// `key_notes_in_range` uses it to tighten the query's left bound from
    /// the song start to `tick_start - max_note_len`. Without it, a viewport
    /// near the end of a 100M-note song would iterate every note from tick 0
    /// every frame (multi-second stalls in the CPU build path).
    ///
    /// May be monotonically non-decreasing (kept when the longest note is
    /// deleted) — a too-large value only widens the returned range slightly,
    /// never drops notes. The default `u32::MAX` = unknown, which makes the
    /// query degenerate to scanning from the song start (safe, just slow).
    fn max_note_len(&self) -> u32 {
        u32::MAX
    }

    /// Iterate the notes for `key` that may intersect `[tick_start, tick_end]`.
    ///
    /// The right bound is exact (start < tick_end); the left bound is
    /// conservative — it includes every note with `start >= tick_start -
    /// max_note_len`, i.e. anything whose end could still reach `tick_start`.
    /// Callers must still perform their own viewport/pixel culling.
    fn key_notes_in_range(&self, key: u8, tick_start: u32, tick_end: u32) -> NoteRangeIter<'_> {
        let lo = tick_start.saturating_sub(self.max_note_len());
        self.key_notes(key).range(lo, tick_end)
    }
}
