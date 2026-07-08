//! Core MIDI data types shared across the yinhe workspace.

pub mod arrangement_view;
pub mod automation;
pub mod automation_panel_view;
pub mod hash;
mod note;
pub mod palette;
pub mod pc_event;
pub mod piano_roll_view;
mod source;
pub mod tempo_event;
pub mod time_format;
pub mod view_base;

pub use arrangement_view::ArrangementView;
pub use automation::{AutomationEdit, AutomationEvent, AutomationLane, AutomationTarget, PencilNoteDrag, SegmentShape};
pub use automation_panel_view::AutomationPanelView;
pub use hash::*;
pub use note::{Note, TimeSigEvent};
pub use palette::TRACK_PALETTE;
pub use pc_event::PcEvent;
pub use piano_roll_view::PianoRollView;
pub use source::{key_notes_in_range, NoteSource};
pub use tempo_event::TempoEvent;
pub use time_format::{build_time_sig_segments, measure_ticks};
pub use view_base::TimelineViewBase;

/// Returns true if the given MIDI key (0–127) is a black key on a piano.
pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}
