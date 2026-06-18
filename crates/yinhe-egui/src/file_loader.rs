use std::sync::mpsc;

use yinhe_core::YinModel;
use yinhe_mid2::{LoadProgress, MidiImportEncoding};
use yinhe_yin::ProjectSoundFonts;

use crate::dialogs::archive_picker::ArchivePickerState;
use yinhe_editor_core::progress::{self, SharedProgress, StageStatus};

/// Events sent from the background loading thread to the UI thread.
pub(crate) enum MidiLoadEvent {
    Progress(LoadProgress),
    Complete(Box<Result<YinModel, yinhe_mid2::MidiError>>),
}

/// Events for .yin project loading.
pub(crate) enum YinLoadEvent {
    Complete(Result<(YinModel, ProjectSoundFonts, String), String>),
}

/// Events for archive opening.
pub(crate) enum ArchiveLoadEvent {
    Complete(Result<(yinhe_archive::Archive, Vec<yinhe_archive::ArchiveEntry>), String>),
}

pub(crate) struct MidiLoader {
    pub path: String,
    pub rx: mpsc::Receiver<MidiLoadEvent>,
    pub current_progress: Option<LoadProgress>,
}

pub(crate) struct YinLoader {
    pub path: String,
    pub rx: mpsc::Receiver<YinLoadEvent>,
}

pub(crate) struct ArchiveLoader {
    pub path: String,
    pub rx: mpsc::Receiver<ArchiveLoadEvent>,
}

/// Result of polling the async loader.
pub(crate) enum LoadResult {
    ModelLoaded {
        path: String,
        model: YinModel,
    },
    ModelFromYin {
        path: String,
        model: YinModel,
        file_name: String,
        sf: ProjectSoundFonts,
    },
    ArchiveError(String),
    NotReady,
}

/// Manages async file loading (file dialog + background thread).
pub(crate) struct FileLoader {
    midi_loader: Option<MidiLoader>,
    yin_loader: Option<YinLoader>,
    archive_loader: Option<ArchiveLoader>,
    pub archive_picker: Option<ArchivePickerState>,
    load_progress: SharedProgress,
}

impl FileLoader {
    pub fn new(load_progress: SharedProgress) -> Self {
        Self {
            midi_loader: None,
            yin_loader: None,
            archive_loader: None,
            archive_picker: None,
            load_progress,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.midi_loader.is_some()
            || self.yin_loader.is_some()
            || self.archive_loader.is_some()
            || self.archive_picker.is_some()
    }

    pub fn load_progress(&self) -> &SharedProgress {
        &self.load_progress
    }

    /// Show file dialog and start loading in a background thread.
    pub fn pick_file(&mut self, encoding: MidiImportEncoding) {
        if self.is_loading() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "All supported",
                &["mid", "midi", "yin", "zip", "7z", "tar", "gz", "xz", "tgz", "txz"],
            )
            .add_filter("MIDI", &["mid", "midi"])
            .add_filter("Yinhe Project", &["yin"])
            .add_filter("Archive", &["zip", "7z", "tar", "gz", "xz", "tgz", "txz"])
            .pick_file()
        {
            let path_str = path.to_string_lossy().to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();

            progress::set_visible(&self.load_progress, true);

            match ext.as_str() {
                "yin" => self.start_yin(path_str),
                "zip" | "7z" | "tar" | "gz" | "xz" | "tgz" | "txz" => self.start_archive(path_str),
                _ => self.start_midi(path_str, encoding),
            }
        }
    }

    fn start_yin(&mut self, path_str: String) {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = path_str.clone();
        let progress = self.load_progress.clone();
        std::thread::spawn(move || {
            progress::set_stage(&progress, 0, StageStatus::Done);
            progress::set_stage(&progress, 1, StageStatus::Active);
            let result = yinhe_yin::load_yin_with_sf(&path_for_thread);
            match result {
                Ok((model, sf)) => {
                    let file_name = std::path::Path::new(&path_for_thread)
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    progress::set_stage(&progress, 1, StageStatus::Done);
                    progress::set_visible(&progress, false);
                    let _ = tx.send(YinLoadEvent::Complete(Ok((model, sf, file_name))));
                }
                Err(e) => {
                    progress::set_visible(&progress, false);
                    let _ = tx.send(YinLoadEvent::Complete(Err(e.to_string())));
                }
            }
        });
        self.yin_loader = Some(YinLoader {
            path: path_str,
            rx,
        });
    }

