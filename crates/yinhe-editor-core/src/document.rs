use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_model::{NoteEvent, YinModel};
use yinhe_types::TRACK_PALETTE;

use crate::config::{ProjectSfConfig, SfEntry};
use crate::edit_state::EditState;
use crate::history::UndoStack;
use crate::project_data::ProjectData;
use crate::quantize::QuantizePreset;

/// Per-track mutable overrides (mute, solo).
#[derive(Clone)]
pub struct TrackOverride {
    pub muted: bool,
    pub soloed: bool,
}

impl Default for TrackOverride {
    fn default() -> Self {
        Self {
            muted: false,
            soloed: false,
        }
    }
}

/// Per-document state: persistent data + editing state + undo history.
///
/// Layout/zoom state (PianoRollView, ArrangementView) lives in `App`,
/// not here, so loading a new MIDI file preserves the user's zoom/scroll.
pub struct Document {
    /// Persistent project data (source of truth for save, undo snapshots).
    pub data: ProjectData,
    /// Transient editing state (not persisted, not in undo snapshots).
    pub edit: EditState,
    /// Per-document undo/redo stack.
    pub history: UndoStack,
    /// File identity.
    pub file_name: String,
    pub file_path: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

impl Document {
    // ── Convenience accessors ──

    pub fn model(&self) -> &YinModel {
        &self.data.model
    }

    /// Compatibility: convert YinModel to MidiFile for legacy consumers.
    /// This is O(N) — prefer using model() directly when possible.
    pub fn midi(&self) -> yinhe_midi::MidiFile {
        self.data.to_midi()
    }

    pub fn track_names(&self) -> &[String] {
        &self.data.track_names
    }

    pub fn selected(&self) -> &HashSet<(u16, u32, u8)> {
        &self.edit.selected
    }

    pub fn track_info_cache(&self) -> &[yinhe_midi::TrackInfo] {
        &self.edit.track_info_cache
    }

    // ── Constructors ──

    pub fn empty() -> Self {
        let mut model = YinModel {
            conductor: yinhe_model::ConductorData {
                tempo: vec![yinhe_model::TempoEvent {
                    tick: 0,
                    bpm: 120.0,
                }],
                time_sig: vec![yinhe_model::TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                }],
            },
            tracks: vec![yinhe_model::TrackData {
                uuid: uuid::Uuid::new_v4().to_string(),
                name: "Track 1".to_string(),
                port: 0,
                channel: 0,
                notes: Vec::new(),
                cc: std::collections::BTreeMap::new(),
                pitch_bend: Vec::new(),
                program_change: Vec::new(),
                rpn: std::collections::BTreeMap::new(),
            }],
            meta: yinhe_model::ProjectMeta::default(),
            key_index: yinhe_model::KeyIndex::default(),
            key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
            note_count: 0,
            tick_length: 0,
        };
        model.rebuild();

        let track_names = model.tracks.iter().map(|t| t.name.clone()).collect();
        let track_info_cache = build_track_info(&model);
        let num_tracks = model.tracks.len();
        let conductor_track_idx = None;
        let midi = Arc::new(yinhe_model::convert::to_midi::yinmodel_to_midi(&model));

        Document {
            data: ProjectData {
                model: Arc::new(model),
                midi,
                track_names,
                project_name: String::new(),
                project_artist: String::new(),
                project_description: String::new(),
                project_ppq: 480,
                compression_level: 0,
                midi_version: 0,
            },
            edit: EditState {
                track_visible: vec![true],
                track_pianoroll_visible: vec![true],
                track_info_cache,
                track_colors_cache: (0..num_tracks)
                    .map(|i| track_color(i, conductor_track_idx))
                    .collect(),
                conductor_track_idx,
                ..Default::default()
            },
            history: UndoStack::new(),
            file_name: "Untitled".into(),
            file_path: None,
        }
    }

