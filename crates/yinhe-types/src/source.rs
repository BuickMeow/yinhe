use crate::{AutomationLane, Note, TimeSigEvent};

pub trait NoteSource: Sync {
    fn key_notes(&self, key: u8) -> &[Note];
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
    /// Automation lanes built from control events (CC, PB, RPN, Velocity).
    fn automation_lanes(&self) -> &[AutomationLane] {
        &[]
    }

    /// Return the slice of notes for `key` that may intersect `[tick_start, tick_end]`.
    ///
    /// Uses binary search (`partition_point`) on the sorted-by-start_tick note list.
    /// The returned slice is conservative: callers must still perform their own
    /// viewport/pixel culling.
    fn key_notes_in_range(&self, key: u8, tick_start: u32, tick_end: u32) -> &[Note] {
        let notes = self.key_notes(key);
        if notes.is_empty() || tick_start > tick_end {
            return &[];
        }

        let end = notes.partition_point(|n| n.start_tick <= tick_end);
        &notes[0..end]
    }
}
