//! Yinhe DAW 内核数据模型。
//!
//! `YinModel` 是整个项目的唯一内存数据结构。MIDI 仅作为 import/export
//! 适配器，.yin 文件通过 bincode + zstd 序列化。
//!
//! 设计要点：
//! - NoteEvent 不带 channel/track，由所属 TrackData 隐含
//! - 每个音符由全局唯一 id: u32 区分（由 YinModel 发号器分配）
//! - 内存用 start_tick + end_tick（播放友好），无 length 加法
//! - C1 模式：tracks 为 Vec<Arc<TrackData>>，编辑时只 clone 受影响的 track
//! - 派生索引 (key_notes_cache 等) 在 rebuild() 时全量重建

mod events;
mod model;
mod model_stats;
mod selection;
mod source;
mod tempo_map;

pub use events::NoteEvent;
pub use model::{ConductorData, ProjectMeta, TrackData, TrackInfo, YinModel};
pub use selection::Selection;
pub use yinhe_types::{Note, PcEvent};
pub use tempo_map::{
    DEFAULT_BPM, DEFAULT_MPQ, TempoMap, TempoSegment, bar_at_tick, bar_divide, bpm_from_mpq,
    mpq_from_bpm, recompute_tempo_start_times, seconds_to_ticks, ticks_to_seconds, total_bars,
};