    pub fn from_midi(
        path: &str,
        midi: yinhe_midi::MidiFile,
        quantize: QuantizePreset,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            // Convert to YinModel
            let mut model = yinhe_model::convert::from_midi::midi_to_yinmodel(&midi);

            // Detect conductor track
            let conductor_track_idx = detect_conductor_from_model(&model);
            if conductor_track_idx.is_none() {
                // Insert conductor track
                for track in &mut model.tracks {
                    for note in &mut track.notes {
                        // Notes are already per-track, no global track offset needed
                    }
                }
                model.tracks.insert(
                    0,
                    yinhe_model::TrackData {
                        uuid: uuid::Uuid::new_v4().to_string(),
                        name: "Conductor".to_string(),
                        port: 0,
                        channel: 0,
                        notes: Vec::new(),
                        cc: std::collections::BTreeMap::new(),
                        pitch_bend: Vec::new(),
                        program_change: Vec::new(),
                        rpn: std::collections::BTreeMap::new(),
                    },
                );
            }
            let conductor_track_idx = detect_conductor_from_model(&model);

            let num_tracks = model.tracks.len();
            let track_names: Vec<String> = model.tracks.iter().map(|t| t.name.clone()).collect();
            let track_info_cache = build_track_info(&model);
            let pc_map_cache = build_pc_map_cache_from_model(&model);
            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(i, conductor_track_idx))
                .collect();

            let mut data = ProjectData {
                model: Arc::new(model.clone()),
                midi: Arc::new(yinhe_model::convert::to_midi::yinmodel_to_midi(&model)),
                track_names,
                project_name: String::new(),
                project_artist: String::new(),
                project_description: String::new(),
                project_ppq: 480,
                compression_level: 0,
                midi_version: 0,
            };
            data.rebuild_model();

