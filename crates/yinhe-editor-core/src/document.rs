//! Per-document state: persistent data + editing state + undo history.

use std::sync::Arc;

use yinhe_core::{TrackData, YinModel};
use yinhe_types::TRACK_PALETTE;
use yinhe_yin::{MappingFile, ProjectFile};

use crate::edit_state::EditState;
use crate::history::{UndoEntry, UndoStack};
use crate::project_data::ProjectData;
use crate::quantize::QuantizePreset;

pub mod arrange_move;
pub mod automation_edit;
pub mod note_edit;
pub mod selection;
pub mod track_ops;

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

// ---------------------------------------------------------------------------
// Accessors + constructors
// ---------------------------------------------------------------------------

impl Document {
    pub fn model(&self) -> &YinModel {
        &self.data.model
    }

    pub fn track_names(&self) -> &[String] {
        &self.data.track_names
    }

    pub fn selected(&self) -> &yinhe_core::Selection {
        &self.edit.selected
    }

    pub fn track_info_cache(&self) -> &[yinhe_core::TrackInfo] {
        &self.edit.track_info_cache
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// Mark that this document was loaded from a file.
    /// Called after loading MIDI/.yin to indicate it's not a fresh empty doc.
    pub fn mark_loaded(&mut self) {
        self.history.mark_loaded();
    }

    pub fn empty() -> Self {
        let mut model = YinModel {
            conductor: Arc::new(yinhe_core::ConductorData {
                tempo: yinhe_types::AutomationLane {
                    target: yinhe_types::AutomationTarget::Tempo,
                    track: 0,
                    events: vec![yinhe_types::AutomationEvent {
                        tick: 0,
                        value: 120.0,
                        shape: yinhe_types::SegmentShape::Step,
                    }],
                },
                time_sig: vec![yinhe_types::TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                }],
            }),
            tracks: {
                let mut tracks: Vec<Arc<TrackData>> = Vec::with_capacity(17);
                let mut t = TrackData::new(0, 0);
                t.name = "Conductor".to_string();
                tracks.push(Arc::new(t));
                for ch in 0..16u8 {
                    let mut t = TrackData::new(0, ch);
                    t.name = format!("A{}", ch + 1);
                    tracks.push(Arc::new(t));
                }
                tracks
            },
            ..Default::default()
        };
        model.rebuild();

        let track_names = model.tracks.iter().map(|t| t.name.clone()).collect();
        let num_tracks = model.tracks.len();
        let conductor_track_idx = Some(0);

        let data = ProjectData::new(
            Arc::new(model),
            track_names,
            ProjectFile::default(),
            MappingFile::default(),
        );
        let track_info_cache = data.track_info();

        Document {
            data,
            edit: EditState {
                track_visible: vec![true; num_tracks],
                track_pianoroll_visible: vec![true; num_tracks],
                track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
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
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
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
                // Shift automation lane track indices by 1 to match.
                for track in model.tracks.iter_mut() {
                    let track = Arc::make_mut(track);
                    for lane in track.automation_lanes.iter_mut() {
                        lane.track += 1;
                    }
                }
                model.tracks.insert(0, Arc::new(conductor));
                model.rebuild();
            }
            let conductor_track_idx = detect_conductor_from_model(&model);

            let num_tracks = model.tracks.len();
            let track_names: Vec<String> = model.tracks.iter().map(|t| t.name.clone()).collect();
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

            let track_info_cache = data.track_info();
            let pc_map_cache = data.pc_map_cache();

            Ok(Document {
                data,
                edit: EditState {
                    quantize_arrange,
                    quantize_pianoroll,
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
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
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
        let mut doc = Self::from_model(path, model, quantize_arrange, quantize_pianoroll, project_file, mapping).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        doc.file_path = Some(path.to_string());
        Ok((doc, sf.mode))
    }

    pub fn recode_track_names(&mut self, _encoding: yinhe_mid2::MidiImportEncoding) {
        // TODO: implement track name re-encoding for YinModel
        self.data.bump_revision();
    }
}

// ---------------------------------------------------------------------------
// Undo/redo
// ---------------------------------------------------------------------------

impl Document {
    /// Undo the most recent operation. Returns true if something was undone.
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.history.past.pop_back() else {
            return false;
        };

        // Save current selection so redo can restore it.
        let current_selected = self.edit.selected.clone();
        let current_track_selected = self.edit.track_selected.clone();
        let current_sel_rect = self.edit.sel_rect.clone();

        // Apply reverse action.
        entry.action.undo(self);

        // Restore selection from the undo entry.
        self.edit.selected = entry.selected;
        self.edit.track_selected = entry.track_selected;
        self.edit.sel_rect = entry.sel_rect;

        // Push reversed action onto the redo stack.
        self.history.future.push(UndoEntry {
            action: entry.action.reversed(),
            label: entry.label,
            selected: current_selected,
            track_selected: current_track_selected,
            sel_rect: current_sel_rect,
        });

        true
    }

    /// Redo the most recently undone operation. Returns true if something was redone.
    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.history.future.pop() else {
            return false;
        };

        let current_selected = self.edit.selected.clone();
        let current_track_selected = self.edit.track_selected.clone();
        let current_sel_rect = self.edit.sel_rect.clone();

        entry.action.undo(self);

        self.edit.selected = entry.selected;
        self.edit.track_selected = entry.track_selected;
        self.edit.sel_rect = entry.sel_rect;

        self.history.past.push_back(UndoEntry {
            action: entry.action.reversed(),
            label: entry.label,
            selected: current_selected,
            track_selected: current_track_selected,
            sel_rect: current_sel_rect,
        });

        true
    }

    /// Rebuild track_info_cache, track_colors_cache, and resize track_visible/
    /// track_pianoroll_visible/track_overrides to match current track count.
    /// Called after track structure changes (add/remove/move/undo/redo).
    pub(crate) fn sync_track_caches(&mut self) {
        self.edit.track_info_cache = self.data.track_info();
        let num_tracks = self.data.model.tracks.len();
        self.edit.track_colors_cache = (0..num_tracks)
            .map(|i| track_color(i, self.edit.conductor_track_idx))
            .collect();
        while self.edit.track_visible.len() < num_tracks {
            self.edit.track_visible.push(true);
        }
        while self.edit.track_pianoroll_visible.len() < num_tracks {
            self.edit.track_pianoroll_visible.push(true);
        }
        while self.edit.track_overrides.len() < num_tracks {
            self.edit.track_overrides.push(Default::default());
        }
        while self.edit.track_visible.len() > num_tracks {
            self.edit.track_visible.pop();
        }
        while self.edit.track_pianoroll_visible.len() > num_tracks {
            self.edit.track_pianoroll_visible.pop();
        }
        while self.edit.track_overrides.len() > num_tracks {
            self.edit.track_overrides.pop();
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Detect the conductor track: track 0 with no notes and no control data.
pub fn detect_conductor_from_model(model: &YinModel) -> Option<u16> {
    if model.tracks.is_empty() {
        return None;
    }
    let first = &model.tracks[0];
    if model.track_note_count.first().copied().unwrap_or(0) > 0 {
        return None;
    }
    if !first.automation_lanes.is_empty() || !first.program_change.is_empty() {
        return None;
    }
    Some(0)
}

/// Track color from palette, with conductor offset.
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
    fn empty_creates_valid_document_with_conductor_and_16_tracks() {
        let doc = Document::empty();
        assert_eq!(doc.model().tracks.len(), 17);
        assert_eq!(doc.model().tracks[0].name, "Conductor");
        assert_eq!(doc.model().tracks[1].name, "A1");
        assert_eq!(doc.model().tracks[16].name, "A16");
        assert_eq!(doc.track_names().len(), 17);
        assert_eq!(doc.edit.conductor_track_idx, Some(0));
        assert_eq!(doc.edit.track_visible.len(), 17);
        assert_eq!(doc.edit.track_pianoroll_visible.len(), 17);
        assert_eq!(doc.file_name, "Untitled");
        // Conductor track channels: A1 on ch0, A16 on ch15
        assert_eq!(doc.model().tracks[1].channel, 0);
        assert_eq!(doc.model().tracks[16].channel, 15);
    }

    #[test]
    fn detect_conductor_none_when_track0_has_notes() {
        let t = TrackData::new(0, 0);
        let mut model = YinModel {
            tracks: vec![Arc::new(t)],
            ..Default::default()
        };
        model.load_track_notes(vec![vec![yinhe_core::NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
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
        model.load_track_notes(vec![vec![], vec![yinhe_core::NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
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
