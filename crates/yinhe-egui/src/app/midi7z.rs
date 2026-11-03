//! 桌面端「导出压缩 MIDI（.7z）」：后台生成 MIDI + LZMA2 压缩。
//!
//! 生成/压缩期间用模态进度挡住编辑；模型在开始时 `Arc` 快照，之后的编辑
//! 不影响本次导出。

use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex};

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::midi_7z::export_midi_7z;
use yinhe_midi::MidiExportOptions;

use crate::app::App;

struct Job {
    path: PathBuf,
    rx: mpsc::Receiver<Result<(), String>>,
    progress: Arc<Mutex<f32>>,
}

#[derive(Default)]
pub(crate) struct Midi7zState {
    job: Option<Job>,
}

impl App {
    /// 文件菜单入口：选择 `.7z` 目标并启动后台任务。
    pub(crate) fn export_midi_7z_dialog(&mut self) {
        if self.midi7z.job.is_some() {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let default_name = {
            let name = &self.workspace.documents[idx].file_name;
            let stem = std::path::Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("export");
            format!("{stem}.7z")
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("7z", &["7z"])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        let path = if path.extension().is_some() {
            path
        } else {
            path.with_extension("7z")
        };
        let model = Arc::clone(&self.workspace.documents[idx].data.model);
        let opts = MidiExportOptions {
            encoding: self.audio_settings.midi_export_encoding,
            rpn_full: self.audio_settings.midi_export_rpn_full,
            curve_interpolate: self.audio_settings.midi_export_curve_interpolate,
            curve_density: self.audio_settings.midi_export_curve_density,
            strip_empty_tracks: self.audio_settings.midi_export_strip_empty_tracks,
            dedup_overlaps: self.audio_settings.midi_export_dedup_overlaps,
        };
        let progress = Arc::new(Mutex::new(0.0));
        let p = Arc::clone(&progress);
        let (tx, rx) = mpsc::channel();
        let out = path.clone();
        let spawn = std::thread::Builder::new()
            .name("yinhe-midi-7z".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = export_midi_7z(&model, &out, &opts, |f| {
                    if let Ok(mut p) = p.lock() {
                        *p = f;
                    }
                });
                let _ = tx.send(result);
            });
        if spawn.is_err() {
            self.show_error(t!("midi7z.failed"), t!("midi7z.thread_failed"));
            return;
        }
        self.midi7z.job = Some(Job { path, rx, progress });
    }

    pub(crate) fn poll_midi7z(&mut self, ctx: &egui::Context) {
        let Some(job) = self.midi7z.job.take() else {
            return;
        };
        let mut keep = true;
        match job.rx.try_recv() {
            Ok(Ok(())) => {
                self.notifications.success(
                    t!("midi7z.done"),
                    job.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            Ok(Err(e)) => self.show_error(t!("midi7z.failed"), e),
            Err(TryRecvError::Disconnected) => {
                self.show_error(t!("midi7z.failed"), t!("midi7z.thread_failed"));
            }
            Err(TryRecvError::Empty) => {
                keep = true;
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
        }
        if keep {
            self.midi7z.job = Some(job);
        }
    }

    pub(crate) fn show_midi7z_ui(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.midi7z.job else {
            return;
        };
        let f = job
            .progress
            .lock()
            .map(|p| p.clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let path = job.path.clone();
        egui::Modal::new(egui::Id::unique("midi7z_export")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(t!("midi7z.working"));
            ui.label(
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default(),
            );
            ui.add(
                egui::ProgressBar::new(f)
                    .show_percentage()
                    .desired_width(340.0),
            );
        });
    }
}