            Ok(Document {
                data,
                edit: EditState {
                    quantize,
                    track_visible: vec![true; num_tracks],
                    track_pianoroll_visible: vec![true; num_tracks],
                    track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
                    track_info_cache,
                    pc_map_cache,
                    track_colors_cache,
                    conductor_track_idx,
                    ..Default::default()
                },
                history: UndoStack::new(),
                file_name,
                file_path: None,
            })
        })
    }

    pub fn from_yin(
        path: &str,
        quantize: QuantizePreset,
    ) -> std::io::Result<(Self, bool)> {
        // Load .yin file
        let model = yinhe_model::io::load::load_yin(path)?;

        let file_name = std::path::Path::new(path)
            .file_stem()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let num_tracks = model.tracks.len();
        let track_names: Vec<String> = model.tracks.iter().map(|t| t.name.clone()).collect();
        let track_info_cache = build_track_info(&model);
        let pc_map_cache = build_pc_map_cache_from_model(&model);
        let conductor_track_idx = detect_conductor_from_model(&model);

        // Extract project metadata
        let project_name = model.meta.name.clone();
        let project_artist = model.meta.artist.clone();
        let project_description = model.meta.description.clone();
        let project_ppq = model.meta.ppq;
        let compression_level = model.meta.compression_level;

        let track_colors_cache = (0..num_tracks)
            .map(|i| track_color(i, conductor_track_idx))
            .collect();

        let mut data = ProjectData {
            model: Arc::new(model.clone()),
            midi: Arc::new(yinhe_model::convert::to_midi::yinmodel_to_midi(&model)),
            track_names,
            project_name,
            project_artist,
            project_description,
            project_ppq,
            compression_level,
            midi_version: 0,
        };
        data.rebuild_model();

        Ok((
            Document {
                data,
                edit: EditState {
                    quantize,
                    track_visible: vec![true; num_tracks],
                    track_pianoroll_visible: vec![true; num_tracks],
                    track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
                    track_info_cache,
                    pc_map_cache,
                    track_colors_cache,
                    conductor_track_idx,
                    ..Default::default()
                },
                history: UndoStack::new(),
                file_name,
                file_path: Some(path.to_string()),
            },
            false, // soundfont_project_mode
        ))
    }

    /// Re-decode all track names using a different encoding.
    pub fn recode_track_names(&mut self, _encoding: yinhe_midi::MidiImportEncoding) {
        // TODO: implement track name re-encoding for YinModel
        self.data.bump_version();
    }

    // ── Note editing operations ──

    /// Delete all selected notes. Returns `true` if any notes were removed.
    pub fn delete_selected(&mut self) -> bool {
        if self.edit.selected.is_empty() {
            return false;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);
            for &(track, start_tick, key) in &self.edit.selected {
                let track = track as usize;
                if track < model.tracks.len() {
                    model.tracks[track]
                        .notes
                        .retain(|n| !(n.key == key as u8 && n.tick == start_tick));
                }
            }
            self.edit.selected.clear();
        }
        self.data.rebuild_model();
        true
    }

    /// Duplicate all selected notes. New notes are placed after the original
    /// selection, offset by the selection duration. Returns `true` if any
    /// notes were duplicated.
    pub fn duplicate_selected(&mut self) -> bool {
        if self.edit.selected.is_empty() {
            return false;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);

            let mut selected_data: Vec<(NoteEvent, u16)> = Vec::new();
            for &(track, start_tick, key) in &self.edit.selected {
                let t = track as usize;
                if t < model.tracks.len() {
                    if let Some(note) = model.tracks[t]
                        .notes
                        .iter()
                        .find(|n| n.key == key as u8 && n.tick == start_tick)
                    {
                        selected_data.push((*note, track));
                    }
                }
            }

            if selected_data.is_empty() {
                return false;
            }

            let min_start = selected_data.iter().map(|(n, _)| n.tick).min().unwrap();
            let max_end = selected_data
                .iter()
                .map(|(n, _)| n.tick + n.duration)
                .max()
                .unwrap();
            let offset = (max_end - min_start).max(1);

            let mut new_selected = HashSet::new();
            for (note, track) in &selected_data {
                let t = *track as usize;
                if t < model.tracks.len() {
                    let new_note = NoteEvent {
                        tick: note.tick + offset,
                        duration: note.duration,
                        key: note.key,
                        velocity: note.velocity,
                    };
                    let insert_pos = model.tracks[t]
                        .notes
                        .partition_point(|n| n.tick < new_note.tick);
                    model.tracks[t].notes.insert(insert_pos, new_note);
                    new_selected.insert((*track, note.tick + offset, note.key));
                }
            }

            self.edit.selected = new_selected;
        }
        self.data.rebuild_model();
        true
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave,
    /// -12 for down). Returns `true` if any notes were moved.
    pub fn transpose_selected(&mut self, semitones: i8) -> bool {
        if self.edit.selected.is_empty() {
            return false;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);

            let mut moved_data: Vec<(NoteEvent, u16)> = Vec::new();
            for &(track, start_tick, key) in &self.edit.selected {
                let t = track as usize;
                if t < model.tracks.len() {
                    if let Some(pos) = model.tracks[t]
                        .notes
                        .iter()
                        .position(|n| n.key == key as u8 && n.tick == start_tick)
                    {
                        let note = model.tracks[t].notes.remove(pos);
                        moved_data.push((note, track));
                    }
                }
            }

            if moved_data.is_empty() {
                return false;
            }

            let mut new_selected = HashSet::new();
            for (note, track) in &moved_data {
                let t = *track as usize;
                if t < model.tracks.len() {
                    let new_key = ((note.key as i16) + (semitones as i16)).clamp(0, 127) as u8;
                    let new_note = NoteEvent {
                        tick: note.tick,
                        duration: note.duration,
                        key: new_key,
                        velocity: note.velocity,
                    };
                    let insert_pos = model.tracks[t]
                        .notes
                        .partition_point(|n| n.tick < new_note.tick);
                    model.tracks[t].notes.insert(insert_pos, new_note);
                    new_selected.insert((*track, note.tick, new_key));
                }
            }

            self.edit.selected = new_selected;
        }
        self.data.rebuild_model();
        true
    }

    /// Restore document state from an undo snapshot, rebuild caches,
    /// and clear selection.
    pub fn apply_undo_snapshot(&mut self, snap: crate::history::UndoSnapshot) {
        self.data = snap.data;
        self.data.bump_version();
        self.edit.track_info_cache = self.data.track_info();
        for (i, ti) in self.edit.track_info_cache.iter().enumerate() {
            if i < self.data.track_names.len() {
                self.data.track_names[i] = ti.name.clone();
            }
        }
        self.edit.pc_map_cache = self.data.pc_map_cache();
        self.edit.selected.clear();
    }
}

// ── Free functions ──

/// Detect the conductor track from a YinModel.
pub fn detect_conductor_from_model(model: &YinModel) -> Option<u16> {
    if model.tracks.is_empty() {
        return None;
    }
    // First track is conductor if it has no notes and no CC events
    let first = &model.tracks[0];
    if !first.notes.is_empty() {
        return None;
    }
    if !first.cc.is_empty() || !first.pitch_bend.is_empty() || !first.program_change.is_empty() {
        return None;
    }
    Some(0)
}

/// Build track info from YinModel.
fn build_track_info(model: &YinModel) -> Vec<yinhe_midi::TrackInfo> {
    model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| yinhe_midi::TrackInfo {
            index: i as u16,
            name: track.name.clone(),
            note_count: track.notes.len() as u64,
            port: track.port,
            channel: track.channel,
        })
        .collect()
}