    fn start_archive(&mut self, path_str: String) {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = path_str.clone();
        std::thread::spawn(move || {
            let result = yinhe_archive::Archive::open(&path_for_thread)
                .map(|archive| {
                    let entries = archive.list_midi_files();
                    (archive, entries)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(ArchiveLoadEvent::Complete(result));
        });
        self.archive_loader = Some(ArchiveLoader {
            path: path_str,
            rx,
        });
    }

    fn start_midi(&mut self, path_str: String, encoding: MidiImportEncoding) {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = path_str.clone();
        let progress = self.load_progress.clone();
        std::thread::spawn(move || {
            progress::set_stage(&progress, 0, StageStatus::Active);
            let tx_inner = tx.clone();
            let result = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
                let data = match std::fs::read(&path_for_thread) {
                    Ok(d) => d,
                    Err(e) => return Err(yinhe_mid2::MidiError::Io(e)),
                };
                yinhe_mid2::parse_bytes_with_encoding(&data, encoding, |p| {
                    let _ = tx_inner.send(MidiLoadEvent::Progress(p));
                })
            });
            progress::set_stage(&progress, 0, StageStatus::Done);
            // Stage 1 used to be archive pre-conversion that always discarded
            // its result. Removed in Phase 4b: mark done immediately.
            progress::set_stage(&progress, 1, StageStatus::Done);
            progress::set_visible(&progress, false);
            let _ = tx.send(MidiLoadEvent::Complete(Box::new(result)));
        });
        self.midi_loader = Some(MidiLoader {
            path: path_str,
            rx,
            current_progress: None,
        });
    }
}

impl FileLoader {
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
                            Ok(model) => {
                                let path = loader.path.clone();
                                progress::set_visible(&self.load_progress, false);
                                return LoadResult::ModelLoaded { path, model };
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
                        Ok((model, sf, file_name)) => {
                            let path = loader.path.clone();
                            progress::set_visible(&self.load_progress, false);
                            return LoadResult::ModelFromYin {
                                path,
                                model,
                                file_name,
                                sf,
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

        // Poll Archive loader
        if let Some(loader) = self.archive_loader.take() {
            if let Ok(event) = loader.rx.try_recv() {
                match event {
                    ArchiveLoadEvent::Complete(result) => match result {
                        Ok((archive, entries)) => {
                            progress::set_visible(&self.load_progress, false);
                            if entries.is_empty() {
                                tracing::warn!("压缩包中没有找到 MIDI 文件: {}", loader.path);
                                return LoadResult::ArchiveError(
                                    "压缩包中没有找到 MIDI 文件".to_string(),
                                );
                            }
                            if entries.len() == 1 {
                                let entry = entries[0].clone();
                                tracing::info!(
                                    "压缩包中只有一个 MIDI 文件，直接加载: {}",
                                    entry.name
                                );
                                self.start_load_from_archive(archive, entry);
                                return LoadResult::NotReady;
                            }
                            self.archive_picker = Some(ArchivePickerState::Opening {
                                path: loader.path,
                                rx: {
                                    let (tx, rx) = mpsc::channel();
                                    let _ = tx.send(Ok((archive, entries)));
                                    rx
                                },
                            });
                            return LoadResult::NotReady;
                        }
                        Err(e) => {
                            tracing::error!("打开压缩包失败: {}", e);
                            return LoadResult::ArchiveError(format!("打开压缩包失败: {}", e));
                        }
                    },
                }
            }
            self.archive_loader = Some(loader);
        }

        LoadResult::NotReady
    }

    /// Start loading a MIDI file from an archive entry.
    pub fn start_load_from_archive(
        &mut self,
        archive: yinhe_archive::Archive,
        entry: yinhe_archive::ArchiveEntry,
    ) {
        let (tx, rx) = mpsc::channel();
        let progress = self.load_progress.clone();
        let entry_name = entry.name.clone();
        progress::set_visible(&self.load_progress, true);
        std::thread::spawn(move || {
            progress::set_stage(&progress, 0, StageStatus::Active);
            let read_result = archive.read_file(&entry_name).map_err(|e| {
                yinhe_mid2::MidiError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            });
            match read_result {
                Ok(data) => {
                    let load_result = yinhe_mid2::parse_bytes_with_encoding(
                        &data,
                        MidiImportEncoding::Utf8,
                        |_p| {},
                    );
                    progress::set_stage(&progress, 0, StageStatus::Done);
                    progress::set_stage(&progress, 1, StageStatus::Done);
                    progress::set_visible(&progress, false);
                    let _ = tx.send(MidiLoadEvent::Complete(Box::new(load_result)));
                }
                Err(e) => {
                    let _ = tx.send(MidiLoadEvent::Complete(Box::new(Err(e))));
                }
            }
        });
        self.midi_loader = Some(MidiLoader {
            path: entry.name,
            rx,
            current_progress: None,
        });
    }

    /// Draw a dark overlay + centered window with multi-stage loading progress.
    pub fn show_loading_overlay(&self, ui: &mut eframe::egui::Ui) {
        crate::dialogs::loading_overlay::show(ui, self);
    }
}
