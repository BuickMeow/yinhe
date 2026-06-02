use crate::{Note, NoteScanIndex};

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
    /// Optional scan index for fast seeking to visible notes.
    fn scan_index(&self) -> Option<&NoteScanIndex> {
        None
    }
}
