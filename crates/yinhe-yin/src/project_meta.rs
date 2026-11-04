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

    // ── 歌曲信息（旧存档缺失时为空/0）──
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub track_no: u16,
    #[serde(default)]
    pub disc_no: u16,
    #[serde(default)]
    pub copyright: String,
    #[serde(default)]
    pub artists: Vec<yinhe_core::ArtistRef>,
    /// 累计前台工作时间（秒）。
    #[serde(default)]
    pub work_time_secs: u64,

    /// 最终封面。不写进 project.json（避免 base64 +33%），由 `.yin` 的可选
    /// cover 段承载；加载时由 container 解出后回填到这里。
    #[serde(skip)]
    pub cover: Option<yinhe_core::CoverArt>,

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
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year: String::new(),
            track_no: 0,
            disc_no: 0,
            copyright: String::new(),
            artists: Vec::new(),
            work_time_secs: 0,
            cover: None,
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
            album: meta.album.clone(),
            album_artist: meta.album_artist.clone(),
            genre: meta.genre.clone(),
            year: meta.year.clone(),
            track_no: meta.track_no,
            disc_no: meta.disc_no,
            copyright: meta.copyright.clone(),
            artists: meta.artists.clone(),
            work_time_secs: meta.work_time_secs,
            cover: None,
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
            album: meta.album.clone(),
            album_artist: meta.album_artist.clone(),
            genre: meta.genre.clone(),
            year: meta.year.clone(),
            track_no: meta.track_no,
            disc_no: meta.disc_no,
            copyright: meta.copyright.clone(),
            artists: meta.artists.clone(),
            work_time_secs: meta.work_time_secs,
            cover: None,
            sf_channel_overrides,
        }
    }
}
