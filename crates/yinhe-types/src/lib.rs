//! Core MIDI data types shared across the yinhe workspace.

pub mod arr_row_layout;
pub mod arrangement_view;
pub mod automation;
pub mod automation_panel_view;
pub mod hash;
pub mod metadata;
mod note;
pub mod note_bucket;
pub mod palette;
pub mod pc_event;
pub mod piano_roll_view;
mod source;
pub mod time_format;
pub mod view_base;

pub use arr_row_layout::{ArRow, ArRowLayout};
pub use arrangement_view::ArrangementView;
pub use automation::{
    AmMsState, AutomationEdit, AutomationEvent, AutomationLane, AutomationTarget, SegmentShape,
};
pub use automation_panel_view::{AnchorSelRect, AutomationPanelView};
pub use hash::*;
pub use metadata::{ChordEvent, KeySigEvent, LyricsEvent, MarkerEvent, ScaleType, from_midi_sf_mi};
pub use note::{Note, PencilNoteDrag, TimeSigEvent, VelocityEdit};
pub use note_bucket::{BUCKET_CHUNK_CAP, NoteBucket, NoteRangeIter};
pub use palette::TRACK_PALETTE;
pub use pc_event::PcEvent;
pub use piano_roll_view::PianoRollView;
pub use source::NoteSource;
pub use time_format::{
    build_time_sig_segments, compute_measure_divisor, measure_bounds_at_tick, measure_ticks,
};
pub use view_base::TimelineViewBase;

/// Returns true if the given MIDI key (0–127) is a black key on a piano.
pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

/// 升号音名表：`NOTE_NAMES[i]` 是音级 i 的名字。
pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// 把 MIDI key（0..=127）格式化为 "C5" 式音名。
///
/// 八度约定与 PR 键盘（piano_view/keyboard.rs）一致：octave = key / 12，
/// 即 key 60 标注为 "C5"。
pub fn key_name(key: u8) -> String {
    format!("{}{}", NOTE_NAMES[(key % 12) as usize], key / 12)
}
