use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::Arc;

use eframe::egui;

use crate::app::App;
use crate::document::Document;
use crate::widgets::transport_bar;

/// Actions detected from keyboard input in the current frame.
#[derive(Default)]
pub(crate) struct KeyboardActions {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub delete_selected: bool,
    pub duplicate_selected: bool,
    pub transpose_up: bool,
    pub transpose_down: bool,
    pub undo: bool,
    pub redo: bool,
}

impl App {
    /// Handle keyboard shortcuts.
    /// Returns a `KeyboardActions` struct describing which actions were triggered.
    pub(crate) fn handle_keyboard_shortcuts(&self, ui: &egui::Ui) -> KeyboardActions {
        let mut actions = KeyboardActions::default();

        let is_playing_any = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);

        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                if is_playing_any {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            if i.key_pressed(egui::Key::Escape) {
                actions.stop_play = true;
            }

            // Delete / Backspace — delete selected notes
            if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
                actions.delete_selected = true;
            }

            // Ctrl+D / Cmd+D — duplicate selected notes
            if (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(egui::Key::D) {
                actions.duplicate_selected = true;
            }

            // Shift+↑ / Shift+↓ — transpose octave
            if i.modifiers.shift {
                if i.key_pressed(egui::Key::ArrowUp) {
                    actions.transpose_up = true;
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    actions.transpose_down = true;
                }
            }

            // Cmd/Ctrl+Z — undo;  Cmd/Ctrl+Shift+Z or Cmd/Ctrl+Y — redo.
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::Z) {
                if i.modifiers.shift {
                    actions.redo = true;
                } else {
                    actions.undo = true;
                }
            }
            if cmd && i.key_pressed(egui::Key::Y) {
                actions.redo = true;
            }
        });

        actions
    }

    /// Delete all selected notes from the active document.
    pub(crate) fn delete_selected_notes(&mut self) {
        self.with_undo("Delete notes", |doc| {
            if doc.selected.is_empty() {
                return false;
            }
            let changed = {
                let midi = Arc::make_mut(&mut doc.midi);
                for &(track, start_tick, key) in &doc.selected {
                    let notes = &mut midi.key_notes[key as usize];
                    notes.retain(|n| !(n.track == track && n.start_tick == start_tick));
                }
                doc.selected.clear();
                rebuild_midi_metadata(midi);
                true
            };
            doc.midi_version = doc.midi_version.wrapping_add(1);
            changed
        });
    }

    /// Duplicate all selected notes (Ctrl+D / Cmd+D).
    /// New notes are placed after the original selection, offset by the selection duration.
    pub(crate) fn duplicate_selected_notes(&mut self) {
        self.with_undo("Duplicate notes", |doc| {
            if doc.selected.is_empty() {
                return false;
            }

            let changed = {
                let midi = Arc::make_mut(&mut doc.midi);

                // Collect full note data for each selected entry
                let mut selected_data: Vec<(yinhe_types::Note, u8)> = Vec::new();
                for &(track, start_tick, key) in &doc.selected {
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

                // Calculate offset: duration of the selection (max_end - min_start)
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

                doc.selected = new_selected;
                rebuild_midi_metadata(midi);
                true
            };
            doc.midi_version = doc.midi_version.wrapping_add(1);
            changed
        });
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave, -12 for down).
    pub(crate) fn transpose_selected_notes(&mut self, semitones: i8) {
        let label = if semitones >= 0 {
            "Transpose up"
        } else {
            "Transpose down"
        };
        self.with_undo(label, |doc| {
            if doc.selected.is_empty() {
                return false;
            }

            let changed = {
                let midi = Arc::make_mut(&mut doc.midi);

                // Remove selected notes from their current keys and collect their data
                let mut moved_data: Vec<(yinhe_types::Note, u8)> = Vec::new();
                for &(track, start_tick, key) in &doc.selected {
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

                // Re-insert at new keys
                let mut new_selected = HashSet::new();
                for (note, old_key) in &moved_data {
                    let new_key = ((*old_key as i16) + (semitones as i16)).clamp(0, 127) as u8;
                    let notes = &mut midi.key_notes[new_key as usize];
                    let insert_pos = notes.partition_point(|n| n.start_tick < note.start_tick);
                    notes.insert(insert_pos, note.clone());
                    new_selected.insert((note.track, note.start_tick, new_key));
                }

                doc.selected = new_selected;
                rebuild_midi_metadata(midi);
                true
            };
            doc.midi_version = doc.midi_version.wrapping_add(1);
            changed
        });
    }

    /// Capture a `Snapshot` of the active document's persistent state.
    /// Returns `None` if no document is active.
    pub(crate) fn capture_snapshot(&self, label: &'static str) -> Option<crate::history::Snapshot> {
        let idx = self.active_doc?;
        let doc = self.documents.get(idx)?;
        Some(crate::history::Snapshot::capture(doc, label))
    }

    /// Run an edit closure, recording an undo entry beforehand and notifying
    /// audio afterwards.
    ///
    /// The closure receives `&mut Document` and should return `true` if it
    /// actually changed anything; on `false` no snapshot is pushed and audio
    /// is not notified.
    pub(crate) fn with_undo<F>(&mut self, label: &'static str, f: F)
    where
        F: FnOnce(&mut crate::document::Document) -> bool,
    {
        let Some(idx) = self.active_doc else { return };
        // Build snapshot before mutation. Cheap: Arc::clone + small clones.
        let snapshot = crate::history::Snapshot::capture(&self.documents[idx], label);
        let changed = {
            let doc = &mut self.documents[idx];
            f(doc)
        };
        if !changed {
            return;
        }
        let midi_clone = {
            let doc = &mut self.documents[idx];
            doc.history.push(snapshot);
            self.pianoroll_view.base.dirty = true;
            Arc::clone(&doc.midi)
        };
        if let Some(ref audio) = self.audio {
            let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes {
                midi: midi_clone,
            });
        }
    }

    /// Restore the previous state on the active document's history stack.
    pub(crate) fn undo(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let current = crate::history::Snapshot::capture(&self.documents[idx], "current");
        let restored = self.documents[idx].history.undo(current);
        if let Some(snap) = restored {
            self.apply_snapshot(idx, snap);
        }
    }

    /// Re-apply the most recently undone state on the active document.
    pub(crate) fn redo(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let current = crate::history::Snapshot::capture(&self.documents[idx], "current");
        let restored = self.documents[idx].history.redo(current);
        if let Some(snap) = restored {
            self.apply_snapshot(idx, snap);
        }
    }

    /// Apply a snapshot to the document at `idx`: restore persistent fields,
    /// rebuild caches, clear selection, and notify audio.
    fn apply_snapshot(&mut self, idx: usize, snap: crate::history::Snapshot) {
        let midi_clone = {
            let doc = &mut self.documents[idx];
            snap.restore(doc);
            // Rebuild track_info_cache from the restored midi (port, channel,
            // note_count, name — all may have changed).
            doc.track_info_cache = doc.midi.track_info();
            // Sync track_names from cache (track_info() reads midi.track_names).
            for (i, ti) in doc.track_info_cache.iter().enumerate() {
                if i < doc.track_names.len() {
                    doc.track_names[i] = ti.name.clone();
                }
            }
            // Rebuild pc_map_cache from restored control events.
            let mut pc_map = std::collections::HashMap::new();
            for ev in &doc.midi.control_events {
                if let yinhe_midi::MidiControlEvent::ProgramChange {
                    program, track, ..
                } = ev
                {
                    let ch = doc.midi.track_channels.get(*track as usize).copied().unwrap_or(0);
                    pc_map.entry(ch).or_insert(*program);
                }
            }
            doc.pc_map_cache = pc_map;
            // Clear selection: restored notes' (start_tick, key) may no longer
            // match what the user had selected.
            doc.selected.clear();
            self.pianoroll_view.base.dirty = true;
            Arc::clone(&doc.midi)
        };
        if let Some(ref audio) = self.audio {
            let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes {
                midi: midi_clone,
            });
        }
    }
}

