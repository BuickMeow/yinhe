use serde::{Deserialize, Serialize};

// ── JSON structures ──

/// A single SoundFont entry stored in the project JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfEntryJson {
    pub path: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Project-level soundfont override for one port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfPortOverride {
    pub port: u8,
    pub entries: Vec<SfEntryJson>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectJson {
    pub version: u8,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    /// Ticks per beat (quarter note).
    #[serde(default = "default_ppq")]
    pub ppq: u32,
    /// zstd compression level (0 = default / 3).
    #[serde(default = "default_zstd_level")]
    pub zstd_level: i32,
    /// Song description / notes.
    #[serde(default)]
    pub description: String,
    /// `true` = project mode (per-port SF).  `false` = global mode.
    #[serde(default)]
    pub soundfont_project_mode: bool,
    /// Per-port soundfont entries (only used in project mode).
    #[serde(default)]
    pub soundfont_overrides: Vec<SfPortOverride>,
}

fn default_ppq() -> u32 {
    480
}

fn default_zstd_level() -> i32 {
    0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MappingJson {
    pub ports: Vec<PortMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortMapping {
    pub port: u8,
    pub channels: Vec<ChannelMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelMapping {
    pub channel: u8,
    pub tracks: Vec<TrackMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackMapping {
    pub uuid: String,
    pub name: String,
    pub color: [f32; 3],
    /// Original MIDI track index (preserved across save/load for correct name mapping).
    #[serde(default)]
    pub track_index: u16,
    /// MIDI Channel Prefix (meta event 0x20) for this track, if present.
    /// Used as fallback for channel info on tracks with no note/CC events.
    #[serde(default)]
    pub channel_prefix: Option<u8>,
}
