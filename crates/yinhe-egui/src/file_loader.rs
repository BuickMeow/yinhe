use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use rust_i18n::t;

use yinhe_core::YinModel;
use yinhe_mid2::{LoadProgress, MidiImportEncoding};
use yinhe_yin::{MappingFile, ProjectSoundFonts};

use crate::dialogs::archive_picker::{ArchivePickerState, PasswordPrompt};
use yinhe_editor_core::progress::{self, SharedProgress, StageStatus};

/// Events sent from the background loading thread to the UI thread.
pub(crate) enum MidiLoadEvent {
    Progress(LoadProgress),
    Complete(Box<Result<YinModel, yinhe_mid2::MidiError>>),
}

/// Events for .yin project loading.
pub(crate) enum YinLoadEvent {
    Complete(Result<(YinModel, ProjectSoundFonts, MappingFile, String), String>),
}

/// Events for archive opening.
pub(crate) enum ArchiveLoadEvent {
    Complete(
        Result<
            (yinhe_archive::Archive, Vec<yinhe_archive::ArchiveEntry>),
            yinhe_archive::ArchiveError,
        >,
    ),
}

pub(crate) struct MidiLoader {
    pub path: String,
    pub rx: mpsc::Receiver<MidiLoadEvent>,
    pub current_progress: Option<LoadProgress>,
    pub cancel: Arc<AtomicBool>,
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
        mapping: MappingFile,
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
    pub password_prompt: Option<PasswordPrompt>,
    load_progress: SharedProgress,
}

