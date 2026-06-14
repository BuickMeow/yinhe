use std::sync::mpsc;

use yinhe_midi::LoadProgress;

use crate::progress::{self, SharedProgress, StageStatus};

/// Events sent from the background loading thread to the UI thread.
pub(crate) enum MidiLoadEvent {
    Progress(LoadProgress),
    Complete(Box<Result<yinhe_midi::MidiFile, yinhe_midi::MidiError>>),
}

/// Events for .yin project loading.
pub(crate) enum YinLoadEvent {
    Complete(Result<(yinhe_midi::MidiFile, String), String>),
}

/// Tracks the state of an in-flight MIDI load operation.
pub(crate) struct MidiLoader {
    pub path: String,
    pub rx: mpsc::Receiver<MidiLoadEvent>,
    pub current_progress: Option<LoadProgress>,
}

/// Tracks the state of an in-flight .yin load operation.
pub(crate) struct YinLoader {
    pub path: String,
    pub rx: mpsc::Receiver<YinLoadEvent>,
}

/// Result of polling the async loader.
pub(crate) enum LoadResult {
    MidiLoaded {
        path: String,
        midi: yinhe_midi::MidiFile,
    },
    YinLoaded {
        path: String,
        midi: yinhe_midi::MidiFile,
        file_name: String,
    },
    NotReady,
}

/// Manages async file loading (file dialog + background thread).
pub(crate) struct FileLoader {
    midi_loader: Option<MidiLoader>,
    yin_loader: Option<YinLoader>,
    /// Shared multi-stage progress state.
    load_progress: SharedProgress,
}

