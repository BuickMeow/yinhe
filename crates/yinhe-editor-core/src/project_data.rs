use std::collections::HashMap;
use std::sync::Arc;

use yinhe_midi::{MidiFile, TrackInfo};
use yinhe_model::YinModel;

/// Persistent project data. This is the source of truth for saving,
/// and the content of undo/redo snapshots.
///
/// `Arc<YinModel>` ensures snapshot clone is O(1) — actual data copy
/// only happens on `Arc::make_mut` (copy-on-write).
#[derive(Clone)]
pub struct ProjectData {
    pub model: Arc<YinModel>,
    /// Cached MidiFile for legacy consumers (audio engine, piano roll).
    /// Rebuilt lazily when model changes.
    pub midi: Arc<MidiFile>,
    /// Authoritative, editable track names. Mirrored into `track_info_cache`.
    pub track_names: Vec<String>,
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
        // Rebuild cached MidiFile for legacy consumers
        self.midi = Arc::new(yinhe_model::convert::to_midi::yinmodel_to_midi(model));
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

    /// Convert to MidiFile for legacy consumers (audio engine, export).
    pub fn to_midi(&self) -> MidiFile {
        yinhe_model::convert::to_midi::yinmodel_to_midi(&self.model)
    }
}