impl FileLoader {
    pub fn new(load_progress: SharedProgress) -> Self {
        Self {
            midi_loader: None,
            yin_loader: None,
            archive_loader: None,
            archive_picker: None,
            password_prompt: None,
            load_progress,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.midi_loader.is_some()
            || self.yin_loader.is_some()
            || self.archive_loader.is_some()
            || self.archive_picker.is_some()
            || self.password_prompt.is_some()
    }

    pub fn load_progress(&self) -> &SharedProgress {
        &self.load_progress
    }

    /// Cancel any in-progress loading. Sets the cancel flag so background
    /// threads stop reporting progress, and clears the loader state.
    pub fn cancel_loading(&mut self) {
        if let Some(ref loader) = self.midi_loader {
            loader.cancel.store(true, Ordering::Relaxed);
        }
        self.midi_loader = None;
        self.yin_loader = None;
        self.archive_loader = None;
        self.password_prompt = None;
        progress::set_visible(&self.load_progress, false);
    }

    /// Show file dialog and start loading in a background thread.
    pub fn pick_file(&mut self, encoding: MidiImportEncoding) {
        if self.is_loading() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                t!("file_dialog.all_supported").as_ref(),
                &[
                    "mid", "midi", "yin", "zip", "7z", "rar", "lzh", "lha", "tar", "gz", "xz",
                    "tgz", "txz", "tbz", "bz2",
                ],
            )
            .add_filter(t!("file_dialog.midi").as_ref(), &["mid", "midi"])
            .add_filter(t!("file_dialog.yinhe_project").as_ref(), &["yin"])
            .add_filter(
                t!("file_dialog.archive").as_ref(),
                &[
                    "zip", "7z", "rar", "lzh", "lha", "tar", "gz", "xz", "tgz", "txz", "tbz", "bz2",
                ],
            )
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
                "zip" | "7z" | "rar" | "lzh" | "lha" | "tar" | "gz" | "xz" | "tgz" | "txz"
                | "tbz" | "bz2" => self.start_archive(path_str, None),
                _ => self.start_midi(path_str, encoding),
            }
        }
    }

    fn start_yin(&mut self, path_str: String) {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = path_str.clone();
        let progress = self.load_progress.clone();
        std::thread::spawn(move || {
            progress::set_stage_label(&progress, 0, t!("dialog.loading.yin_stage").to_string());
            progress::set_stage(&progress, 0, StageStatus::Active);
            let result = yinhe_yin::load_yin_with_sf_progress(&path_for_thread, |p| {
                let detail = match p.stage {
                    yinhe_yin::YinProgressStage::Decompress => {
                        t!("dialog.loading.yin_decompress").to_string()
                    }
                    yinhe_yin::YinProgressStage::Rebuild => {
                        t!("dialog.loading.yin_rebuild").to_string()
                    }
                    yinhe_yin::YinProgressStage::Resort => {
                        t!("dialog.loading.yin_resort").to_string()
                    }
                    _ => String::new(),
                };
                progress::set_stage_progress(&progress, 0, p.fraction, detail);
            });
            match result {
                Ok((model, sf, mapping)) => {
                    let file_name = std::path::Path::new(&path_for_thread)
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    progress::set_stage(&progress, 0, StageStatus::Done);
                    progress::set_visible(&progress, false);
                    let _ = tx.send(YinLoadEvent::Complete(Ok((model, sf, mapping, file_name))));
                }
                Err(e) => {
                    progress::set_visible(&progress, false);
                    let _ = tx.send(YinLoadEvent::Complete(Err(e.to_string())));
                }
            }
        });
        self.yin_loader = Some(YinLoader { path: path_str, rx });
    }

    /// Start loading an archive with optional password.
    /// `password == None` means no password; `Some("")` is treated as no password.
    pub(crate) fn start_archive(&mut self, path_str: String, password: Option<String>) {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = path_str.clone();
        std::thread::spawn(move || {
            let result =
                yinhe_archive::Archive::open_with_password(&path_for_thread, password.as_deref())
                    .map(|archive| {
                        let entries = archive.list_midi_files();
                        (archive, entries)
                    });
            let _ = tx.send(ArchiveLoadEvent::Complete(result));
        });
        self.archive_loader = Some(ArchiveLoader { path: path_str, rx });
    }

    fn start_midi(&mut self, path_str: String, encoding: MidiImportEncoding) {
        let (tx, rx) = mpsc::channel();
        let path_for_thread = path_str.clone();
        let progress = self.load_progress.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            let data = match std::fs::read(&path_for_thread) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(MidiLoadEvent::Complete(Box::new(Err(
                        yinhe_mid2::MidiError::Io(e),
                    ))));
                    return;
                }
            };
            Self::parse_midi_and_report(data, encoding, tx, progress, cancel_for_thread);
        });
        self.midi_loader = Some(MidiLoader {
            path: path_str,
            rx,
            current_progress: None,
            cancel,
        });
    }

    /// Parse MIDI bytes in the current thread and report progress/result via channel.
    fn parse_midi_and_report(
        data: Vec<u8>,
        encoding: MidiImportEncoding,
        tx: mpsc::Sender<MidiLoadEvent>,
        progress: SharedProgress,
        cancel: Arc<AtomicBool>,
    ) {
        progress::set_stage(&progress, 0, StageStatus::Active);
        let tx_inner = tx.clone();
        let result = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
            yinhe_mid2::parse_bytes_with_encoding(&data, encoding, |p| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let _ = tx_inner.send(MidiLoadEvent::Progress(p));
            })
        });
        // stage 0 覆盖整个解析（音轨并行解析 + 模型构建），完成才标 Done。
        progress::set_stage(&progress, 0, StageStatus::Done);
        progress::set_visible(&progress, false);
        let _ = tx.send(MidiLoadEvent::Complete(Box::new(result)));
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
                        Ok((model, sf, mapping, file_name)) => {
                            let path = loader.path.clone();
                            progress::set_visible(&self.load_progress, false);
                            return LoadResult::ModelFromYin {
                                path,
                                model,
                                file_name,
                                sf,
                                mapping,
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
                            // 密码错误/缺失：弹出密码输入框而非直接报错
                            match &e {
                                yinhe_archive::ArchiveError::PasswordRequired => {
                                    progress::set_visible(&self.load_progress, false);
                                    self.password_prompt =
                                        Some(PasswordPrompt::new(loader.path.clone(), false));
                                    return LoadResult::NotReady;
                                }
                                yinhe_archive::ArchiveError::WrongPassword => {
                                    progress::set_visible(&self.load_progress, false);
                                    self.password_prompt =
                                        Some(PasswordPrompt::new(loader.path.clone(), true));
                                    return LoadResult::NotReady;
                                }
                                _ => {
                                    return LoadResult::ArchiveError(format!(
                                        "打开压缩包失败: {}",
                                        e
                                    ));
                                }
                            }
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
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = cancel.clone();
        progress::set_visible(&self.load_progress, true);
        std::thread::spawn(move || {
            let data = match archive.read_file(&entry_name) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(MidiLoadEvent::Complete(Box::new(Err(
                        yinhe_mid2::MidiError::Io(std::io::Error::other(e.to_string())),
                    ))));
                    return;
                }
            };
            Self::parse_midi_and_report(
                data,
                MidiImportEncoding::Utf8,
                tx,
                progress,
                cancel_for_thread,
            );
        });
        self.midi_loader = Some(MidiLoader {
            path: entry.name,
            rx,
            current_progress: None,
            cancel,
        });
    }
}