/// Rebuild computed metadata on a MidiFile after note mutations.
pub(crate) fn rebuild_midi_metadata(midi: &mut yinhe_midi::MidiFile) {
    midi.note_count = 0;
    let mut max_tick = 0u64;
    for notes in &midi.key_notes {
        midi.note_count += notes.len() as u64;
        for note in notes {
            max_tick = max_tick.max(note.end_tick as u64);
        }
    }
    midi.tick_length = max_tick;
    midi.scan_index = Some(yinhe_types::NoteScanIndex::build(&midi.key_notes, max_tick));
    midi.automation_lanes = yinhe_midi::build_automation_lanes(&midi.control_events, &midi.key_notes, &midi.track_channels);
}

impl App {
    /// Handle file menu actions from the transport bar.
    pub(crate) fn handle_file_action(
        &mut self,
        action: transport_bar::FileAction,
        ctx: &egui::Context,
    ) {
        match action {
            transport_bar::FileAction::NewProject => {
                self.documents.push(Document::empty());
                self.active_doc = Some(self.documents.len() - 1);
                self.teardown_audio();
            }
            transport_bar::FileAction::Open => {
                self.file_loader.pick_file(self.audio_settings.midi_import_encoding);
            }
            transport_bar::FileAction::Save => {
                if let Some(idx) = self.active_doc {
                    let path = self.documents[idx].file_path.clone();
                    if let Some(path) = path {
                        self.save_project_async(idx, path);
                    } else {
                        self.save_as_dialog();
                    }
                }
            }
            transport_bar::FileAction::SaveAs => {
                self.save_as_dialog();
            }
            transport_bar::FileAction::CloseDocument => {
                if let Some(idx) = self.active_doc {
                    self.close_document(idx);
                }
            }
            transport_bar::FileAction::ExportMidi => {
                self.export_midi_dialog();
            }
            transport_bar::FileAction::ExportAudio => {
                self.export_audio_dialog();
            }
            transport_bar::FileAction::Exit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            transport_bar::FileAction::Settings => {
                self.audio_settings.show_settings = true;
            }
        }
    }

