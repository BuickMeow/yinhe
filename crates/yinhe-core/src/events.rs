use serde::{Deserialize, Serialize};

/// A note event stored per-track.
///
/// Channel / track are NOT stored here — they are implied by the owning
/// `TrackData`. This saves 3+ bytes per note in dense scores.
///
/// Memory representation uses `start_tick + end_tick` (rather than tick + duration)
/// for fast playback scheduling without addition.
///
/// `id` 是全局唯一身份（由 YinModel 发号器分配，0 = 未分配）。
/// MIDI 解析时填 0，由 `YinModel::load_track_notes` 统一发号；
/// `.yin` 序列化保留 id，加载时若 id=0 则重新分配。
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct NoteEvent {
    pub id: u32,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub velocity: u8,
}


