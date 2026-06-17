use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
    /// Project archive (for Event Browser archive view + .yin save).
    pub archive: Option<yinhe_project::ProjectArchive>,
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

impl Document {
    // ── Convenience accessors for the most common fields ──

    pub fn midi(&self) -> &Arc<yinhe_midi::MidiFile> {
        &self.data.midi
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
        let mut m = yinhe_midi::MidiFile::default();
        m.track_ports = vec![0];
        m.track_names = vec!["Track 1".to_string()];
        let track_names = m.track_names.clone();
        let track_info_cache = m.track_info();
        let num_tracks = 1;
        let conductor_track_idx = None;
        Document {
            data: ProjectData {
                midi: Arc::new(m),
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
            archive: None,
        }
    }

    pub fn from_midi(
        path: &str,
        midi: yinhe_midi::MidiFile,
        quantize: QuantizePreset,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let archive = yinhe_project::conversion::midi_to_archive(&midi);
            let mut midi = yinhe_project::conversion::archive_to_midi(&archive);
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let track_info_cache_initial = midi.track_info();
            let conductor_track_idx = detect_conductor(&track_info_cache_initial, &midi.control_events);
            if conductor_track_idx.is_none() {
                for notes in &mut midi.key_notes {
                    for note in notes.iter_mut() {
                        note.track += 1;
                    }
                }
                for ev in &mut midi.control_events {
                    match ev {
                        yinhe_midi::MidiControlEvent::ControlChange { track, .. }
                        | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
                        | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track += 1,
                    }
                }
                midi.track_ports.insert(0, 0);
                midi.track_channels.insert(0, 0);
                midi.track_names.insert(0, "Conductor".to_string());
                midi.track_channel_prefixes.insert(0, None);
            }
            let track_info_cache_initial = midi.track_info();
            let conductor_track_idx = detect_conductor(&track_info_cache_initial, &midi.control_events);
            let num_tracks = midi.track_ports.len();

            let mut track_names: Vec<String> = (0..num_tracks)
                .map(|i| {
                    midi.track_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("Track {}", i + 1))
                })
                .collect();

            let mut project_name_override: Option<String> = None;
            if let Some(c_idx) = conductor_track_idx {
                let c = c_idx as usize;
                if let Some(name) = track_names.get(c) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with("Track ") {
                        project_name_override = Some(trimmed.to_string());
                    }
                }
                if c < track_names.len() {
                    track_names[c] = "Conductor".to_string();
                }
                if c < midi.track_names.len() {
                    midi.track_names[c] = "Conductor".to_string();
                } else {
                    while midi.track_names.len() < c {
                        midi.track_names.push(String::new());
                    }
                    midi.track_names.push("Conductor".to_string());
                }
            }
            let track_info_cache = midi.track_info();

            let pc_map_cache = build_pc_map_cache(&midi);

