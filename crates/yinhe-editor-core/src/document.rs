use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_core::{NoteEvent, TrackData, YinModel};
use yinhe_types::TRACK_PALETTE;
use yinhe_yin::{MappingFile, ProjectFile};

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
pub struct Document {
    pub data: ProjectData,
    pub edit: EditState,
    pub history: UndoStack,
    pub file_name: String,
    pub file_path: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

impl Document {
    pub fn model(&self) -> &YinModel {
        &self.data.model
    }

    pub fn track_names(&self) -> &[String] {
        &self.data.track_names
    }

    pub fn selected(&self) -> &HashSet<(u16, u32, u8)> {
        &self.edit.selected
    }

    pub fn track_info_cache(&self) -> &[yinhe_core::TrackInfo] {
        &self.edit.track_info_cache
    }

    pub fn empty() -> Self {
        let mut model = YinModel {
            conductor: Arc::new(yinhe_core::ConductorData {
                tempo: vec![yinhe_core::TempoEvent { tick: 0, bpm: 120.0 }],
                time_sig: vec![yinhe_core::TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                }],
            }),
            tracks: vec![Arc::new({
                let mut t = TrackData::new(0, 0);
                t.name = "Track 1".to_string();
                t
            })],
            ..Default::default()
        };
        model.rebuild();

        let track_names = model.tracks.iter().map(|t| t.name.clone()).collect();
        let track_info_cache = build_track_info(&model);
        let num_tracks = model.tracks.len();
        let conductor_track_idx = None;

        Document {
            data: ProjectData::new(
                Arc::new(model),
                track_names,
                ProjectFile::default(),
                MappingFile::default(),
            ),
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

    /// Create a Document from a freshly parsed YinModel.
    /// Path is used only to derive the file name; data ownership comes from
    /// the model. Inserts a conductor track at index 0 if absent.
    pub fn from_model(
        path: &str,
        model: YinModel,
        quantize: QuantizePreset,
        project_file: ProjectFile,
        mapping_file: MappingFile,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let mut model = model;

            // Detect conductor track; insert one if missing.
            let conductor_track_idx = detect_conductor_from_model(&model);
            if conductor_track_idx.is_none() {
                let mut conductor = TrackData::new(0, 0);
                conductor.name = "Conductor".to_string();
                // Shift all existing note track indices by 1 to make room.
                for bucket in model.notes.iter_mut() {
                    for n in Arc::make_mut(bucket).iter_mut() {
                        n.track += 1;
                    }
                }
                model.tracks.insert(0, Arc::new(conductor));
                model.rebuild();
            }
            let conductor_track_idx = detect_conductor_from_model(&model);

            let num_tracks = model.tracks.len();
            let track_names: Vec<String> = model.tracks.iter().map(|t| t.name.clone()).collect();
            let track_info_cache = build_track_info(&model);
            let pc_map_cache = build_pc_map_cache_from_model(&model);
            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(i, conductor_track_idx))
                .collect();

            let mut data = ProjectData::new(
                Arc::new(model),
                track_names,
                project_file,
                mapping_file,
            );
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

    /// Load a `.yin` file. Returns `(Document, soundfont_project_mode)`.
    pub fn from_yin_path(
        path: &str,
        quantize: QuantizePreset,
    ) -> std::io::Result<(Self, bool)> {
        let (model, sf, mapping) = yinhe_yin::load_yin_with_sf(path).map_err(|e| match e {
            yinhe_yin::YinError::Io(io) => io,
            other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
        })?;
        let project_file = yinhe_yin::ProjectFile::from_meta_with_sf(
            &model.meta,
            sf.mode,
            sf.overrides.clone(),
        );
        let mut doc = Self::from_model(path, model, quantize, project_file, mapping).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        doc.file_path = Some(path.to_string());
        Ok((doc, sf.mode))
    }

    /// Legacy: load a .yin file via the old yinhe-project archive path.
    /// Kept until Phase 4d removes the old path.
    pub fn from_yin(
        path: &str,
        quantize: QuantizePreset,
    ) -> std::io::Result<(Self, bool)> {
        Self::from_yin_path(path, quantize)
    }

    pub fn recode_track_names(&mut self, _encoding: yinhe_mid2::MidiImportEncoding) {
        // TODO: implement track name re-encoding for YinModel
        self.data.bump_version();
    }
}

impl Document {
    /// Create an undo snapshot that includes the current selection.
    pub fn snapshot_with_selection(&self, label: &'static str) -> crate::history::UndoSnapshot {
        crate::history::UndoSnapshot {
            data: self.data.clone(),
            label,
            selected: self.edit.selected.clone(),
            track_selected: self.edit.track_selected.clone(),
            sel_rect: self.edit.sel_rect.clone(),
        }
    }

    pub fn add_note(&mut self, track_idx: u16, note: NoteEvent) -> bool {
        let t = track_idx as usize;
        if t >= self.data.model.tracks.len() {
            return false;
        }
        if Some(track_idx) == self.edit.conductor_track_idx {
            return false;
        }
        if self.data.model.track_note_count.get(t).copied().unwrap_or(0) == 0 {
            return false;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let key = note.key as usize;
        let insert_pos = model.notes[key].partition_point(|n| n.start_tick < note.start_tick);
        Arc::make_mut(&mut model.notes[key]).insert(
            insert_pos,
            yinhe_types::Note {
                start_tick: note.start_tick,
                end_tick: note.end_tick,
                velocity: note.velocity,
                dup_index: note.dup_index,
                track: track_idx,
            },
        );
        self.data.rebuild_model();
        true
    }

    pub fn delete_selected(&mut self) -> bool {
        if self.edit.selected.is_empty() {
            return false;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);
            for &(track, start_tick, key) in &self.edit.selected {
                let key = key as usize;
                Arc::make_mut(&mut model.notes[key]).retain(|n| {
                    !(n.track == track && n.start_tick == start_tick)
                });
            }
            self.edit.selected.clear();
        }
        self.data.rebuild_model();
        true
    }

