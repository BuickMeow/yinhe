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
//! 0: postcard(conductor + tracks payload + segments)  ← 非音符部分
//! 1: delta 列（varint u32：轨段首为绝对 start，其余 = start - prev）
//! 2: key   列（u8 × N）
//! 3: vel   列（u8 × N）
//! 4: gate  列（varint u32）
//! 5: id delta 列（zigzag varint 字节流，跨段连续累加）
//! ```
//!
//! v8：音符改按 **(track, start, key)** 排序的轨段布局（段表在 meta 流），
//! 并落盘音符 id：
//! - 黑乐谱的重复单元是「单轨内乐句复现」，v6 的全局 (start, track, key)
//!   排序会把同一轨的音符隔到全曲其他轨之后（重复距离 ≈ 全曲音符数），
//!   超出 zstd 窗口；轨内串行后重复在轨内近距离匹配。实测 start.mid
//!   1.64 亿音符：v7 同款布局 38.9MB → 轨段列式 5.0MB（zstd3，-87%），
//!   4444 万音符（Broken World）10.1MB → 2.2MB
//! - id 落盘：导入时 id 按 track 顺序分配，与轨段布局同序 → delta 恒为 1，
//!   zigzag varint + zstd 后 1.64 亿音符仅 ~5KB（+0.1%）。id 不再每次
//!   加载重分配（跨会话稳定），但发号器仍推进到 max+1 供编辑新增使用
//! - 压缩级别存 `project.json`（compression_level，默认 3，UI 可调）
//! - 不兼容旧文件（v1-v7 不提供读取，快速迭代期）

mod audio_section;
mod codec;
mod container;
mod data_section;
mod error;
mod io;
mod mapping;
mod mixer_section;
mod progress;
mod project_meta;

pub use error::YinError;
pub use io::{
    ProjectSoundFonts, load_yin, load_yin_bytes, load_yin_bytes_with_sf, load_yin_with_sf,
    load_yin_with_sf_progress, save_yin, save_yin_bytes, save_yin_bytes_with_sf,
    save_yin_with_files, save_yin_with_files_progress, save_yin_with_sf,
};
pub use mapping::{ChannelMap, MappingFile, PortMap, TrackMap};
pub use progress::{YinProgress, YinProgressStage};
pub use project_meta::{ProjectFile, SfChannelOverride, SfEntryJson};

pub const MAGIC: &[u8; 4] = b"YINH";
pub const VERSION: u16 = 8;
