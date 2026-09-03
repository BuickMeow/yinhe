use std::path::PathBuf;

use super::{AudioSettings, RECENT_FILES_LIMIT};

fn config_path() -> PathBuf {
    crate::paths::app_config_file()
}

impl AudioSettings {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(json) => match serde_json::from_str::<AudioSettings>(&json) {
                    Ok(mut s) => {
                        if !s.default_sf2_path.is_empty() && s.global_sf_config.ports[0].is_empty()
                        {
                            s.global_sf_config = std::mem::take(&mut s.global_sf_config)
                                .with_fallback_path(&s.default_sf2_path);
                        }
                        s.ui_scale = s.ui_scale.clamp(0.75, 2.0);
                        s.font_scale = s.font_scale.clamp(0.75, 2.0);
                        if s.pinned_edit_actions.len() < 12 {
                            s.pinned_edit_actions.resize(12, false);
                        }
                        let before_len = s.recent_files.len();
                        s.recent_files
                            .retain(|p| std::path::Path::new(p).is_absolute());
                        if s.recent_files.len() != before_len {
                            tracing::info!(
                                "清理 recent_files 残留相对路径 {} 条",
                                before_len - s.recent_files.len()
                            );
                        }
                        s.theme_base.danger = yinhe_theme::base::FIXED_DANGER;
                        s.theme_base.warning = yinhe_theme::base::FIXED_WARNING;
                        return s;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse settings: {}", e);
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read settings file: {}", e);
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = config_path();
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::error!("Failed to save settings: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize settings: {}", e);
            }
        }
    }

    pub fn push_recent_file(&mut self, path: &str) -> bool {
        let before = self.recent_files.clone();
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(RECENT_FILES_LIMIT);
        self.recent_files != before
    }

    pub fn remove_recent_file(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
    }

    pub fn available_devices(&self) -> &[String] {
        &self.available_devices
    }

    pub fn available_sample_rates(&self) -> &[u32] {
        &self.available_sample_rates
    }

    pub fn refresh_devices(&mut self, devices: Vec<String>, rates: Vec<u32>, default_rate: u32) {
        self.available_devices = devices;
        self.available_sample_rates = rates;
        if !self.available_sample_rates.contains(&self.sample_rate) {
            self.sample_rate = default_rate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn recent_files_dedup_cap_and_remove() {
        let mut s = AudioSettings::default();
        for i in 0..12 {
            assert!(s.push_recent_file(&format!("/tmp/{i}.yin")));
        }
        assert_eq!(s.recent_files.len(), RECENT_FILES_LIMIT);
        assert_eq!(s.recent_files[0], "/tmp/11.yin");
        assert!(s.push_recent_file("/tmp/5.yin"));
        assert_eq!(s.recent_files[0], "/tmp/5.yin");
        assert_eq!(
            s.recent_files.iter().filter(|p| *p == "/tmp/5.yin").count(),
            1
        );
        assert!(!s.push_recent_file("/tmp/5.yin"));
        s.remove_recent_file("/tmp/5.yin");
        assert!(!s.recent_files.iter().any(|p| p == "/tmp/5.yin"));
    }
}
