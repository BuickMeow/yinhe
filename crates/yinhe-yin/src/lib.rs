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
//! data:         [u8; data_len]     (见下)
//! ```
//!
//! `project.json` 和 `mapping.json` 携带人类可读元数据，不压缩；
//! `data` 段 = 6 个 (len u32 LE + zstd 块)：
//! ```text
//! 0: postcard(conductor + tracks payload)   ← 非音符部分
//! 1: delta 列（varint u32：第一音符绝对 start，其余 = start - prev）
//! 2: key   列（u8 × N）
//! 3: track 列（varint u16）
//! 4: vel   列（u8 × N）
//! 5: gate  列（varint u32）
//! ```
//!
//! v6 设计：
//! - 音符全局按 (start, track, key) 排序后**列式**存储：黑乐谱的重复单元是
//!   同一 tick 全轨齐发的图案，该排序让图案整块重复；按字段拆列后每列独立
//!   zstd，避免交错流互相稀释。实测 1.64 亿音符（start.mid）：v4 key 桶
//!   75MB → 列式 40.8MB（zstd3）/ 13.5MB（zstd19）；4444 万音符
//!   （Broken World）37.3MB → 10.5MB / 4.65MB
//! - 不序列化音符 id：id 是会话内身份（undo/selection/音频匹配），加载时
//!   由 `load_bucket_notes` 重新分配；全局递增 id 在 zstd 下几乎压不动
//! - 压缩级别存 `project.json`（compression_level，默认 3，UI 可调）
//! - v6 起二进制段由 `bincode` 切 `postcard`（`bincode` 已停止维护）；不兼容 v5 及更早文件
//! - 不兼容旧文件（v1-v5 不提供读取，快速迭代期）

mod container;
mod error;
mod io;
mod mapping;
mod project_meta;

pub use error::YinError;
pub use io::{
    ProjectSoundFonts, YinProgress, YinProgressStage, load_yin, load_yin_bytes,
    load_yin_bytes_with_sf, load_yin_with_sf, load_yin_with_sf_progress, save_yin, save_yin_bytes,
    save_yin_bytes_with_sf, save_yin_with_files, save_yin_with_files_progress, save_yin_with_sf,
};
pub use mapping::{ChannelMap, MappingFile, PortMap, TrackMap};
pub use project_meta::{ProjectFile, SfEntryJson, SfPortOverride};

pub const MAGIC: &[u8; 4] = b"YINH";
pub const VERSION: u16 = 6;
