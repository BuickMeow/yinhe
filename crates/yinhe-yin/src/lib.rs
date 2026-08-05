//! Read / write `.yin` files: serialized `yinhe_core::YinModel`.
//!
//! Format (极简版):
//! ```text
//! magic:        b"YINH"            (4 bytes)
//! version:      u16 LE             (2 bytes)
//! project_len:  u32 LE             (4 bytes)
//! project_json: [u8; project_len]  (utf-8 JSON)
//! mapping_len:  u32 LE             (4 bytes)
//! mapping_json: [u8; mapping_len]  (utf-8 JSON)
//! data_len:     u32 LE             (4 bytes)
//! data:         [u8; data_len]     (zstd of bincode(ModelData))
//! ```
//!
//! No column splitting, no archive index, no per-stream files. Just three
//! length-prefixed sections wrapped in a tiny header.
//!
//! `project.json` and `mapping.json` carry human-readable metadata so the
//! file's identity (name, soundfont config, view state) is inspectable
//! without paying the cost of zstd-decoding the full event stream.
//!
//! v4（data 段）设计：
//! - 音符按 key 桶直存（`key_notes[128]`），加载直接入桶；桶内 delta+gate
//!   编码（同 key 相邻 start 差 + 长度），黑乐谱音符极密，转小数后 zstd
//!   压缩率大幅提升
//! - bincode varint 编码（LEB128）：delta/gate 99% < 16 tick，定长 u32
//!   裸数据膨胀 ~2.3 倍，varint 后 11B → ~5B/音符
//! - 不序列化音符 id：id 是会话内身份（undo/selection/音频匹配），加载时
//!   由 `load_bucket_notes` 重新分配；全局递增 id 在 zstd 下几乎压不动
//! - 实测 5.5M 音符 3.22MB、1.64 亿音符 75MB（start.mid，787 轨）

mod container;
mod error;
mod io;
mod mapping;
mod project_meta;

pub use error::YinError;
pub use io::{
    ProjectSoundFonts, load_yin, load_yin_bytes, load_yin_bytes_with_sf, load_yin_with_sf,
    save_yin, save_yin_bytes, save_yin_bytes_with_sf, save_yin_with_files, save_yin_with_sf,
};
pub use mapping::{ChannelMap, MappingFile, PortMap, TrackMap, ViewState};
pub use project_meta::{ProjectFile, SfEntryJson, SfPortOverride};

pub const MAGIC: &[u8; 4] = b"YINH";
pub const VERSION: u16 = 4;
