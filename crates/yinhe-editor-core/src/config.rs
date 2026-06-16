use serde::{Deserialize, Serialize};

/// A single SoundFont entry — one .sf2/.sf3/.sfz file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SfEntry {
    pub path: String,
    pub name: String,
    pub enabled: bool,
}

/// Song-specific soundfont config — lives in `Document`.
///
/// When `GlobalSfConfig.global_enabled` is `false`, each port uses its
/// entry here (if present), otherwise falls back to the built-in SF.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectSfConfig {
    /// Port → SF entries for that port.
    pub overrides: Vec<(u8, Vec<SfEntry>)>,
}
