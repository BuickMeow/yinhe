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
        });

        actions
    }

    /// Delete all selected notes from the active document.
    pub(crate) fn delete_selected_notes(&mut self) {
        let Some(idx) = self.active_doc else { return };

        let midi_clone = {
            let doc = &mut self.documents[idx];
            if doc.selected.is_empty() {
                return;
            }

            let midi = Arc::make_mut(&mut doc.midi);
            for &(track, start_tick, key) in &doc.selected {
                let notes = &mut midi.key_notes[key as usize];
                notes.retain(|n| !(n.track == track && n.start_tick == start_tick));
            }
            doc.selected.clear();
            rebuild_midi_metadata(midi);
            self.pianoroll_view.base.dirty = true;
            Arc::clone(&doc.midi)
        };

        if let Some(ref audio) = self.audio {
            let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes {
                midi: midi_clone,
            });
        }
    }

    /// Duplicate all selected notes (Ctrl+D / Cmd+D).
    /// New notes are placed after the original selection, offset by the selection duration.
    pub(crate) fn duplicate_selected_notes(&mut self) {
        let Some(idx) = self.active_doc else { return };

        let midi_clone = {
            let doc = &mut self.documents[idx];
            if doc.selected.is_empty() {
                return;
            }

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
                return;
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
            self.pianoroll_view.base.dirty = true;
            Arc::clone(&doc.midi)
        };

        if let Some(ref audio) = self.audio {
            let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes {
                midi: midi_clone,
            });
        }
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave, -12 for down).
    pub(crate) fn transpose_selected_notes(&mut self, semitones: i8) {
        let Some(idx) = self.active_doc else { return };

        let midi_clone = {
            let doc = &mut self.documents[idx];
            if doc.selected.is_empty() {
                return;
            }

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
                return;
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
fn rebuild_midi_metadata(midi: &mut yinhe_midi::MidiFile) {
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
                self.file_loader.pick_file();
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
                // not yet implemented
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
}
