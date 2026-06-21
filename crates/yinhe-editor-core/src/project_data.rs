use std::collections::HashMap;
use std::sync::Arc;

use yinhe_core::{YinModel, TrackInfo};
use yinhe_yin::{MappingFile, ProjectFile};

/// Persistent project data. This is the source of truth for saving,
/// and the content of undo/redo snapshots.
///
/// `Arc<YinModel>` ensures snapshot clone is O(1) — actual data copy
/// only happens on `Arc::make_mut` (copy-on-write).
#[derive(Clone)]
pub struct ProjectData {
    pub model: Arc<YinModel>,
    /// Authoritative, editable track names. Mirrored into `track_info_cache`.
    pub track_names: Vec<String>,

    /// The original `project.json` structure, preserved for faithful round-tripping.
    pub project_file: ProjectFile,
    /// The original `mapping.json` structure, preserved for faithful round-tripping.
    pub mapping_file: MappingFile,

    // ── Convenience mirrors of project_file fields ──
    // These are the editable copies used by the UI. On save, they are
    // synced back into `project_file` before serialization.
    pub project_name: String,
    pub project_artist: String,
    pub project_description: String,
    pub project_ppq: u32,
    pub compression_level: i32,

    /// Monotonic counter bumped on every YinModel mutation or snapshot restore.
    /// Used as pianoroll layer-cache key so GPU re-renders when data changes.
    pub midi_version: u64,
}

impl ProjectData {
    /// Snapshot this data for undo. Cheap: Arc::clone + small field clones.
    pub fn snapshot(&self, label: &'static str) -> crate::history::UndoSnapshot {
        crate::history::UndoSnapshot {
            data: self.clone(),
        label,
        }
    }

    /// Bump the version counter to invalidate GPU layer caches.
    pub fn bump_version(&mut self) {
        self.midi_version = self.midi_version.wrapping_add(1);
    }

    /// Rebuild derived indices on the YinModel after mutations.
    ///
    /// O(N) where N = total notes. Call after `Arc::make_mut`.
    pub fn rebuild_model(&mut self) {
        let model = Arc::make_mut(&mut self.model);
        model.rebuild();
    }

    /// Get a reference to the YinModel.
    pub fn model(&self) -> &YinModel {
        &self.model
    }

    /// Rebuild `track_info_cache` from current model.
    pub fn track_info(&self) -> Vec<TrackInfo> {
        self.model
            .tracks
            .iter()
            .enumerate()
            .map(|(i, track)| TrackInfo {
                index: i as u16,
                name: track.name.clone(),
                note_count: track.notes.len() as u64,
                port: track.port,
                channel: track.channel,
            })
            .collect()
    }

    /// Rebuild `pc_map_cache` from current control events.
    pub fn pc_map_cache(&self) -> HashMap<u8, u8> {
        let mut pc_map = HashMap::new();
        for track in &self.model.tracks {
            for pc in &track.program_change {
                pc_map.entry(track.channel).or_insert(pc.program);
            }
        }
        pc_map
    }

    /// Sync convenience fields back into `project_file` before saving.
    pub fn sync_project_file(&mut self) {
        self.project_file.name = self.project_name.clone();
        self.project_file.artist = self.project_artist.clone();
        self.project_file.description = self.project_description.clone();
        self.project_file.ppq = self.project_ppq;
        self.project_file.compression_level = self.compression_level;
    }

    /// Rebuild `mapping_file` from current model tracks.
    pub fn sync_mapping_file(&mut self) {
        self.mapping_file = MappingFile::from_tracks(&self.model.tracks);
    }

    /// Construct a `ProjectData` with a given model and file structures.
    pub fn new(
        model: Arc<YinModel>,
        track_names: Vec<String>,
        project_file: ProjectFile,
        mapping_file: MappingFile,
    ) -> Self {
        let project_name = project_file.name.clone();
        let project_artist = project_file.artist.clone();
        let project_description = project_file.description.clone();
        let project_ppq = project_file.ppq;
        let compression_level = project_file.compression_level;

        Self {
            model,
            track_names,
            project_file,
            mapping_file,
            project_name,
            project_artist,
            project_description,
            project_ppq,
            compression_level,
            midi_version: 0,
        }
    }
}
