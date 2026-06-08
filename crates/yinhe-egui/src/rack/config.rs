use serde::{Deserialize, Serialize};

/// A single SoundFont entry — one .sf2/.sf3/.sfz file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SfEntry {
    pub path: String,
    pub name: String,
    pub enabled: bool,
}

/// Global soundfont config — persisted to `settings.json`.
///
/// Always has 16 ports (A–P). Ports with no entries are simply empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalSfConfig {
    pub ports: [Vec<SfEntry>; 16],
    pub global_enabled: bool,
}

impl GlobalSfConfig {
    /// Build the default config: Port A gets the built-in GeneralUser GS.
    pub fn builtin_default() -> Self {
        let builtin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/GeneralUser GS v1.472.sf2");
        let mut ports = std::array::from_fn(|_| Vec::new());
        if builtin.exists() {
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

/// Song-specific soundfont overrides — lives in `Document`, not persisted yet.
///
/// Only ports that appear in `overrides` differ from the global config.
/// When `project_enabled` is false, only the global config is used.
#[derive(Clone, Debug)]
pub struct ProjectSfConfig {
    /// Port → entries that *replace* the global config for that port.
    pub overrides: Vec<(u8, Vec<SfEntry>)>,
    pub project_enabled: bool,
}

impl Default for ProjectSfConfig {
    fn default() -> Self {
        Self {
            overrides: Vec::new(),
            project_enabled: false,
        }
    }
}
