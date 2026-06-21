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

/// Project-level soundfont override for one MIDI port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfPortOverride {
    pub port: u8,
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
    // These fields are `#[serde(default)]` so older `.yin` files (written
    // before SF persistence existed) still load cleanly with empty SF state.

    /// `true` = project was saved while in per-port (project) SF mode.
    /// `false` = global mode (or unknown / pre-SF-persistence file).
    #[serde(default)]
    pub soundfont_project_mode: bool,

    /// Per-port SoundFont entries. Only meaningful in project mode, but is
    /// always serialized (so the user's project-mode list survives a global-
    /// mode save→load cycle if they switch back).
    #[serde(default)]
    pub soundfont_overrides: Vec<SfPortOverride>,
}

impl ProjectFile {
    /// Build from `ProjectMeta` only — leaves SF fields empty/default.
    pub fn from_meta(meta: &ProjectMeta) -> Self {
        Self {
            version: 2,
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            description: meta.description.clone(),
            ppq: meta.ppq,
            compression_level: meta.compression_level,
            soundfont_project_mode: false,
            soundfont_overrides: Vec::new(),
        }
    }

    /// Build from `ProjectMeta` plus SF state.
    pub fn from_meta_with_sf(
        meta: &ProjectMeta,
        soundfont_project_mode: bool,
        soundfont_overrides: Vec<SfPortOverride>,
    ) -> Self {
        Self {
            version: 2,
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            description: meta.description.clone(),
            ppq: meta.ppq,
            compression_level: meta.compression_level,
            soundfont_project_mode,
            soundfont_overrides,
        }
    }

    pub fn into_meta(self) -> ProjectMeta {
        ProjectMeta {
            name: self.name,
            artist: self.artist,
            description: self.description,
            ppq: self.ppq,
            compression_level: self.compression_level,
        }
    }
}