            let ticks_per_beat = midi.ticks_per_beat;
            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(i, conductor_track_idx))
                .collect();

            Ok(Document {
                data: ProjectData {
                    midi: Arc::new(midi),
                    track_names,
                    project_name: project_name_override.unwrap_or_default(),
                    project_artist: String::new(),
                    project_description: String::new(),
                    project_ppq: ticks_per_beat,
                    compression_level: 0,
                    midi_version: 0,
                },
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
                archive: Some(archive),
            })
        })
    }

    pub fn from_yin(
        path: &str,
        quantize: QuantizePreset,
    ) -> std::io::Result<(Self, bool)> {
        let (midi, file_name, archive) = yinhe_project::conversion::load_project_full(path)?;
        let mut midi = midi;
        let num_tracks = midi.track_ports.len();
        let mut track_names: Vec<String> = (0..num_tracks)
            .map(|i| {
                midi.track_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Track {}", i + 1))
            })
            .collect();
        let track_info_cache_initial = midi.track_info();
        let conductor_track_idx = detect_conductor(&track_info_cache_initial, &midi.control_events);
        if let Some(c_idx) = conductor_track_idx {
            let c = c_idx as usize;
            if c < track_names.len() {
                track_names[c] = "Conductor".to_string();
            }
            if c < midi.track_names.len() {
                midi.track_names[c] = "Conductor".to_string();
            } else {
                while midi.track_names.len() < c {
                    midi.track_names.push(String::new());
                }
                midi.track_names.push("Conductor".to_string());
            }
        }
        let track_info_cache = midi.track_info();

        let pc_map_cache = build_pc_map_cache(&midi);

        // Extract project metadata from archive (don't store archive itself)
        let (
            project_name,
            project_artist,
            project_description,
            project_ppq,
            project_sf,
            soundfont_project_mode,
        ) = archive
            .get_json::<yinhe_project::ProjectJson>("project.json")
            .map(|p| {
                let sf = ProjectSfConfig {
                    overrides: p
                        .soundfont_overrides
                        .into_iter()
                        .map(|ov| {
                            (
                                ov.port,
                                ov.entries
                                    .into_iter()
                                    .map(|e| SfEntry {
                                        path: e.path,
                                        name: e.name,
                                        enabled: e.enabled,
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                };
                (p.name, p.artist, p.description, p.ppq, sf, p.soundfont_project_mode)
            })
            .unwrap_or((
                String::new(),
                String::new(),
                String::new(),
                480,
                ProjectSfConfig::default(),
                false,
            ));

        let compression_level = archive.compression_level;
        let track_colors_cache = (0..num_tracks)
            .map(|i| track_color(i, conductor_track_idx))
            .collect();

        Ok((Document {
            data: ProjectData {
                midi: Arc::new(midi),
                track_names,
                project_name,
                project_artist,
                project_description,
                project_ppq,
                compression_level,
                midi_version: 0,
            },
            edit: EditState {
                quantize,
                track_visible: vec![true; num_tracks],
                track_pianoroll_visible: vec![true; num_tracks],
                track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
                track_info_cache,
                pc_map_cache,
                track_colors_cache,
                project_sf,
                conductor_track_idx,
                ..Default::default()
            },
            history: UndoStack::new(),
            file_name,
            file_path: Some(path.to_string()),
            archive: Some(archive),
        }, soundfont_project_mode))
    }

    /// Re-decode all track names using a different encoding.
    pub fn recode_track_names(&mut self, encoding: yinhe_midi::MidiImportEncoding) {
        {
            Arc::make_mut(&mut self.data.midi).recode_track_names(encoding);
        }
        self.data.midi_version = self.data.midi_version.wrapping_add(1);
        for (i, name) in self.data.midi.track_names.iter().enumerate() {
            if i < self.data.track_names.len() {
                self.data.track_names[i] = name.clone();
            }
            if let Some(ti) = self.edit.track_info_cache.get_mut(i) {
                ti.name = name.clone();
            }
        }
    }

    // ── Note editing operations ──

    /// Delete all selected notes. Returns `true` if any notes were removed.
    pub fn delete_selected(&mut self) -> bool {
        if self.edit.selected.is_empty() {
            return false;
        }
        {
            let midi = Arc::make_mut(&mut self.data.midi);
            for &(track, start_tick, key) in &self.edit.selected {
                let notes = &mut midi.key_notes[key as usize];
                notes.retain(|n| !(n.track == track && n.start_tick == start_tick));
            }
            self.edit.selected.clear();
        }
        self.data.rebuild_midi_metadata();
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
            let midi = Arc::make_mut(&mut self.data.midi);

            let mut selected_data: Vec<(yinhe_types::Note, u8)> = Vec::new();
            for &(track, start_tick, key) in &self.edit.selected {
                if let Some(note) = midi.key_notes[key as usize]
                    .iter()
                    .find(|n| n.track == track && n.start_tick == start_tick)
                {
                    selected_data.push((note.clone(), key));
                }
            }

            if selected_data.is_empty() {
                return false;
            }

            let min_start = selected_data.iter().map(|(n, _)| n.start_tick).min().unwrap();
            let max_end = selected_data.iter().map(|(n, _)| n.end_tick).max().unwrap();
            let offset = (max_end - min_start).max(1);

            let mut new_selected = HashSet::new();
            for (note, key) in &selected_data {
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick + offset,
                    end_tick: note.end_tick + offset,
                    ..note.clone()
                };
                let notes = &mut midi.key_notes[*key as usize];
                let insert_pos = notes.partition_point(|n| n.start_tick < new_note.start_tick);
                notes.insert(insert_pos, new_note);
                new_selected.insert((note.track, note.start_tick + offset, *key));
            }

            self.edit.selected = new_selected;
        }
        self.data.rebuild_midi_metadata();
        true
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave,
    /// -12 for down). Returns `true` if any notes were moved.
    pub fn transpose_selected(&mut self, semitones: i8) -> bool {
        if self.edit.selected.is_empty() {
            return false;
        }
        {
            let midi = Arc::make_mut(&mut self.data.midi);

            let mut moved_data: Vec<(yinhe_types::Note, u8)> = Vec::new();
            for &(track, start_tick, key) in &self.edit.selected {
                let notes = &mut midi.key_notes[key as usize];
                if let Some(pos) = notes.iter().position(|n| n.track == track && n.start_tick == start_tick)
                {
                    let note = notes.remove(pos);
                    moved_data.push((note, key));
                }
            }

            if moved_data.is_empty() {
                return false;
            }

            let mut new_selected = HashSet::new();
            for (note, old_key) in &moved_data {
                let new_key = ((*old_key as i16) + (semitones as i16)).clamp(0, 127) as u8;
                let notes = &mut midi.key_notes[new_key as usize];
                let insert_pos = notes.partition_point(|n| n.start_tick < note.start_tick);
                notes.insert(insert_pos, note.clone());
                new_selected.insert((note.track, note.start_tick, new_key));
            }

            self.edit.selected = new_selected;
        }
        self.data.rebuild_midi_metadata();
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

/// Detect the conductor track using a SMF format-1 heuristic.
pub fn detect_conductor(
    track_info: &[yinhe_midi::TrackInfo],
    control_events: &[yinhe_midi::MidiControlEvent],
) -> Option<u16> {
    if track_info.is_empty() {
        return None;
    }
    if track_info[0].note_count != 0 {
        return None;
    }
    let has_ctrl_on_zero = control_events.iter().any(|e| match e {
        yinhe_midi::MidiControlEvent::ControlChange { track, .. }
        | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
        | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track == 0,
    });
    if has_ctrl_on_zero {
        return None;
    }
    Some(0)
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

/// Build the pc_map_cache from MidiFile control events.
fn build_pc_map_cache(midi: &yinhe_midi::MidiFile) -> HashMap<u8, u8> {
    let mut pc_map = HashMap::new();
    for ev in &midi.control_events {
        if let yinhe_midi::MidiControlEvent::ProgramChange {
            program, track, ..
        } = ev
        {
            let ch = midi.track_channels.get(*track as usize).copied().unwrap_or(0);
            pc_map.entry(ch).or_insert(*program);
        }
    }
    pc_map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_creates_valid_document_with_one_track() {
        let doc = Document::empty();
        assert_eq!(doc.midi().track_ports.len(), 1);
        assert_eq!(doc.midi().track_names.len(), 1);
        assert_eq!(doc.midi().track_names[0], "Track 1");
        assert_eq!(doc.track_names().len(), 1);
        assert_eq!(doc.track_names()[0], "Track 1");
        assert!(doc.edit.conductor_track_idx.is_none());
        assert_eq!(doc.edit.track_visible.len(), 1);
        assert_eq!(doc.edit.track_pianoroll_visible.len(), 1);
        assert_eq!(doc.file_name, "Untitled");
    }

    #[test]
    fn detect_conductor_none_when_track0_has_notes() {
        let track_info = vec![yinhe_midi::TrackInfo {
            index: 0,
            name: "Piano".into(),
            note_count: 10,
            port: 0,
            channel: 0,
        }];
        assert_eq!(detect_conductor(&track_info, &[]), None);
    }

    #[test]
    fn detect_conductor_some_when_track0_has_no_notes_and_no_ctrl() {
        let track_info = vec![
            yinhe_midi::TrackInfo {
                index: 0,
                name: "Conductor".into(),
                note_count: 0,
                port: 0,
                channel: 0,
            },
            yinhe_midi::TrackInfo {
                index: 1,
                name: "Piano".into(),
                note_count: 5,
                port: 0,
                channel: 0,
            },
        ];
        assert_eq!(detect_conductor(&track_info, &[]), Some(0));
    }

    #[test]
    fn detect_conductor_none_when_track0_no_notes_but_has_ctrl() {
        let track_info = vec![yinhe_midi::TrackInfo {
            index: 0,
            name: "CC Track".into(),
            note_count: 0,
            port: 0,
            channel: 0,
        }];
        let ctrl = vec![yinhe_midi::MidiControlEvent::ControlChange {
            tick: 0,
            controller: 7,
            value: 100,
            track: 0,
        }];
        assert_eq!(detect_conductor(&track_info, &ctrl), None);
    }

    #[test]
    fn detect_conductor_none_for_empty_track_info() {
        assert_eq!(detect_conductor(&[], &[]), None);
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
        // Track 1 after conductor at 0 should use palette index 0 (1-1=0)
        let color = track_color(1, Some(0));
        assert_eq!(color, TRACK_PALETTE[0]);
    }

    #[test]
    fn build_pc_map_cache_via_from_midi() {
        let mut m = yinhe_midi::MidiFile::default();
        m.track_ports = vec![0, 0];
        m.track_names = vec!["Conductor".into(), "Piano".into()];
        m.track_channels = vec![0, 0];
        m.track_channel_prefixes = vec![None, None];
        m.control_events = vec![yinhe_midi::MidiControlEvent::ProgramChange {
            tick: 0,
            program: 48,
            track: 1,
        }];
        let doc = Document::from_midi("test.mid", m, QuantizePreset::default())
            .expect("from_midi failed");
        assert_eq!(doc.edit.pc_map_cache.get(&0), Some(&48));
    }
}
