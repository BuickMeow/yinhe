use std::sync::mpsc;

use eframe::egui;

use crate::app::App;
use crate::document::Document;
use crate::widgets::transport_bar;

impl App {
    /// Handle keyboard shortcuts (Space for play/pause, Escape for stop).
    /// Returns (toggle_play, pause_return, stop_play).
    pub(crate) fn handle_keyboard_shortcuts(&self, ui: &egui::Ui) -> (bool, bool, bool) {
        let mut toggle_play = false;
        let mut pause_return = false;
        let mut stop_play = false;

        let is_playing_any = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);

        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                if is_playing_any {
                    pause_return = true;
                } else {
                    toggle_play = true;
                }
            }
            if i.key_pressed(egui::Key::Escape) {
                stop_play = true;
            }
        });

        (toggle_play, pause_return, stop_play)
    }

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
