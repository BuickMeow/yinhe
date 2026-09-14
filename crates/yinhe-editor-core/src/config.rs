use serde::{Deserialize, Serialize};

/// A single SoundFont entry — one .sf2/.sf3/.sfz file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SfEntry {
    pub path: String,
    pub name: String,
    pub enabled: bool,
}

/// 工程内音色库覆盖：某**源通道**（`TrackData::global_channel`，0..256）
/// 单独配置的音色库列表。未在此列出的通道使用全局音色库（设置页配置）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectSfConfig {
    /// 源通道 → 该通道的 SF 条目。
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
        std::env::current_exe().ok().and_then(|exe| {
            exe.parent()
                .map(|p| p.join("../assets").join(BUILTIN_SF_NAME))
        }),
        Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../assets")
                .join(BUILTIN_SF_NAME),
        ),
    ];

    candidates.into_iter().flatten().find(|path| path.exists())
}

/// 全局音色库（应用级默认）— 持久化到 `yinhe_settings.json`。
///
/// 语义：所有未在工程内单独配置的源通道都使用此列表。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalSfConfig {
    /// 全局 SF 条目列表。
    #[serde(default)]
    pub entries: Vec<SfEntry>,
}

impl GlobalSfConfig {
    /// 默认配置：内置 GeneralUser GS。
    pub fn builtin_default() -> Self {
        let mut entries = Vec::new();
        if let Some(builtin) = builtin_soundfont_path() {
            entries.push(SfEntry {
                path: builtin.to_string_lossy().to_string(),
                name: "GeneralUser GS".into(),
                enabled: true,
            });
        }
        Self { entries }
    }

    /// 旧格式迁移：把旧的单路径默认值搬进条目列表（列表为空时）。
    pub fn with_fallback_path(mut self, old_path: &str) -> Self {
        if !old_path.is_empty() && self.entries.is_empty() {
            let name = std::path::Path::new(old_path)
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("SoundFont")
                .to_string();
            self.entries = vec![SfEntry {
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
