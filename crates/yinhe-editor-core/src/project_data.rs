use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use yinhe_core::YinModel;
use yinhe_midi::{MidiFile, TrackInfo};

use crate::midi_compat::core_to_midi_file;

/// Persistent project data. This is the source of truth for saving,
/// and the content of undo/redo snapshots.
///
/// `Arc<YinModel>` ensures snapshot clone is O(1) — actual data copy
/// only happens on `Arc::make_mut` (copy-on-write).
///
/// `midi_compat` is a lazy compatibility bridge for legacy consumers
/// still calling `doc.midi()`. It is rebuilt on demand and invalidated
/// on every model rebuild. Will be removed once all consumers read
/// the YinModel directly.
#[derive(Clone)]
pub struct ProjectData {
    pub model: Arc<YinModel>,
    /// Lazy compatibility cache; cleared on every `rebuild_model`.
    /// Cloning a `ProjectData` produces a fresh empty cache.
    midi_compat: ArcOnceLock<MidiFile>,
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

/// Wrapper that gives `OnceLock<Arc<T>>` a `Clone` impl producing a
/// fresh empty cell (since OnceLock<T: Clone> is not itself Clone).
#[derive(Default)]
pub(crate) struct ArcOnceLock<T>(OnceLock<Arc<T>>);

impl<T> Clone for ArcOnceLock<T> {
    fn clone(&self) -> Self {
        // Snapshot clones produce an empty cache: cheap, and the clone
        // will recompute on first access. This matches undo semantics.
        Self(OnceLock::new())
    }
}

impl<T> ArcOnceLock<T> {
    fn clear(&mut self) {
        self.0 = OnceLock::new();
    }
    fn get_or_build(&self, build: impl FnOnce() -> T) -> Arc<T> {
        self.0.get_or_init(|| Arc::new(build())).clone()
    }
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
        // Invalidate the legacy MidiFile cache; consumers will rebuild on demand.
        self.midi_compat.clear();
    }

    /// Get a reference to the YinModel.
    pub fn model(&self) -> &YinModel {
        &self.model
    }

    /// Lazy MidiFile view of the model. First call after `rebuild_model`
    /// pays the conversion cost; subsequent calls return the cached Arc.
    pub fn midi(&self) -> Arc<MidiFile> {
        let model = self.model.clone();
        self.midi_compat.get_or_build(move || core_to_midi_file(&model))
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

    /// Build a fresh MidiFile by value (for export). Does not touch the cache.
    pub fn to_midi(&self) -> MidiFile {
        core_to_midi_file(&self.model)
    }

    /// Construct a `ProjectData` with a given model. The MidiFile cache
    /// starts empty and is filled lazily.
    pub fn new(
        model: Arc<YinModel>,
        track_names: Vec<String>,
        project_name: String,
        project_artist: String,
        project_description: String,
        project_ppq: u32,
        compression_level: i32,
    ) -> Self {
        Self {
            model,
            midi_compat: ArcOnceLock(OnceLock::new()),
            track_names,
            project_name,
            project_artist,
            project_description,
            project_ppq,
            compression_level,
            midi_version: 0,
        }
    }
}
