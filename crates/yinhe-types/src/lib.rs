//! Core MIDI data types shared across the yinhe workspace.

pub mod automation;
mod note;
pub mod palette;
mod source;
pub mod view_base;

pub use automation::{AutomationEvent, AutomationLane, AutomationTarget};
pub use note::{MidiControlEvent, Note, NoteScanIndex, ScanBlock, TimeSigEvent, seek_first_note};
pub use palette::TRACK_PALETTE;
pub use source::NoteSource;
pub use view_base::TimelineViewBase;

/// Returns true if the given MIDI key (0–127) is a black key on a piano.
pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}
