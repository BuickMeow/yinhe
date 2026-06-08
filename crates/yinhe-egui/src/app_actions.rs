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
                // Tear down old audio so it stops immediately
                self.teardown_audio();
            }
            transport_bar::FileAction::Open => {
                self.file_loader.pick_midi_file();
            }
            transport_bar::FileAction::CloseDocument => {
                if let Some(idx) = self.active_doc {
                    self.close_document(idx);
                }
            }
            transport_bar::FileAction::Exit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            transport_bar::FileAction::Settings => {
                self.audio_settings.show_settings = true;
            }
            _ => {
                // Save, SaveAs, ExportAudio, ExportMidi
                // not yet implemented
            }
        }
    }
}
