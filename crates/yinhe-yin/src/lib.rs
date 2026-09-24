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
//! `data` 段 = 6 个 (len u32 LE + 块)：
//! ```text
//! 0: zstd(postcard(conductor + tracks payload + segments))  ← 非音符部分
//! 1: delta 列（varint u32：轨段首为绝对 start，其余 = start - prev）
//! 2: key   列（u8 × N）
//! 3: vel   列（u8 × N）
//! 4: gate  列（varint u32）
//! 5: id delta 列（zigzag varint，段内独立：段首为绝对 id）
//! ```
//! 音符列是**分帧流**：`[len u32 LE][zstd 帧]` 重复，帧边界落在音符边界。
//!
//! v9：音符列改分帧流 + 流式加载，内存大幅降低：
//! - 保存：归并时按 track 暂存列缓冲，归并后按 track 升序拼接、攒到 16MB
//!   即压缩一帧（不需要全量 SoA/全局列缓冲）。1.64 亿音符保存峰值
//!   ~6.7GB → ~3.9GB（模型本身 2.6GB）。实测 16MB 分帧压缩率损失 <2%
//! - 加载：逐帧解压、边解析边 `NoteLoader::feed`（不物化全量列 Vec），
//!   `finish` 逐桶排序分块（峰值 ~6.5GB → ~2.7GB）
//! - id delta 从跨段连续改为**段内独立**（段首绝对 id）：流式归并的输出
//!   顺序（按 start）与存储顺序（按 track 段）不同，跨段连续无法单遍算出；
//!   段首绝对值仅 ~4KB，压缩率无影响
//!
//! v8：音符按 **(track, start, key)** 排序的轨段布局（段表在 meta 流），
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
//! - 不兼容旧文件（v1-v8 不提供读取，快速迭代期）

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
pub const VERSION: u16 = 9;