    /// Spawn a background thread to save the project.
    fn save_project_async(&mut self, idx: usize, path: String) {
        let doc = &self.documents[idx];
        let midi = doc.midi.clone();
        let track_names = doc.track_names.clone();
        let project_name = doc.project_name.clone();
        let project_artist = doc.project_artist.clone();
        let project_description = doc.project_description.clone();
        let project_ppq = doc.project_ppq;
        let compression_level = doc.archive.as_ref().map(|a| a.compression_level).unwrap_or(0);
        let project_sf = doc.project_sf.clone();
        let global_enabled = self.audio_settings.global_sf_config.global_enabled;
        let path_for_thread = path.clone();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let archive = crate::project_io::build_archive_from(
                &midi,
                &track_names,
                &project_name,
                &project_artist,
                project_ppq,
                compression_level,
                &project_description,
                &project_sf,
                global_enabled,
            );
            if let Err(e) = archive.write_to(&path_for_thread) {
                tracing::error!("Failed to save project: {}", e);
            }
            let _ = tx.send(());
        });

        // Update metadata immediately (not waiting for thread)
        if let Some(doc) = self.documents.get_mut(idx) {
            doc.file_path = Some(path);
        }
        self.save_rx = Some(rx);
    }

    fn save_as_dialog(&mut self) {
        let default_name = if let Some(idx) = self.active_doc {
            format!("{}.yin", self.documents[idx].file_name)
        } else {
            "Untitled.yin".to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Yinhe Project", &["yin"])
            .set_file_name(&default_name)
            .save_file()
        {
            let mut path_str = path.to_string_lossy().to_string();
            // Ensure .yin extension
            if !path_str.ends_with(".yin") {
                path_str.push_str(".yin");
            }
            if let Some(idx) = self.active_doc {
                let path2 = path_str.clone();
                self.save_project_async(idx, path2);
                // Update file_name
                if let Some(doc) = self.documents.get_mut(idx) {
                    doc.file_name = path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                }
            }
        }
    }

    fn export_midi_dialog(&mut self) {
        let default_name = if let Some(idx) = self.active_doc {
            format!("{}.mid", self.documents[idx].file_name)
        } else {
            "export.mid".to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .set_file_name(&default_name)
            .save_file()
        {
            let path_str = path.to_string_lossy().to_string();
            if let Some(idx) = self.active_doc {
                let doc = &self.documents[idx];
                if let Err(e) = crate::project_io::export_midi(doc, &path_str) {
                    tracing::error!("Failed to export MIDI: {}", e);
                }
            }
        }
    }

    fn export_audio_dialog(&mut self) {
        if self.active_doc.is_none() {
            return;
        }

        if self.export_rx.is_some() {
            return; // already exporting
        }

        // Close settings window to avoid ComboBox overlay conflict
        self.audio_settings.show_settings = false;

        // Show export settings dialog first
        self.show_export_bit_depth = true;
    }

    /// Called after the bit-depth dialog is confirmed.
    /// Opens the file-save dialog and starts the export.
    pub(crate) fn start_export(&mut self) {
        let idx = match self.active_doc {
            Some(idx) => idx,
            None => return,
        };

        let doc = &self.documents[idx];
        let default_name = format!("{}.wav", doc.file_name);

        let path = match rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_file_name(&default_name)
            .save_file()
        {
            Some(p) => p,
            None => return,
        };

        let mut path_str = path.to_string_lossy().to_string();
        if !path_str.ends_with(".wav") {
            path_str.push_str(".wav");
        }

        // Collect render inputs
        let midi = doc.midi.clone();
        let sr = if self.export_sample_rate > 0 {
            self.export_sample_rate
        } else {
            self.audio_settings.sample_rate
        };
        let port_sf = self.resolve_sf_config(doc);
        let has_solo = doc.track_overrides.iter().any(|t| t.soloed);
        let skip: Vec<bool> = doc
            .track_overrides
            .iter()
            .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
            .collect();
        let bit_depth = self.export_bit_depth;
        let layer_count = if self.export_layer_count == 0 {
            None
        } else {
            Some(self.export_layer_count as usize)
        };
        let export_progress = self.export_progress.clone();

        // Reset progress state
        {
            let mut p = export_progress.lock().unwrap();
            p.visible = true;
            p.progress = 0.0;
            p.status = "准备中…".into();
        }

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = yinhe_audio::export::export_wav(
                midi,
                sr,
                &port_sf,
                &skip,
                std::path::Path::new(&path_str),
                bit_depth,
                layer_count,
                |pct, msg| {
                    if let Ok(mut p) = export_progress.lock() {
                        p.progress = pct;
                        if !msg.is_empty() {
                            p.status = msg.to_string();
                        }
                    }
                },
            );
            // Mark done
            if let Ok(mut p) = export_progress.lock() {
                p.visible = false;
            }
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });

        self.export_rx = Some(rx);
    }
}

