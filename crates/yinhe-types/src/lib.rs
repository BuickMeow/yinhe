//! Core MIDI data types shared across the yinhe workspace.

mod note;
pub mod palette;
mod source;

pub use note::{MidiControlEvent, Note, NoteScanIndex, ScanBlock, TimeSigEvent, seek_first_note};
pub use palette::TRACK_PALETTE;
pub use source::NoteSource;

/// Returns true if the given MIDI key (0–127) is a black key on a piano.
pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}
