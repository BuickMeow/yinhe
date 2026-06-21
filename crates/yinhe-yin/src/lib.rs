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
pub const VERSION: u16 = 2;
