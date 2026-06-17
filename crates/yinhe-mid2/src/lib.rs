//! Standard MIDI File (.mid) ↔ `yinhe_core::YinModel`.
//!
//! 不经过任何 MidiFile 中间结构。parser 直接从 midly 的 SMF 输出
//! 聚合成 YinModel。writer 反向，从 YinModel 直接产出 SMF。

mod encoding;
mod error;
mod parser;
mod writer;

pub use encoding::MidiImportEncoding;
pub use error::MidiError;
pub use parser::{LoadProgress, parse_bytes, parse_bytes_with_encoding, parse_path};
pub use writer::write_to_bytes;
