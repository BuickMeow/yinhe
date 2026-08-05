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

/// `.yin` 加载路径的桶内音符：与 `NoteEvent` 的区别是自带 `track`（
/// 桶式存储没有 per-track 容器可隐含 track），且不存 `id`（加载时重新分配）
/// 与 `key`（桶下标即 key）。
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct BucketNote {
    pub track: u16,
    pub start_tick: u32,
    pub end_tick: u32,
    pub velocity: u8,
}
