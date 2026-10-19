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

/// Project-level soundfont override for one source channel (0..=255).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfChannelOverride {
    pub channel: u8,
    pub entries: Vec<SfEntryJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Schema version of project.json itself.
    pub version: u16,
    pub name: String,
    pub artist: String,
    pub description: String,
    pub ppq: u32,
    pub compression_level: i32,

    // ── 工程默认（无原生事件时的回退） ──
    /// 本工程默认 BPM（conductor 无 Tempo 事件时生效）。
    #[serde(default = "default_bpm")]
    pub default_bpm: f32,
    /// 本工程默认拍号 `(分子, 分母 2 次幂)`（conductor 无拍号事件时生效）。
    #[serde(default = "default_time_sig")]
    pub default_time_sig: (u8, u8),

    // ── SoundFont state ──
    /// 每源通道（0..=255，共 256 条）的音色库覆盖；未列出的通道用全局音色库。
    #[serde(default)]
    pub sf_channel_overrides: Vec<SfChannelOverride>,
}

fn default_bpm() -> f32 {
    120.0
}

fn default_time_sig() -> (u8, u8) {
    (4, 2)
}

impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: 0,
            name: String::new(),
            artist: String::new(),
            description: String::new(),
            ppq: 480,
            compression_level: 3,
            default_bpm: default_bpm(),
            default_time_sig: default_time_sig(),
            sf_channel_overrides: Vec::new(),
        }
    }
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
            default_bpm: meta.default_bpm,
            default_time_sig: meta.default_time_sig,
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
            default_bpm: meta.default_bpm,
            default_time_sig: meta.default_time_sig,
            sf_channel_overrides,
        }
    }
}
