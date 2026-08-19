//! 插件描述信息（扫描产出，UI 展示与持久化引用用）。

use std::path::PathBuf;

/// 一个 CLAP 插件的元数据。一个 .clap 包可含多个插件（不同 id）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInfo {
    /// .clap 包路径。
    pub path: PathBuf,
    /// CLAP 插件 id（如 "com.u-he.diva"），持久化时以此为准、路径为辅。
    pub id: String,
    pub name: String,
    pub vendor: Option<String>,
    pub version: Option<String>,
    /// CLAP features 列表（如 "instrument"、"audio-effect"、"stereo"）。
    pub features: Vec<String>,
}

impl PluginInfo {
    /// 是否乐器（features 含 "instrument"）。
    pub fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }

    /// 是否效果器（features 含 "audio-effect"）。
    pub fn is_audio_effect(&self) -> bool {
        self.features.iter().any(|f| f == "audio-effect")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_features() {
        let info = PluginInfo {
            path: PathBuf::from("/x.clap"),
            id: "a.b".into(),
            name: "x".into(),
            vendor: None,
            version: None,
            features: vec!["instrument".into(), "stereo".into()],
        };
        assert!(info.is_instrument());
        assert!(!info.is_audio_effect());
    }
}
