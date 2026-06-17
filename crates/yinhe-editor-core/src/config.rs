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

/// Name of the built-in GeneralUser GS SoundFont file.
pub const BUILTIN_SF_NAME: &str = "GeneralUser GS v1.472.sf2";

/// Try to locate the built-in SoundFont.
///
/// In release builds it should live next to the executable under `assets/`.
/// In development we fall back to `crates/yinhe-egui/../assets/` via
/// `CARGO_MANIFEST_DIR`.
pub fn builtin_soundfont_path() -> Option<std::path::PathBuf> {
    let candidates = [
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("assets").join(BUILTIN_SF_NAME))),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("../assets").join(BUILTIN_SF_NAME))),
        Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../assets")
                .join(BUILTIN_SF_NAME),
        ),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|path| path.exists())
}

/// Global soundfont config — persisted to `yinhe_settings.json`.
///
/// Always has 16 ports (A–P). Ports with no entries are simply empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalSfConfig {
    /// Global SF list.  In global mode all ports share `ports[0]`.
    pub ports: [Vec<SfEntry>; 16],
    /// `true` = global mode (one SF set for all ports).
    /// `false` = project mode (per-port SF from `ProjectSfConfig`).
    pub global_enabled: bool,
}

impl GlobalSfConfig {
    /// Build the default config: Port A gets the built-in GeneralUser GS.
    pub fn builtin_default() -> Self {
        let mut ports = std::array::from_fn(|_| Vec::new());
        if let Some(builtin) = builtin_soundfont_path() {
            ports[0] = vec![SfEntry {
                path: builtin.to_string_lossy().to_string(),
                name: "GeneralUser GS".into(),
                enabled: true,
            }];
        }
        Self {
            ports,
            global_enabled: true,
        }
    }

    /// Migrate the old single-path default into Port A's entry list.
    pub fn with_fallback_path(mut self, old_path: &str) -> Self {
        if !old_path.is_empty() && self.ports[0].is_empty() {
            let name = std::path::Path::new(old_path)
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("SoundFont")
                .to_string();
            self.ports[0] = vec![SfEntry {
                path: old_path.to_string(),
                name,
                enabled: true,
            }];
        }
        self
    }
}

impl Default for GlobalSfConfig {
    fn default() -> Self {
        Self::builtin_default()
    }
}
