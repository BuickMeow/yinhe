use std::sync::Arc;

use yinhe_core::{NoteEvent, TrackData, YinModel};
use yinhe_types::{AutomationEdit, AutomationEvent, PencilNoteDrag, TRACK_PALETTE};
use yinhe_yin::{MappingFile, ProjectFile};

use crate::edit_state::EditState;
use crate::history::{AutomationDelta, NoteDelta, UndoAction, UndoEntry, UndoStack};
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
                tempo: vec![yinhe_core::TempoEvent { tick: 0, bpm: 120.0 }],
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
        self.data.bump_version();
    }
}

// ---------------------------------------------------------------------------
// Note editing methods — each returns Option<UndoAction> for the caller to push
// ---------------------------------------------------------------------------

impl Document {
    /// Undo the most recent operation. Returns true if something was undone.
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.history.past.pop() else {
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

        self.history.past.push(UndoEntry {
            action: entry.action.reversed(),
            label: entry.label,
            selected: current_selected,
            track_selected: current_track_selected,
            sel_rect: current_sel_rect,
        });

        true
    }
}

impl Document {
    /// Add a single note. Returns an `UndoAction` if the note was added.
    pub fn add_note(&mut self, track_idx: u16, note: NoteEvent) -> Option<UndoAction> {
        let t = track_idx as usize;
        if t >= self.data.model.tracks.len() {
            return None;
        }
        if Some(track_idx) == self.edit.conductor_track_idx {
            return None;
        }
        let key = note.key;
        let typed_note = yinhe_types::Note {
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            velocity: note.velocity,
            dup_index: note.dup_index,
            track: track_idx,
        };
        {
            let model = Arc::make_mut(&mut self.data.model);
            let k = key as usize;
            let insert_pos = model.notes[k].partition_point(|n| n.start_tick < note.start_tick);
            Arc::make_mut(&mut model.notes[k]).insert(insert_pos, typed_note);
            model.mark_dirty(key);
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after: vec![(typed_note, key)],
        }))
    }

    /// Delete all selected notes. Returns an `UndoAction` if any notes were deleted.
    pub fn delete_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        // Collect before any mutation.
        let matched = crate::batch_ops::collect_selected(&self.data.model, &self.edit.selected);
        if matched.is_empty() {
            self.edit.selected.clear();
            return None;
        }
        {
            let model = Arc::make_mut(&mut self.data.model);
            crate::batch_ops::remove_selected(model, &self.edit.selected);
            self.edit.selected.clear();
        }
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: matched,
            after: vec![],
        }))
    }

    /// Duplicate all selected notes. Returns an `UndoAction` if any notes were duplicated.
    pub fn duplicate_selected(&mut self) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let after = {
            let model = Arc::make_mut(&mut self.data.model);

            let selected_data = crate::batch_ops::collect_selected(model, &self.edit.selected);
            if selected_data.is_empty() {
                return None;
            }

            let min_start = selected_data.iter().map(|(n, _)| n.start_tick).min().unwrap();
            let max_end = selected_data.iter().map(|(n, _)| n.end_tick).max().unwrap();
            let offset = (max_end - min_start).max(1);

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, key) in &selected_data {
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick + offset,
                    end_tick: note.end_tick + offset,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: note.track,
                };
                new_by_key.entry(*key).or_default().push(new_note);
            }

            // Build after vec before moving new_by_key.
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

            crate::batch_ops::insert_batch(model, new_by_key);

            // Offset selection rects to cover the duplicated notes.
            self.edit.selected.offset(offset as i64, 0);
            after
        };
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// Transpose all selected notes by `semitones`. Returns an `UndoAction` if any notes were transposed.
    pub fn transpose_selected(&mut self, semitones: i8) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let (before, after) = {
            let model = Arc::make_mut(&mut self.data.model);

            let moved_data = crate::batch_ops::remove_selected(model, &self.edit.selected);
            if moved_data.is_empty() {
                return None;
            }

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            for (note, old_key) in &moved_data {
                let new_key = ((*old_key as i16) + (semitones as i16)).clamp(0, 127) as u8;
                let new_note = yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: note.track,
                };
                new_by_key.entry(new_key).or_default().push(new_note);
            }

            // Build after vec before moving new_by_key.
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();

            crate::batch_ops::insert_batch(model, new_by_key);

            // Offset selection rects to follow the transposed notes.
            self.edit.selected.offset(0, semitones as i32);
            (moved_data, after)
        };
        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta { before, after }))
    }

    // ── Select All ──

    /// Select all notes in the currently selected track(s) for Piano Roll.
    /// Range: tick 0 → last note end (global), keys 0–127.
    ///
    /// Uses `model.tick_length` (O(1)) instead of scanning all key buckets (O(N)).
    /// Sets `sel_rect.rect` to the full global range so the visual selection box
    /// covers 0 → tick_length, keys 0–127.
    pub fn select_all_pr(&mut self) {
        let model = &self.data.model;
        let max_end = model.tick_length as u32;
        if max_end == 0 {
            return;
        }

        let conductor = self.edit.conductor_track_idx;
        let tracks: Vec<u16> = if self.edit.track_selected.is_empty() {
            // 没有预选 track 时，全选所有非 conductor track
            let num_tracks = model.tracks.len() as u16;
            (0..num_tracks).filter(|&t| Some(t) != conductor).collect()
        } else {
            self.edit.track_selected.iter().copied().collect()
        };
        if tracks.is_empty() {
            return;
        }

        self.edit.selected.clear();
        for &track_idx in &tracks {
            if Some(track_idx) == conductor {
                continue;
            }
            self.edit.selected.add_rect_track(0, max_end + 1, 0, 127, track_idx, track_idx);
        }

        // Update visual sel_rect to show full range (PR uses f64 ticks).
        self.edit.sel_rect.rect = Some((0.0, max_end as f64 + 1.0, 0, 127));
    }

    /// Select all notes across all tracks for Arrange.
    /// Range: tick 0 → global last note end, keys 0–127, all tracks except conductor.
    pub fn select_all_ar(&mut self) {
        let model = &self.data.model;
        let max_end = model.tick_length as u32;
        if max_end == 0 {
            return;
        }
        let conductor = self.edit.conductor_track_idx;
        let num_tracks = model.tracks.len() as u16;

        self.edit.selected.clear();
        // One rect per non-conductor track range is overkill; use a single
        // broad rect and rely on conductor guard in add_note / move_selected.
        // But to be precise, split into: tracks before conductor, tracks after.
        match conductor {
            Some(c) if c > 0 => {
                self.edit.selected.add_rect_track(0, max_end + 1, 0, 127, 0, c - 1);
            }
            _ => {}
        }
        let after = conductor.map(|c| c + 1).unwrap_or(0);
        if after < num_tracks {
            self.edit.selected.add_rect_track(0, max_end + 1, 0, 127, after, num_tracks - 1);
        }

        // Update visual sel_rect to show full range (Arrange uses f64 ticks).
        self.edit.sel_rect.rect = Some((0.0, max_end as f64 + 1.0, 0, 127));
    }

    // ── Paste from clipboard ──

    /// Paste notes from clipboard (selection rects) at the cursor position.
    ///
    /// Clipboard stores only selection rects (not note data) for performance.
    /// Notes are queried from the model at paste time. If the notes have been
    /// deleted (e.g. after cut), falls back to the undo entry identified by
    /// `cut_past_len` which contains the deleted notes in its `before` field.
    pub fn paste_from_selection(
        &mut self,
        clipboard: &yinhe_core::Selection,
        cursor_tick: f64,
        cut_past_len: Option<usize>,
    ) -> Option<UndoAction> {
        if clipboard.is_empty() {
            return None;
        }

        // Try querying the model first (normal copy-paste).
        let model = &self.data.model;
        let mut notes = crate::batch_ops::collect_selected(model, clipboard);

        // Undo bridge: if model query returned nothing (notes were cut/deleted),
        // fall back to the correct undo entry identified by cut_past_len.
        //
        // cut_past_len was captured as past.len() BEFORE the delete was pushed.
        // After push, the delete entry sits at index `cut_past_len` (push appends
        // at the end, so old length = new entry's index).
        if notes.is_empty() {
            let entry = cut_past_len
                .and_then(|len| self.history.past.get(len))
                .or_else(|| self.history.past.last());
            if let Some(entry) = entry {
                if let UndoAction::Notes(delta) = &entry.action {
                    if !delta.before.is_empty() {
                        notes = delta.before.iter()
                            .filter(|(n, key)| clipboard.contains(n.track, n.start_tick, *key))
                            .cloned()
                            .collect();
                    }
                }
            }
        }

        if notes.is_empty() {
            return None;
        }

        // Calculate offset: cursor - min start_tick.
        let min_start = notes.iter().map(|(n, _)| n.start_tick).min().unwrap_or(0);
        let offset = (cursor_tick as i64 - min_start as i64).max(0) as u32;

        let conductor = self.edit.conductor_track_idx;
        let model = Arc::make_mut(&mut self.data.model);

        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
            std::collections::HashMap::new();
        for (note, key) in &notes {
            if Some(note.track) == conductor {
                continue;
            }
            let new_note = yinhe_types::Note {
                start_tick: note.start_tick + offset,
                end_tick: note.end_tick + offset,
                velocity: note.velocity,
                dup_index: 0,
                track: note.track,
            };
            new_by_key.entry(*key).or_default().push(new_note);
        }

        if new_by_key.is_empty() {
            return None;
        }

        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();

        crate::batch_ops::insert_batch(model, new_by_key);

        // Update selection to cover pasted notes.
        self.edit.selected.clear();
        let max_end = after.iter().map(|(n, _)| n.end_tick).max().unwrap_or(0);
        let min_tick = after.iter().map(|(n, _)| n.start_tick).min().unwrap_or(0);
        let mut track_lo = u16::MAX;
        let mut track_hi = 0u16;
        for (n, _) in &after {
            track_lo = track_lo.min(n.track);
            track_hi = track_hi.max(n.track);
        }
        self.edit.selected.add_rect_track(min_tick, max_end + 1, 0, 127, track_lo, track_hi);

        self.data.rebuild_model_dirty();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// 在指定 track 的指定 lane 上添加一个 automation 事件。
    ///
    /// 如果该 track 没有 target 对应的 lane，会先创建。
    /// 返回 (track_idx, lane_idx, UndoAction)，调用方需把 UndoAction push 到 history。
    pub fn add_automation_event(
        &mut self,
        track_idx: usize,
        target: yinhe_types::AutomationTarget,
        event: yinhe_types::AutomationEvent,
    ) -> Option<(usize, usize, UndoAction)> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);

        // 找或创建 lane
        let lane_idx = match track.automation_lanes.iter().position(|l| l.target == target) {
            Some(idx) => idx,
            None => {
                track.automation_lanes.push(yinhe_types::AutomationLane {
                    target: target.clone(),
                    track: track_idx as u16,
                    events: Vec::new(),
                });
                track.automation_lanes.len() - 1
            }
        };
        let lane = &mut track.automation_lanes[lane_idx];

        let before = lane.events.clone();
        let insert_pos = lane.events.partition_point(|e| e.tick < event.tick);
        lane.events.insert(insert_pos, event);
        let after = lane.events.clone();

        self.data.midi_version = self.data.midi_version.wrapping_add(1);
        Some((track_idx, lane_idx, UndoAction::Automation(crate::history::AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        })))
    }

    /// 移动指定 lane 上 tick=`old_tick` 的事件到 `(new_tick, new_value)`。
    /// 如果 `new_tick` 与同 lane 已有事件冲突，会先移除冲突项。
    pub fn move_automation_event(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        old_tick: u32,
        new_tick: u32,
        new_value: u16,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let lane = track.automation_lanes.get_mut(lane_idx)?;

        let before = lane.events.clone();
        // 验证原事件存在
        lane.events.iter().position(|e| e.tick == old_tick)?;

        if old_tick == new_tick {
            // 只改 value，不改 tick：直接原地修改，避免 retain 误删原事件
            let evt = lane.events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.value = new_value;
        } else {
            // 移除目标 tick 上已有的事件（避免重复 tick）
            lane.events.retain(|e| e.tick != new_tick);
            // 找到原事件并修改
            let evt = lane.events.iter_mut().find(|e| e.tick == old_tick)?;
            evt.tick = new_tick;
            evt.value = new_value;
            lane.events.sort_by_key(|e| e.tick);
        }
        let after = lane.events.clone();

        self.data.midi_version = self.data.midi_version.wrapping_add(1);
        Some(UndoAction::Automation(crate::history::AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        }))
    }

    /// 删除指定 lane 上 tick=`tick` 的事件。
    pub fn delete_automation_event(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        tick: u32,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let lane = track.automation_lanes.get_mut(lane_idx)?;

        let before = lane.events.clone();
        lane.events.retain(|e| e.tick != tick);
        if before.len() == lane.events.len() {
            return None;
        }
        let after = lane.events.clone();

        self.data.midi_version = self.data.midi_version.wrapping_add(1);
        Some(UndoAction::Automation(crate::history::AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        }))
    }

    /// 修改指定 lane 上 tick=`tick` 的事件的 shape。
    pub fn set_automation_shape(
        &mut self,
        track_idx: usize,
        lane_idx: usize,
        tick: u32,
        shape: yinhe_types::SegmentShape,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let lane = track.automation_lanes.get_mut(lane_idx)?;

        let before = lane.events.clone();
        let evt = lane.events.iter_mut().find(|e| e.tick == tick)?;
        if evt.shape == shape {
            return None;
        }
        evt.shape = shape;
        let after = lane.events.clone();

        self.data.midi_version = self.data.midi_version.wrapping_add(1);
        Some(UndoAction::Automation(crate::history::AutomationDelta {
            track_idx,
            lane_idx,
            before,
            after,
        }))
    }

    /// 查找指定 track 上 target 对应的 lane 索引。
    pub fn find_automation_lane(
        &self,
        track_idx: usize,
        target: &yinhe_types::AutomationTarget,
    ) -> Option<usize> {
        let track = self.data.model.tracks.get(track_idx)?;
        track.automation_lanes.iter().position(|l| &l.target == target)
    }

    // -----------------------------------------------------------------------
    // P0 refactoring: editing logic moved from yinhe-egui::ui_helpers
    // -----------------------------------------------------------------------

    /// Move all selected notes by (delta_ticks, delta_keys).
    ///
    /// Returns an `UndoAction` if any notes were moved. The caller is
    /// responsible for pushing it to the history stack, marking the view
    /// dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn move_selected_notes(&mut self, delta_ticks: i64, delta_keys: i32) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        if delta_ticks == 0 && delta_keys == 0 {
            return None;
        }

        let model = Arc::make_mut(&mut self.data.model);

        // Batch removal + collect removed notes.
        let originals = crate::batch_ops::remove_selected(model, &self.edit.selected);

        // Batch insert: group by destination key, extend.
        let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> = std::collections::HashMap::new();
        for (note, old_key) in &originals {
            let new_key = ((*old_key as i32) + delta_keys).clamp(0, 127) as u8;
            let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
            let length = note.end_tick - note.start_tick;
            let moved = yinhe_types::Note {
                start_tick: new_tick,
                end_tick: new_tick + length,
                velocity: note.velocity,
                dup_index: 0,
                track: note.track,
            };
            new_by_key.entry(new_key).or_default().push(moved);
        }
        let after: Vec<(yinhe_types::Note, u8)> = new_by_key
            .iter()
            .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
            .collect();
        crate::batch_ops::insert_batch(model, new_by_key);

        // Offset selection rects to follow the moved notes.
        self.edit.selected.offset(delta_ticks, delta_keys);
        model.rebuild_dirty();
        self.data.midi_version = self.data.midi_version.wrapping_add(1);

        Some(UndoAction::Notes(NoteDelta {
            before: originals,
            after,
        }))
    }

    /// Move all selected notes and automation events by `(delta_ticks, delta_tracks)`.
    ///
    /// This is the single atomic operation for AR arrange drag. It:
    /// 1. Collects all notes in the selection (using original selection rects)
    /// 2. Removes them from the model
    /// 3. Re-inserts them at new tick + new track
    /// 4. Moves automation events (same track or cross-track)
    /// 5. Offsets the selection rects to follow
    ///
    /// Returns a single `Composite` UndoAction (or None if nothing moved).
    pub fn move_selected_arrange(&mut self, delta_ticks: i64, delta_tracks: i32) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        if delta_ticks == 0 && delta_tracks == 0 {
            return None;
        }

        let mut sub_actions: Vec<UndoAction> = Vec::new();
        let model = Arc::make_mut(&mut self.data.model);
        let num_tracks = model.tracks.len() as i32;
        let rects = self.edit.selected.rects.clone();

        // ── 1. Move notes (tick + track in one pass) ──
        // Collect originals, remove from model, re-insert at new positions.
        let originals = crate::batch_ops::remove_selected(model, &self.edit.selected);
        if !originals.is_empty() {
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> = std::collections::HashMap::new();
            for (note, old_key) in &originals {
                let new_tick = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                let raw_track = (note.track as i32 + delta_tracks).clamp(0, num_tracks - 1);
                // Skip over conductor track: notes cannot land on it.
                let new_track = if Some(raw_track as u16) == self.edit.conductor_track_idx {
                    if delta_tracks < 0 {
                        // Moving up: clamp to first non-conductor track
                        (raw_track + 1).min(num_tracks - 1) as u16
                    } else {
                        // Moving down: clamp to last track before conductor (or stay)
                        (raw_track - 1).max(0) as u16
                    }
                } else {
                    raw_track as u16
                };
                let length = note.end_tick - note.start_tick;
                let moved = yinhe_types::Note {
                    start_tick: new_tick,
                    end_tick: new_tick + length,
                    velocity: note.velocity,
                    dup_index: 0,
                    track: new_track,
                };
                new_by_key.entry(*old_key).or_default().push(moved);
            }
            let after: Vec<(yinhe_types::Note, u8)> = new_by_key
                .iter()
                .flat_map(|(key, notes)| notes.iter().map(|n| (*n, *key)))
                .collect();
            crate::batch_ops::insert_batch(model, new_by_key);
            sub_actions.push(UndoAction::Notes(NoteDelta {
                before: originals,
                after,
            }));
        }

        // ── 2. Move automation events (tick + track in one pass) ──
        // Collect per-lane: (src_track, lane_idx, target, moved_events, remaining_events)
        struct LaneMove {
            src_track: usize,
            lane_idx: usize,
            target: yinhe_types::AutomationTarget,
            events: Vec<AutomationEvent>,
            remaining: Vec<AutomationEvent>,
        }
        let mut lane_moves: Vec<LaneMove> = Vec::new();

        for &(tick_start, tick_end, _key_lo, _key_hi, track_lo, track_hi) in &rects {
            for track_idx in track_lo..=track_hi {
                let track_idx = track_idx as usize;
                if track_idx >= model.tracks.len() {
                    continue;
                }
                let track = Arc::make_mut(&mut model.tracks[track_idx]);
                for lane_idx in 0..track.automation_lanes.len() {
                    let lane = &track.automation_lanes[lane_idx];
                    let mut in_range: Vec<AutomationEvent> = Vec::new();
                    let mut out_of_range: Vec<AutomationEvent> = Vec::new();
                    for evt in lane.events.iter() {
                        if evt.tick >= tick_start && evt.tick < tick_end {
                            let mut moved = *evt;
                            moved.tick = (moved.tick as i64 + delta_ticks).max(0) as u32;
                            in_range.push(moved);
                        } else {
                            out_of_range.push(*evt);
                        }
                    }
                    if !in_range.is_empty() {
                        lane_moves.push(LaneMove {
                            src_track: track_idx,
                            lane_idx,
                            target: lane.target.clone(),
                            events: in_range,
                            remaining: out_of_range,
                        });
                    }
                }
            }
        }

        for lm in &lane_moves {
            // Source lane: replace with remaining
            let src_track = Arc::make_mut(&mut model.tracks[lm.src_track]);
            let src_lane = &mut src_track.automation_lanes[lm.lane_idx];
            let before_src = src_lane.events.clone();
            src_lane.events = lm.remaining.clone();

            if delta_tracks == 0 {
                // Same lane: add moved events back with offset ticks
                src_lane.events.extend(lm.events.iter().copied());
                src_lane.events.sort_by_key(|e| e.tick);
            }
            sub_actions.push(UndoAction::Automation(AutomationDelta {
                track_idx: lm.src_track,
                lane_idx: lm.lane_idx,
                before: before_src,
                after: src_lane.events.clone(),
            }));
        }

        if delta_tracks != 0 {
            // Cross-track: add moved events to destination tracks
            for lm in &lane_moves {
                let raw_dst = (lm.src_track as i32 + delta_tracks)
                    .clamp(0, num_tracks - 1);
                // Skip over conductor track: automation cannot land on it.
                let dst_track_idx = if Some(raw_dst as u16) == self.edit.conductor_track_idx {
                    if delta_tracks < 0 {
                        (raw_dst + 1).min(num_tracks - 1) as usize
                    } else {
                        (raw_dst - 1).max(0) as usize
                    }
                } else {
                    raw_dst as usize
                };
                if dst_track_idx == lm.src_track {
                    continue; // clamped to same track, events already in source lane
                }
                let dst_track = Arc::make_mut(&mut model.tracks[dst_track_idx]);
                let dst_lane_idx = match dst_track.automation_lanes.iter().position(|l| l.target == lm.target) {
                    Some(idx) => idx,
                    None => {
                        dst_track.automation_lanes.push(yinhe_types::AutomationLane {
                            target: lm.target.clone(),
                            track: dst_track_idx as u16,
                            events: Vec::new(),
                        });
                        dst_track.automation_lanes.len() - 1
                    }
                };
                let dst_lane = &mut dst_track.automation_lanes[dst_lane_idx];
                let before_dst = dst_lane.events.clone();
                dst_lane.events.extend(lm.events.iter().copied());
                dst_lane.events.sort_by_key(|e| e.tick);
                sub_actions.push(UndoAction::Automation(AutomationDelta {
                    track_idx: dst_track_idx,
                    lane_idx: dst_lane_idx,
                    before: before_dst,
                    after: dst_lane.events.clone(),
                }));
            }
        }

        // ── 3. Offset selection rects to follow ──
        self.edit.selected.offset_ticks(delta_ticks);
        if delta_tracks != 0 {
            self.edit.selected.offset_tracks(delta_tracks);
        }

        model.rebuild_dirty();
        self.data.midi_version = self.data.midi_version.wrapping_add(1);

        if sub_actions.is_empty() {
            None
        } else if sub_actions.len() == 1 {
            sub_actions.into_iter().next()
        } else {
            Some(UndoAction::Composite(sub_actions))
        }
    }

    /// Apply a pencil-tool drag operation (move or resize a single note).
    ///
    /// Returns an `UndoAction` if the note was modified. The caller is
    /// responsible for pushing it to the history stack, marking the view
    /// dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn pencil_drag_note(&mut self, drag: &PencilNoteDrag) -> Option<UndoAction> {
        match drag {
            PencilNoteDrag::Move { track, start_tick, key, delta_ticks, delta_keys } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k].iter().find(|n| {
                    n.track == *track && n.start_tick == *start_tick
                })?;
                let orig_note = *note;
                let new_key = ((*key as i32) + delta_keys).clamp(0, 127) as u8;
                let new_tick = (orig_note.start_tick as i64 + delta_ticks).max(0) as u32;

                if *delta_ticks != 0 || *delta_keys != 0 {
                    let model = Arc::make_mut(&mut self.data.model);
                    // Remove original from old key bucket
                    let ok = *key as usize;
                    Arc::make_mut(&mut model.notes[ok]).retain(|n| {
                        !(n.track == *track && n.start_tick == orig_note.start_tick && n.dup_index == orig_note.dup_index)
                    });
                    model.mark_dirty(*key);
                    // Insert moved note at new key bucket
                    let length = orig_note.end_tick - orig_note.start_tick;
                    let moved = yinhe_types::Note {
                        start_tick: new_tick,
                        end_tick: new_tick + length,
                        velocity: orig_note.velocity,
                        dup_index: 0,
                        track: *track,
                    };
                    let nk = new_key as usize;
                    let insert_pos = model.notes[nk].partition_point(|n| n.start_tick < moved.start_tick);
                    Arc::make_mut(&mut model.notes[nk]).insert(insert_pos, moved);
                    model.mark_dirty(new_key);
                    model.rebuild_dirty();
                    self.data.midi_version = self.data.midi_version.wrapping_add(1);
                    return Some(UndoAction::Notes(NoteDelta {
                        before: vec![(orig_note, *key)],
                        after: vec![(moved, new_key)],
                    }));
                }
                None
            }
            PencilNoteDrag::ResizeRight { track, start_tick, key, new_end_tick } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k].iter().find(|n| {
                    n.track == *track && n.start_tick == *start_tick
                })?;
                if *new_end_tick != note.end_tick {
                    let before = *note;
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k]).iter_mut().find(|n| {
                        n.track == *track && n.start_tick == *start_tick
                    }) {
                        n.end_tick = (*new_end_tick).max(n.start_tick + 1);
                        let after = *n;
                        model.mark_dirty(*key);
                        model.rebuild_dirty();
                        self.data.midi_version = self.data.midi_version.wrapping_add(1);
                        return Some(UndoAction::Notes(NoteDelta {
                            before: vec![(before, *key)],
                            after: vec![(after, *key)],
                        }));
                    }
                }
                None
            }
            PencilNoteDrag::ResizeLeft { track, start_tick, key, new_start_tick } => {
                let model = &self.data.model;
                let k = *key as usize;
                let note = model.notes[k].iter().find(|n| {
                    n.track == *track && n.start_tick == *start_tick
                })?;
                if *new_start_tick != note.start_tick {
                    let before = *note;
                    let model = Arc::make_mut(&mut self.data.model);
                    if let Some(n) = Arc::make_mut(&mut model.notes[k]).iter_mut().find(|n| {
                        n.track == *track && n.start_tick == *start_tick
                    }) {
                        n.start_tick = (*new_start_tick).min(n.end_tick - 1);
                        let after = *n;
                        model.mark_dirty(*key);
                        model.rebuild_dirty();
                        self.data.midi_version = self.data.midi_version.wrapping_add(1);
                        return Some(UndoAction::Notes(NoteDelta {
                            before: vec![(before, *key)],
                            after: vec![(after, *key)],
                        }));
                    }
                }
                None
            }
        }
    }

    /// Apply a batch of automation edits (add / move / cycle-shape).
    ///
    /// Returns a `Vec<UndoAction>` for all successfully applied edits.
    /// The caller is responsible for pushing them to the history stack,
    /// marking the view dirty, and sending `AudioCommand::ReloadNotes`.
    pub fn apply_automation_edits(&mut self, edits: Vec<AutomationEdit>) -> Vec<UndoAction> {
        let mut actions = Vec::new();
        for edit in edits {
            let action = match edit {
                AutomationEdit::Add { track_idx, target, tick, value, shape } => {
                    let event = yinhe_types::AutomationEvent { tick, value, shape };
                    match self.add_automation_event(track_idx as usize, target, event) {
                        Some((_, _, action)) => Some(action),
                        None => None,
                    }
                }
                AutomationEdit::Move { track_idx, lane_idx, old_tick, new_tick, new_value } => {
                    self.move_automation_event(track_idx as usize, lane_idx, old_tick, new_tick, new_value)
                }
                AutomationEdit::CycleShape { track_idx, lane_idx, tick } => {
                    // Step ↔ Curve{tension:0}
                    let track = self.data.model.tracks.get(track_idx as usize);
                    let lane = track.and_then(|t| t.automation_lanes.get(lane_idx));
                    let evt = lane.and_then(|l| l.events.iter().find(|e| e.tick == tick));
                    if let Some(evt) = evt {
                        let next = match evt.shape {
                            yinhe_types::SegmentShape::Step => yinhe_types::SegmentShape::Curve { tension: 0 },
                            yinhe_types::SegmentShape::Curve { .. } => yinhe_types::SegmentShape::Step,
                        };
                        self.set_automation_shape(track_idx as usize, lane_idx, tick, next)
                    } else {
                        None
                    }
                }
                AutomationEdit::Delete { track_idx, lane_idx, tick } => {
                    self.delete_automation_event(track_idx as usize, lane_idx, tick)
                }
            };
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }

    /// Insert a new MIDI track after `after_idx`. Returns UndoAction.
    /// The new track gets port 0, channel = first unused channel on port 0.
    pub fn add_track(&mut self, after_idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if after_idx >= num_tracks {
            return None;
        }
        // Don't allow adding after conductor if it would insert before conductor
        let insert_idx = after_idx + 1;

        // Find a free channel on port 0
        let used_channels: std::collections::HashSet<u8> = model.tracks.iter()
            .filter(|t| t.port == 0)
            .map(|t| t.channel)
            .collect();
        let channel = (0..16u8).find(|c| !used_channels.contains(c)).unwrap_or(0);

        let mut new_track = yinhe_core::TrackData::new(0, channel);
        new_track.name = format!("A{}", channel + 1);

        let tracks_before: Vec<std::sync::Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.insert(insert_idx, Arc::new(new_track));

        // Remap notes: track >= insert_idx gets +1
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| if i >= insert_idx { (i + 1) as u16 } else { i as u16 })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i == insert_idx { u16::MAX } else if i < insert_idx { i as u16 } else { (i - 1) as u16 })
            .collect();

        let tracks_after: Vec<std::sync::Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap to notes
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                note.track = note_remap[note.track as usize];
            }
        }

        model.rebuild();
        self.data.midi_version = self.data.midi_version.wrapping_add(1);

        // Update edit state
        let num_tracks = model.tracks.len();
        self.edit.track_visible.push(true);
        self.edit.track_pianoroll_visible.push(true);
        self.edit.track_overrides.push(crate::document::TrackOverride::default());
        self.edit.track_info_cache = self.data.track_info();
        self.edit.track_colors_cache = (0..num_tracks)
            .map(|i| crate::document::track_color(i, self.edit.conductor_track_idx))
            .collect();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(insert_idx as u16);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
        })
    }

    /// Remove the track at `idx`. Notes belonging to it are deleted.
    pub fn remove_track(&mut self, idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        if idx >= model.tracks.len() {
            return None;
        }
        // Don't remove conductor track
        if self.edit.conductor_track_idx == Some(idx as u16) {
            return None;
        }
        // Don't remove if only 2 tracks (conductor + 1)
        if model.tracks.len() <= 2 {
            return None;
        }

        let tracks_before: Vec<std::sync::Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.remove(idx);

        // Remap: track < idx stays, track == idx is deleted (u16::MAX), track > idx gets -1
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| if i == idx { u16::MAX } else if i > idx { (i - 1) as u16 } else { i as u16 })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i < idx { i as u16 } else { (i + 1) as u16 })
            .collect();

        let tracks_after: Vec<std::sync::Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap: delete notes on removed track, shift others
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            bucket.retain(|n| note_remap[n.track as usize] != u16::MAX);
            for note in bucket.iter_mut() {
                note.track = note_remap[note.track as usize];
            }
        }
        // Mark all buckets dirty since we may have removed notes from any
        for k in 0..128 {
            model.mark_dirty(k as u8);
        }
        model.rebuild();
        self.data.midi_version = self.data.midi_version.wrapping_add(1);

        // Update edit state
        self.edit.track_visible.remove(idx);
        self.edit.track_pianoroll_visible.remove(idx);
        self.edit.track_overrides.remove(idx);
        let num_tracks = model.tracks.len();
        self.edit.track_info_cache = self.data.track_info();
        self.edit.track_colors_cache = (0..num_tracks)
            .map(|i| crate::document::track_color(i, self.edit.conductor_track_idx))
            .collect();
        self.edit.track_selected.clear();
        // Select the track that took its place (or last track)
        let new_sel = idx.min(num_tracks - 1) as u16;
        self.edit.track_selected.insert(new_sel);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
        })
    }

    /// Move track at `from_idx` to `to_idx`. Other tracks shift to fill the gap.
    pub fn move_track(&mut self, from_idx: usize, to_idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if from_idx >= num_tracks || to_idx >= num_tracks || from_idx == to_idx {
            return None;
        }
        // Don't move conductor track
        if self.edit.conductor_track_idx == Some(from_idx as u16) ||
           self.edit.conductor_track_idx == Some(to_idx as u16) {
            return None;
        }

        let tracks_before: Vec<std::sync::Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.remove(from_idx);
        model.tracks.insert(to_idx, track);

        // Build remap table
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| {
                if i == from_idx { to_idx as u16 }
                else if from_idx < to_idx && i > from_idx && i <= to_idx { (i - 1) as u16 }
                else if from_idx > to_idx && i >= to_idx && i < from_idx { (i + 1) as u16 }
                else { i as u16 }
            })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| {
                if i == to_idx { from_idx as u16 }
                else if from_idx < to_idx && i >= from_idx && i < to_idx { (i + 1) as u16 }
                else if from_idx > to_idx && i > to_idx && i <= from_idx { (i - 1) as u16 }
                else { i as u16 }
            })
            .collect();

        let tracks_after: Vec<std::sync::Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap to notes
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                note.track = note_remap[note.track as usize];
            }
        }

        model.rebuild();
        self.data.midi_version = self.data.midi_version.wrapping_add(1);

        // Update edit state
        self.edit.track_info_cache = self.data.track_info();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(to_idx as u16);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
        })
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
    if !first.automation_lanes.is_empty() || !first.program_change.is_empty() {
        return None;
    }
    Some(0)
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
