use std::sync::mpsc;
use std::sync::Arc;

use eframe::egui;

use crate::app::App;
use yinhe_editor_core::document::Document;
use crate::chrome::transport_bar;

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
        self.with_undo("Delete notes", |doc| doc.delete_selected());
    }

    /// Duplicate all selected notes (Ctrl+D / Cmd+D).
    /// New notes are placed after the original selection, offset by the selection duration.
    pub(crate) fn duplicate_selected_notes(&mut self) {
        self.with_undo("Duplicate notes", |doc| {
            let offset = doc.duplicate_selected();
            doc.edit.sel_rect.pending_delta = offset.map(|o| (o as i64, 0));
            offset.is_some()
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
            let st = doc.transpose_selected(semitones);
            doc.edit.sel_rect.pending_delta = st.map(|s| (0, s as i32));
            st.is_some()
        });
    }

    /// Add a single note to the given track and record an undo entry.
    pub(crate) fn add_note_with_undo(&mut self, track_idx: u16, note: yinhe_core::NoteEvent) {
        self.with_undo("Add note", |doc| doc.add_note(track_idx, note));
    }

    /// Capture an `UndoSnapshot` of the active document's persistent state.
    /// Returns `None` if no document is active.
    pub(crate) fn capture_snapshot(&self, label: &'static str) -> Option<yinhe_editor_core::history::UndoSnapshot> {
        let idx = self.active_doc?;
        let doc = self.documents.get(idx)?;
        Some(doc.snapshot_with_selection(label))
    }

    /// Run an edit closure, recording an undo entry beforehand and notifying
    /// audio afterwards.
    ///
    /// The closure receives `&mut Document` and should return `true` if it
    /// actually changed anything; on `false` no snapshot is pushed and audio
    /// is not notified.
    pub(crate) fn with_undo<F>(&mut self, label: &'static str, f: F)
    where
        F: FnOnce(&mut Document) -> bool,
    {
        let Some(idx) = self.active_doc else { return };
        let snapshot = self.documents[idx].snapshot_with_selection(label);
        let changed = f(&mut self.documents[idx]);
        if !changed {
            return;
        }
        let doc = &mut self.documents[idx];
        doc.history.push(snapshot);
        doc.data.bump_version();
        self.pianoroll_view.base.dirty = true;
        if let Some(ref audio) = self.audio {
            let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes { model: doc.data.model.clone() });
        }
    }

    /// Restore the previous state on the active document's history stack.
    pub(crate) fn undo(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let current = self.documents[idx].snapshot_with_selection("current");
        let restored = self.documents[idx].history.undo(current);
        if let Some(snap) = restored {
            self.apply_snapshot(idx, snap);
        }
    }

    /// Re-apply the most recently undone state on the active document.
    pub(crate) fn redo(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let current = self.documents[idx].snapshot_with_selection("current");
        let restored = self.documents[idx].history.redo(current);
        if let Some(snap) = restored {
            self.apply_snapshot(idx, snap);
        }
    }

    /// Apply a snapshot to the document at `idx`: restore persistent fields,
    /// rebuild caches, clear selection, and notify audio.
    fn apply_snapshot(&mut self, idx: usize, snap: yinhe_editor_core::history::UndoSnapshot) {
        let doc = &mut self.documents[idx];
        doc.apply_undo_snapshot(snap);
        self.pianoroll_view.base.dirty = true;
        if let Some(ref audio) = self.audio {
            let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes { model: doc.data.model.clone() });
        }
    }
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
        let doc = &mut self.documents[idx];
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();

        // Sync SF state into project_file
        doc.data.project_file.soundfont_project_mode =
            !self.audio_settings.global_sf_config.global_enabled;
        doc.data.project_file.soundfont_overrides = doc
            .edit
            .project_sf
            .overrides
            .iter()
            .map(|(port, entries)| yinhe_yin::SfPortOverride {
                port: *port,
                entries: entries
                    .iter()
                    .map(|e| yinhe_yin::SfEntryJson {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect(),
            })
            .collect();

        let model = doc.data.model.clone();
        let project_file = doc.data.project_file.clone();
        let mapping_file = doc.data.mapping_file.clone();
        let path_for_thread = path.clone();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Err(e) = yinhe_yin::save_yin_with_files(
                &model,
                &path_for_thread,
                &project_file,
                &mapping_file,
            ) {
                tracing::error!("Failed to save project: {}", e);
            }
            let _ = tx.send(());
        });

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
                match yinhe_mid2::write_to_bytes(&doc.data.model) {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&path_str, &bytes) {
                            tracing::error!("Failed to export MIDI: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to export MIDI: {}", e);
                    }
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
        let model = doc.data.model.clone();
        let sr = if self.export_sample_rate > 0 {
            self.export_sample_rate
        } else {
            self.audio_settings.sample_rate
        };
        let port_sf = self.resolve_sf_config(doc);
        let has_solo = doc.edit.track_overrides.iter().any(|t| t.soloed);
        let skip: Vec<bool> = doc
            .edit
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
                model,
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
