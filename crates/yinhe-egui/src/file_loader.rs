use std::sync::mpsc;

use crate::loading::{MidiLoadEvent, MidiLoader};

/// Result of polling the async MIDI loader.
pub(crate) enum MidiLoadResult {
    Loaded {
        path: String,
        midi: yinhe_midi::MidiFile,
    },
    NotReady,
}

/// Manages async MIDI file loading (file dialog + background thread).
pub(crate) struct FileLoader {
    midi_loader: Option<MidiLoader>,
}

impl FileLoader {
    pub fn new() -> Self {
        Self { midi_loader: None }
    }

    pub fn is_loading(&self) -> bool {
        self.midi_loader.is_some()
    }

    /// Show file dialog and start loading MIDI in a background thread.
    pub fn pick_midi_file(&mut self) {
        if self.is_loading() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .pick_file()
        {
            let (tx, rx) = mpsc::channel();
            let path_str = path.to_string_lossy().to_string();
            let path_for_thread = path_str.clone();

            std::thread::spawn(move || {
                let result = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
                    let data = match std::fs::read(&path_for_thread) {
                        Ok(d) => d,
                        Err(e) => {
                            return Err(yinhe_midi::MidiError::Io(e));
                        }
                    };
                    yinhe_midi::MidiFile::load_from_bytes_with_progress(
                        &data,
                        |progress| {
                            let _ = tx.send(MidiLoadEvent::Progress(progress));
                        },
                    )
                });
                let _ = tx.send(MidiLoadEvent::Complete(Box::new(result)));
            });

            self.midi_loader = Some(MidiLoader {
                path: path_str,
                rx,
                current_progress: None,
            });
        }
    }

    /// Poll the background thread for loading progress/completion.
    pub fn poll_midi_loading(&mut self) -> MidiLoadResult {
        if let Some(mut loader) = self.midi_loader.take() {
            while let Ok(event) = loader.rx.try_recv() {
                match event {
                    MidiLoadEvent::Progress(progress) => {
                        loader.current_progress = Some(progress);
                    }
                    MidiLoadEvent::Complete(result) => {
                        match *result {
                            Ok(midi) => {
                                let path = loader.path.clone();
                                return MidiLoadResult::Loaded { path, midi };
                            }
                            Err(e) => {
                                tracing::error!("Failed to load MIDI: {}", e);
                            }
                        }
                        return MidiLoadResult::NotReady;
                    }
                }
            }
            self.midi_loader = Some(loader);
        }
        MidiLoadResult::NotReady
    }

    /// Draw a dark overlay + centered window with loading progress.
    pub fn show_midi_loading_overlay(&self, ui: &mut eframe::egui::Ui) {
        if let Some(loader) = &self.midi_loader {
            let screen_rect = ui.ctx().content_rect();
            ui.ctx()
                .layer_painter(eframe::egui::LayerId::new(
                    eframe::egui::Order::Foreground,
                    "midi_loading_overlay".into(),
                ))
                .rect_filled(
                    screen_rect,
                    0.0,
                    eframe::egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
                );

            eframe::egui::Window::new("Loading MIDI")
                .order(eframe::egui::Order::Tooltip)
                .collapsible(false)
                .resizable(false)
                .movable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, eframe::egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    if let Some(progress) = &loader.current_progress {
                        ui.label(format!(
                            "Parsing track {} / {}...",
                            progress.current_track, progress.total_tracks
                        ));
                        let ratio =
                            progress.current_track as f32 / progress.total_tracks.max(1) as f32;
                        ui.add(eframe::egui::ProgressBar::new(ratio).show_percentage());
                    } else {
                        ui.label("Reading MIDI file...");
                        ui.add(eframe::egui::Spinner::new());
                    }
                });
        }
    }
}
