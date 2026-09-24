//! `project.json` — top-level human-readable project metadata.

use serde::{Deserialize, Serialize};

use yinhe_core::ProjectMeta;

/// A single SoundFont entry stored in `project.json` (path + display name + enabled flag).
///
/// Mirrors `yinhe_editor_core::config::SfEntry` but lives in this crate to
/// keep the file format crate self-contained.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SfEntryJson {
    pub path: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Project-level soundfont override for one source channel (0..256).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfChannelOverride {
    pub channel: u8,
    pub entries: Vec<SfEntryJson>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Schema version of project.json itself.
    pub version: u16,
    pub name: String,
    pub artist: String,
    pub description: String,
    pub ppq: u32,
    pub compression_level: i32,

    // ── SoundFont state ──
    //
    // `#[serde(default)]` so older `.yin` files still load cleanly. 旧版按
    // port 的字段（soundfont_project_mode / soundfont_overrides）已废弃：
    // 新格式用 `sf_channel_overrides`（每源通道覆盖），旧字段名不同会被
    // serde 忽略（旧工程回退为全局音色库）。
    /// 每源通道（0..256）的音色库覆盖；未列出的通道用全局音色库。
    #[serde(default)]
    pub sf_channel_overrides: Vec<SfChannelOverride>,
}

impl ProjectFile {
    /// Build from `ProjectMeta` only — leaves SF fields empty/default.
    pub fn from_meta(meta: &ProjectMeta) -> Self {
        Self {
            version: 3,
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            description: meta.description.clone(),
            ppq: meta.ppq,
            compression_level: meta.compression_level,
            sf_channel_overrides: Vec::new(),
        }
    }

    /// Build from `ProjectMeta` plus SF state.
    pub fn from_meta_with_sf(
        meta: &ProjectMeta,
        sf_channel_overrides: Vec<SfChannelOverride>,
    ) -> Self {
        Self {
            version: 3,
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            description: meta.description.clone(),
            ppq: meta.ppq,
            compression_level: meta.compression_level,
            sf_channel_overrides,
        }
    }
}