    pub fn duplicate_selected(&mut self) -> Option<u32> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let offset = {
            let model = Arc::make_mut(&mut self.data.model);

            // Collect selected notes with their key (bucket index).
            let mut selected_data: Vec<(yinhe_types::Note, u16, u8)> = Vec::new();
            for &(track, start_tick, key) in &self.edit.selected {
                let k = key as usize;
                if let Some(note) = model.notes[k]
                    .iter()
                    .find(|n| n.track == track && n.start_tick == start_tick)
                {
                    selected_data.push((*note, track, key));
                }
            }

            if selected_data.is_empty() {
                return None;
            }

            let min_start = selected_data.iter().map(|(n, _, _)| n.start_tick).min().unwrap();
            let max_end = selected_data.iter().map(|(n, _, _)| n.end_tick).max().unwrap();
            let offset = (max_end - min_start).max(1);

            let mut new_selected = HashSet::new();
            for (note, track, key) in &selected_data {
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick + offset,
                    end_tick: note.end_tick + offset,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: *track,
                };
                let k = *key as usize;
                let insert_pos = model.notes[k].partition_point(|n| n.start_tick < new_note.start_tick);
                Arc::make_mut(&mut model.notes[k]).insert(insert_pos, new_note);
                new_selected.insert((*track, note.start_tick + offset, *key));
            }

            self.edit.selected = new_selected;
            offset
        };
        self.data.rebuild_model();
        Some(offset)
    }

    pub fn transpose_selected(&mut self, semitones: i8) -> Option<i8> {
        if self.edit.selected.is_empty() {
            return None;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);

            // Remove selected notes from their current key buckets.
            let mut moved_data: Vec<(yinhe_types::Note, u16, u8)> = Vec::new();
            for &(track, start_tick, key) in &self.edit.selected {
                let k = key as usize;
                if let Some(pos) = model.notes[k]
                    .iter()
                    .position(|n| n.track == track && n.start_tick == start_tick)
                {
                    let note = Arc::make_mut(&mut model.notes[k]).remove(pos);
                    moved_data.push((note, track, key));
                }
            }

            if moved_data.is_empty() {
                return None;
            }

            let mut new_selected = HashSet::new();
            for (note, track, old_key) in &moved_data {
                let new_key = ((*old_key as i16) + (semitones as i16)).clamp(0, 127) as u8;
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: *track,
                };
                let k = new_key as usize;
                let insert_pos = model.notes[k].partition_point(|n| n.start_tick < new_note.start_tick);
                Arc::make_mut(&mut model.notes[k]).insert(insert_pos, new_note);
                new_selected.insert((*track, note.start_tick, new_key));
            }

            self.edit.selected = new_selected;
        }
        self.data.rebuild_model();
        Some(semitones)
    }

    pub fn apply_undo_snapshot(&mut self, snap: crate::history::UndoSnapshot) {
        let old_version = self.data.midi_version;
        self.data = snap.data;
        self.data.midi_version = old_version.wrapping_add(1);
        self.edit.track_info_cache = self.data.track_info();
        for (i, ti) in self.edit.track_info_cache.iter().enumerate() {
            if i < self.data.track_names.len() {
                self.data.track_names[i] = ti.name.clone();
            }
        }
        self.edit.pc_map_cache = self.data.pc_map_cache();
        self.edit.selected = snap.selected;
        self.edit.track_selected = snap.track_selected;
        self.edit.sel_rect = snap.sel_rect;
    }
}

pub fn detect_conductor_from_model(model: &YinModel) -> Option<u16> {
    if model.tracks.is_empty() {
        return None;
    }
    let first = &model.tracks[0];
    if model.track_note_count.first().copied().unwrap_or(0) > 0 {
        return None;
    }
    if !first.cc.is_empty() || !first.pitch_bend.is_empty() || !first.program_change.is_empty() {
        return None;
    }
    Some(0)
}

fn build_track_info(model: &YinModel) -> Vec<yinhe_core::TrackInfo> {
    model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| yinhe_core::TrackInfo {
            index: i as u16,
            name: track.name.clone(),
            note_count: model.track_note_count.get(i).copied().unwrap_or(0),
            port: track.port,
            channel: track.channel,
        })
        .collect()
}

fn build_pc_map_cache_from_model(model: &YinModel) -> HashMap<u8, u8> {
    let mut pc_map = HashMap::new();
    for track in &model.tracks {
        for pc in &track.program_change {
            pc_map.entry(track.channel).or_insert(pc.program);
        }
    }
    pc_map
}

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
        let t = TrackData::new(0, 0);
        let mut model = YinModel {
            tracks: vec![Arc::new(t)],
            ..Default::default()
        };
        model.load_track_notes(vec![vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            dup_index: 0,
        }]]);
        model.rebuild();
        assert_eq!(detect_conductor_from_model(&model), None);
    }

    #[test]
    fn detect_conductor_some_when_track0_has_no_notes_and_no_ctrl() {
        let mut t1 = TrackData::new(0, 0);
        t1.name = "Conductor".into();
        let mut t2 = TrackData::new(0, 0);
        t2.name = "Piano".into();
        let mut model = YinModel {
            tracks: vec![Arc::new(t1), Arc::new(t2)],
            ..Default::default()
        };
        model.load_track_notes(vec![vec![], vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            dup_index: 0,
        }]]);
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