/// Build pc_map_cache from YinModel.
fn build_pc_map_cache_from_model(model: &YinModel) -> HashMap<u8, u8> {
    let mut pc_map = HashMap::new();
    for track in &model.tracks {
        for pc in &track.program_change {
            pc_map.entry(track.channel).or_insert(pc.program);
        }
    }
    pc_map
}

/// Compute the display color for a track, respecting an optional conductor offset.
pub fn track_color(idx: usize, conductor_idx: Option<u16>) -> [f32; 3] {
    if Some(idx as u16) == conductor_idx {
        return [0.94, 0.94, 0.94];
    }
    let palette_idx = match conductor_idx {
        Some(c) if (idx as u16) > c => idx - 1,
        _ => idx,
    };
    TRACK_PALETTE[palette_idx % TRACK_PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_creates_valid_document_with_one_track() {
        let doc = Document::empty();
        assert_eq!(doc.model().tracks.len(), 1);
        assert_eq!(doc.model().tracks[0].name, "Track 1");
        assert_eq!(doc.track_names().len(), 1);
        assert_eq!(doc.track_names()[0], "Track 1");
        assert!(doc.edit.conductor_track_idx.is_none());
        assert_eq!(doc.edit.track_visible.len(), 1);
        assert_eq!(doc.edit.track_pianoroll_visible.len(), 1);
        assert_eq!(doc.file_name, "Untitled");
    }

    #[test]
    fn detect_conductor_none_when_track0_has_notes() {
        let mut model = YinModel {
            conductor: yinhe_model::ConductorData {
                tempo: Vec::new(),
                time_sig: Vec::new(),
            },
            tracks: vec![yinhe_model::TrackData {
                uuid: "test".into(),
                name: "Piano".into(),
                port: 0,
                channel: 0,
                notes: vec![NoteEvent {
                    tick: 0,
                    duration: 480,
                    key: 60,
                    velocity: 100,
                }],
                cc: std::collections::BTreeMap::new(),
                pitch_bend: Vec::new(),
                program_change: Vec::new(),
                rpn: std::collections::BTreeMap::new(),
            }],
            meta: yinhe_model::ProjectMeta::default(),
            key_index: yinhe_model::KeyIndex::default(),
            key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
            note_count: 0,
            tick_length: 0,
        };
        model.rebuild();
        assert_eq!(detect_conductor_from_model(&model), None);
    }

    #[test]
    fn detect_conductor_some_when_track0_has_no_notes_and_no_ctrl() {
        let mut model = YinModel {
            conductor: yinhe_model::ConductorData {
                tempo: Vec::new(),
                time_sig: Vec::new(),
            },
            tracks: vec![
                yinhe_model::TrackData {
                    uuid: "test1".into(),
                    name: "Conductor".into(),
                    port: 0,
                    channel: 0,
                    notes: Vec::new(),
                    cc: std::collections::BTreeMap::new(),
                    pitch_bend: Vec::new(),
                    program_change: Vec::new(),
                    rpn: std::collections::BTreeMap::new(),
                },
                yinhe_model::TrackData {
                    uuid: "test2".into(),
                    name: "Piano".into(),
                    port: 0,
                    channel: 0,
                    notes: vec![NoteEvent {
                        tick: 0,
                        duration: 480,
                        key: 60,
                        velocity: 100,
                    }],
                    cc: std::collections::BTreeMap::new(),
                    pitch_bend: Vec::new(),
                    program_change: Vec::new(),
                    rpn: std::collections::BTreeMap::new(),
                },
            ],
            meta: yinhe_model::ProjectMeta::default(),
            key_index: yinhe_model::KeyIndex::default(),
            key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
            note_count: 0,
            tick_length: 0,
        };
        model.rebuild();
        assert_eq!(detect_conductor_from_model(&model), Some(0));
    }

    #[test]
    fn track_color_conductor_is_whiteish() {
        let color = track_color(0, Some(0));
        assert_eq!(color, [0.94, 0.94, 0.94]);
    }

    #[test]
    fn track_color_cycles_through_palette() {
        let first = track_color(0, None);
        assert_eq!(first, TRACK_PALETTE[0]);
        let second = track_color(1, None);
        assert_eq!(second, TRACK_PALETTE[1]);
        let wrap = track_color(16, None);
        assert_eq!(wrap, TRACK_PALETTE[0]);
    }

    #[test]
    fn track_color_offsets_after_conductor() {
        let color = track_color(1, Some(0));
        assert_eq!(color, TRACK_PALETTE[0]);
    }
}
