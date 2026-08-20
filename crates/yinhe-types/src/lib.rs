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
pub use piano_roll_view::{Orientation, PianoRollView};
pub use source::NoteSource;
pub use time_format::{
    build_time_sig_segments, compute_measure_divisor, measure_bounds_at_tick, measure_ticks,
};
pub use view_base::TimelineViewBase;

/// 内部音高空间的 key 总数（桶数）。标准 MIDI 只有 128 键（0-127），
/// yinhe 内部支持 256 键（0-255）：128-255 是扩展音域，数据无损保存，
/// 仅在导出标准 MIDI / 标准 MIDI 设备发声时受 7 位上限约束。
pub const KEY_COUNT: usize = 256;

/// 最大合法 key（桶下标上界，含）。
pub const MAX_KEY: u8 = (KEY_COUNT - 1) as u8;

/// 标准 MIDI 的 key 数（0-127）。仅 MIDI 导入/导出/设备发声边界使用。
pub const STANDARD_MIDI_KEY_COUNT: usize = 128;

/// Returns true if the given key (0–255) is a black key on a piano.
pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

/// 升号音名表：`NOTE_NAMES[i]` 是音级 i 的名字。
pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// 把 key（0..=255）格式化为 "C5" 式音名。
///
/// 八度约定与 PR 键盘（piano_view/keyboard.rs）一致：octave = key / 12，
/// 即 key 60 标注为 "C5"。
pub fn key_name(key: u8) -> String {
    format!("{}{}", NOTE_NAMES[(key % 12) as usize], key / 12)
}
