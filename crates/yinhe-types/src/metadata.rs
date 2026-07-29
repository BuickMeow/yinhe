//! Score-level metadata event types: key signature, marker, lyrics, chord.
//!
//! 这些事件类型与 `TimeSigEvent` 同级，用于承载 SMF meta 事件中的
//! 调号 (FF 59)、标记 (FF 06/07)、歌词 (FF 05) 以及和弦（非 SMF 标准，
//! 用 Text meta 或自定义格式存储）。

use serde::{Deserialize, Serialize};

/// 调号事件（MIDI Meta FF 59）。
///
/// `sf` = 升降号数：正数 =升号数（+1 = G major / E minor），负数 = 降号数
///（-1 = F major / D minor），0 = C major / A minor。范围 -7..=7。
/// `mi` = 模式：0 = 大调，1 = 小调。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeySigEvent {
    pub tick: u32,
    pub sf: i8,
    pub mi: u8,
}

/// 标记事件（MIDI Meta FF 06 Marker / FF 07 CueMarker）。
///
/// SMF 标准中 Marker 和 CueMarker 都是文本标记，区别在于 DAW 的显示方式。
/// 这里统一存储，不区分 kind——Marker 一般用于段落标记（如 "Verse"、"Chorus"）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkerEvent {
    pub tick: u32,
    pub text: String,
}

/// 歌词事件（MIDI Meta FF 05 Lyrics）。
///
/// SMF 标准中歌词是 per-track 的，每个 syllable 一个事件。
/// 这里存储原始文本片段，不做 syllable 拆分。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LyricsEvent {
    pub tick: u32,
    pub text: String,
}

/// 和弦事件（非 SMF 标准）。
///
/// SMF 没有专门的和弦 meta event。部分 DAW 用 Text (FF 01) 以特定格式
/// 表示和弦（如 "Cmaj7"、"Am7b5"）。这里用 `text` 字段存储和弦名称，
/// 解析/写出时走 Text meta。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChordEvent {
    pub tick: u32,
    pub text: String,
}
