//! `project.json` — top-level human-readable project metadata.

use serde::{Deserialize, Serialize};

use yinhe_core::ProjectMeta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Schema version of project.json itself.
    pub version: u16,
    pub name: String,
    pub artist: String,
    pub description: String,
    pub ppq: u32,
    pub compression_level: i32,
}

impl ProjectFile {
    pub fn from_meta(meta: &ProjectMeta) -> Self {
        Self {
            version: 2,
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            description: meta.description.clone(),
            ppq: meta.ppq,
            compression_level: meta.compression_level,
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
