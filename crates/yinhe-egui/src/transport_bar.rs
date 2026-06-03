use eframe::egui;

use crate::document::Document;
use crate::file_loader::FileLoader;

pub fn show(
    ui: &mut egui::Ui,
    file_loader: &mut FileLoader,
    toggle_play: &mut bool,
    stop_play: &mut bool,
    doc: Option<&Document>,
) {
    let has_active = doc.is_some();

    egui::Panel::top("transport_bar")
        .frame(egui::Frame {
            fill: egui::Color32::from_rgb(25, 25, 28),
            inner_margin: egui::Margin::symmetric(8, 8),
            stroke: egui::Stroke::NONE,
            ..Default::default()
        })
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let open_btn =
                    ui.add_enabled(!file_loader.is_loading(), egui::Button::new("Open MIDI"));
                if open_btn.clicked() {
                    file_loader.pick_midi_file();
                }

                if has_active {
                    ui.separator();
                    let is_playing = doc.map(|d| d.playback.is_playing()).unwrap_or(false);
                    let play_label = if is_playing { "Pause" } else { "Play" };
                    if ui.button(play_label).clicked() {
                        *toggle_play = true;
                    }
                    if ui.button("Stop").clicked() {
                        *stop_play = true;
                    }
                }

                ui.separator();

                if let Some(doc) = doc {
                    ui.label(egui::RichText::new(&doc.file_name).strong());
                }

                if let Some(doc) = doc {
                    ui.separator();
                    ui.label(format!("Notes: {}", doc.midi.note_count));
                    ui.separator();
                    ui.label(format!("Tracks: {}", doc.midi.track_ports.len()));
                    ui.separator();
                    ui.label(format!("TPB: {}", doc.midi.ticks_per_beat));
                }
            });
        });
}
