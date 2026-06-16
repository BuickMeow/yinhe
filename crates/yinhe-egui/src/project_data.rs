use std::collections::HashMap;
use std::sync::Arc;

use yinhe_midi::{MidiFile, TrackInfo};
use yinhe_midi::build_automation_lanes;
use yinhe_types::NoteScanIndex;

/// Persistent project data. This is the source of truth for saving,
/// and the content of undo/redo snapshots.
///
/// `Arc<MidiFile>` ensures snapshot clone is O(1) — actual data copy
/// only happens on `Arc::make_mut` (copy-on-write).
#[derive(Clone)]
pub(crate) struct ProjectData {
    pub midi: Arc<MidiFile>,
    /// Authoritative, editable track names. Mirrored into `track_info_cache`.
    pub track_names: Vec<String>,
    pub project_name: String,
    pub project_artist: String,
    pub project_description: String,
    pub project_ppq: u32,
    pub compression_level: i32,
    /// Monotonic counter bumped on every MidiFile mutation or snapshot restore.
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

    /// Rebuild `note_count`, `tick_length`, `scan_index`, `automation_lanes`
    /// on the MidiFile after note mutations.
    ///
    /// O(N) where N = total notes. Call after `Arc::make_mut`.
    pub fn rebuild_midi_metadata(&mut self) {
        let midi = Arc::make_mut(&mut self.midi);
        midi.note_count = 0;
        let mut max_tick = 0u64;
        for notes in &midi.key_notes {
            midi.note_count += notes.len() as u64;
            for note in notes {
                max_tick = max_tick.max(note.end_tick as u64);
            }
        }
        midi.tick_length = max_tick;
        midi.scan_index = Some(NoteScanIndex::build(&midi.key_notes, max_tick));
        midi.automation_lanes = build_automation_lanes(
            &midi.control_events,
            &midi.key_notes,
            &midi.track_channels,
        );
    }

    /// Rebuild `track_info_cache` from current midi + track_names.
    pub fn track_info(&self) -> Vec<TrackInfo> {
        self.midi.track_info()
    }

    /// Rebuild `pc_map_cache` from current control events.
    pub fn pc_map_cache(&self) -> HashMap<u8, u8> {
        let mut pc_map = HashMap::new();
        for ev in &self.midi.control_events {
            if let yinhe_midi::MidiControlEvent::ProgramChange {
                program, track, ..
            } = ev
            {
                let ch = self.midi.track_channels.get(*track as usize).copied().unwrap_or(0);
                pc_map.entry(ch).or_insert(*program);
            }
        }
        pc_map
    }
}
