//! Yinhe DAW 内核数据模型。
//!
//! `YinModel` 是整个项目的唯一内存数据结构。MIDI 仅作为 import/export
//! 适配器，.yin 文件通过 bincode + zstd 序列化。
//!
//! 设计要点：
//! - NoteEvent 不带 channel/track，由所属 TrackData 隐含
//! - 同 (key, start_tick) 重叠音符用 dup_index: u8 区分
//! - 内存用 start_tick + end_tick（播放友好），无 length 加法
//! - C1 模式：tracks 为 Vec<Arc<TrackData>>，编辑时只 clone 受影响的 track
//! - 派生索引 (key_notes_cache 等) 在 rebuild() 时全量重建

mod events;
mod model;
mod source;
mod tempo_map;

pub use events::{CcEvent, NoteEvent, PcEvent, PitchBendEvent, RpnEvent};
pub use model::{ConductorData, ProjectMeta, TempoEvent, TimeSigEvent, TrackData, YinModel};
pub use tempo_map::{
    DEFAULT_BPM, DEFAULT_MPQ, TempoMap, TempoSegment, bar_at_tick, bar_divide, bpm_from_mpq,
    mpq_from_bpm, recompute_tempo_start_times, seconds_to_ticks, ticks_to_seconds, total_bars,
};