impl FileLoader {
    pub fn new(load_progress: SharedProgress) -> Self {
        Self {
            midi_loader: None,
            yin_loader: None,
            load_progress,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.midi_loader.is_some() || self.yin_loader.is_some()
    }

    /// Show file dialog and start loading in a background thread.
    pub fn pick_file(&mut self) {
        if self.is_loading() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("All supported", &["mid", "midi", "yin"])
            .add_filter("MIDI", &["mid", "midi"])
            .add_filter("Yinhe Project", &["yin"])
            .pick_file()
        {
            let path_str = path.to_string_lossy().to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();

            // Reset and show progress overlay
            progress::set_visible(&self.load_progress, true);

            match ext.as_str() {
                "yin" => {
                    let (tx, rx) = mpsc::channel();
                    let path_for_thread = path_str.clone();
                    let progress = self.load_progress.clone();
                    std::thread::spawn(move || {
                        progress::set_stage(&progress, 0, StageStatus::Done);
                        progress::set_stage(&progress, 1, StageStatus::Active);
                        let result = crate::project_io::load_project(&path_for_thread);
                        match result {
                            Ok((midi, file_name)) => {
                                progress::set_stage(&progress, 1, StageStatus::Done);
                                let _ = tx.send(YinLoadEvent::Complete(Ok((midi, file_name))));
                            }
                            Err(e) => {
                                let _ = tx.send(YinLoadEvent::Complete(Err(e.to_string())));
                            }
                        }
                    });
                    self.yin_loader = Some(YinLoader {
                        path: path_str,
                        rx,
                    });
                }
                _ => {
                    // MIDI file
                    let (tx, rx) = mpsc::channel();
                    let path_for_thread = path_str.clone();
                    let progress = self.load_progress.clone();
                    std::thread::spawn(move || {
                        progress::set_stage(&progress, 0, StageStatus::Active);
                        let result = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
                            let data = match std::fs::read(&path_for_thread) {
                                Ok(d) => d,
                                Err(e) => return Err(yinhe_midi::MidiError::Io(e)),
                            };
                            yinhe_midi::MidiFile::load_from_bytes_with_progress_owned(data, |p| {
                                let _ = tx.send(MidiLoadEvent::Progress(p));
                            })
                        });
                        progress::set_stage(&progress, 0, StageStatus::Done);
                        // Stage 1: archive conversion in background thread
                        if let Ok(ref midi) = result {
                            progress::set_stage(&progress, 1, StageStatus::Active);
                            let p = progress.clone();
                            let cb = |pct: f32, detail: &str| {
                                progress::set_stage_progress(
                                    &p, 1, pct, detail.to_string(),
                                );
                            };
                            let _ = crate::project_io::midi_to_archive_with_names(
                                midi,
                                &midi.track_names,
                                Some(&cb),
                            );
                            progress::set_stage(&progress, 1, StageStatus::Done);
                        }
                        let _ = tx.send(MidiLoadEvent::Complete(Box::new(result)));
                    });
                    self.midi_loader = Some(MidiLoader {
                        path: path_str,
                        rx,
                        current_progress: None,
                    });
                }
            }
        }
    }

    /// Poll the background thread for loading progress/completion.
    pub fn poll_loading(&mut self) -> LoadResult {
        // Poll MIDI loader
        if let Some(mut loader) = self.midi_loader.take() {
            while let Ok(event) = loader.rx.try_recv() {
                match event {
                    MidiLoadEvent::Progress(p) => {
                        loader.current_progress = Some(p);
                        let ratio = p.current_track as f32 / p.total_tracks.max(1) as f32;
                        progress::set_stage_progress(
                            &self.load_progress,
                            0,
                            ratio,
                            format!("{}/{}", p.current_track, p.total_tracks),
                        );
                    }
                    MidiLoadEvent::Complete(result) => {
                        match *result {
                            Ok(midi) => {
                                let path = loader.path.clone();
                                progress::set_visible(&self.load_progress, false);
                                return LoadResult::MidiLoaded { path, midi };
                            }
                            Err(e) => {
                                tracing::error!("Failed to load MIDI: {}", e);
                            }
                        }
                        return LoadResult::NotReady;
                    }
                }
            }
            self.midi_loader = Some(loader);
        }

        // Poll Yin loader
        if let Some(loader) = self.yin_loader.take() {
            if let Ok(event) = loader.rx.try_recv() {
                match event {
                    YinLoadEvent::Complete(result) => match result {
                        Ok((midi, file_name)) => {
                            let path = loader.path.clone();
                            progress::set_visible(&self.load_progress, false);
                            return LoadResult::YinLoaded {
                                path,
                                midi,
                                file_name,
                            };
                        }
                        Err(e) => {
                            tracing::error!("Failed to load .yin project: {}", e);
                        }
                    },
                }
                return LoadResult::NotReady;
            }
            self.yin_loader = Some(loader);
        }

        LoadResult::NotReady
    }

    /// Draw a dark overlay + centered window with multi-stage loading progress.
    pub fn show_loading_overlay(&self, ui: &mut eframe::egui::Ui) {
        if !self.is_loading() {
            return;
        }

        let progress = match self.load_progress.lock() {
            Ok(p) => p.clone(),
            Err(_) => return,
        };
        if !progress.visible {
            return;
        }

        let screen_rect = ui.ctx().content_rect();
        ui.ctx()
            .layer_painter(eframe::egui::LayerId::new(
                eframe::egui::Order::Foreground,
                "loading_overlay".into(),
            ))
            .rect_filled(
                screen_rect,
                0.0,
                eframe::egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
            );

        eframe::egui::Window::new("正在加载")
            .order(eframe::egui::Order::Tooltip)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(
                eframe::egui::Align2::CENTER_CENTER,
                eframe::egui::Vec2::ZERO,
            )
            .show(ui.ctx(), |ui| {
                ui.set_max_width(380.0);
                for stage in &progress.stages {
                    ui.horizontal(|ui| {
                        let icon = match stage.status {
                            StageStatus::Done => "✅",
                            StageStatus::Active => "⏳",
                            StageStatus::Pending => "⬜",
                        };
                        ui.label(icon);
                        ui.add(
                            eframe::egui::ProgressBar::new(stage.progress)
                                .desired_width(200.0)
                                .show_percentage(),
                        );
                        ui.label(eframe::egui::RichText::new(&stage.label).size(12.0));
                    });
                    if !stage.detail.is_empty() {
                        ui.label(
                            eframe::egui::RichText::new(&stage.detail)
                                .size(10.0)
                                .color(eframe::egui::Color32::GRAY),
                        );
                    }
                }
            });
    }
}
