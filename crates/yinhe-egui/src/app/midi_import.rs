//! 桌面端「导入 MIDI」：文件选择 → 后台解析 → 确认框 → 后台合并。
//!
//! 解析与合并都在子线程；合并期间用 `egui::Modal` 挡住编辑，完成后一次性
//! 换入新模型并推入单步 undo（导入不保留源数据，undo 后清空 redo）。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, TryRecvError};

use eframe::egui;
use rust_i18n::t;
use yinhe_core::YinModel;
use yinhe_editor_core::history::EditSnapshot;
use yinhe_editor_core::midi_import::{
    ImportChannelMode, MidiImportMerged, MidiImportOptions, MidiImportSummary,
    merge_midi_into_model,
};

use crate::app::App;
use crate::chrome::mode_bar::ViewMode;

struct ParseJob {
    path: String,
    rx: mpsc::Receiver<Result<Box<YinModel>, String>>,
    progress: Arc<AtomicU32>,
}

struct PendingImport {
    path: String,
    model: Box<YinModel>,
    summary: MidiImportSummary,
    include: Vec<bool>,
    allocate_free: bool,
}

struct ApplyJob {
    doc_idx: usize,
    rx: mpsc::Receiver<(YinModel, Option<MidiImportMerged>)>,
    progress: Arc<AtomicU32>,
    before: EditSnapshot,
}

#[derive(Default)]
pub(crate) struct MidiImportState {
    parse: Option<ParseJob>,
    pending: Option<PendingImport>,
    apply: Option<ApplyJob>,
}

fn frac(progress: &AtomicU32) -> f32 {
    progress.load(Ordering::Relaxed) as f32 / 1000.0
}

impl App {
    /// 文件菜单入口：选文件并开始后台解析。
    pub(crate) fn import_midi_dialog(&mut self, _ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .pick_file()
        else {
            return;
        };
        if self.midi_import.parse.is_some() || self.midi_import.apply.is_some() {
            return;
        }
        let path_str = path.display().to_string();
        let encoding = self.audio_settings.midi_import_encoding;
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::channel();
        let worker_path = path_str.clone();
        let p = Arc::clone(&progress);
        let spawn = std::thread::Builder::new()
            .name("yinhe-midi-import-parse".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let result = yinhe_midi::parse_file_with_encoding(&worker_path, encoding, |pr| {
                    let f = if pr.total_tracks > 0 {
                        pr.current_track as f32 / pr.total_tracks as f32
                    } else {
                        0.0
                    };
                    p.store((20.0 + f * 930.0) as u32, Ordering::Relaxed);
                });
                let _ = tx.send(result.map(Box::new).map_err(|e| e.to_string()));
            });
        if spawn.is_err() {
            self.show_error(t!("midi_import.failed"), t!("midi_import.thread_failed"));
            return;
        }
        self.midi_import.parse = Some(ParseJob {
            path: path_str,
            rx,
            progress,
        });
    }

    /// 每帧轮询解析/合并结果。
    pub(crate) fn poll_midi_import(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.midi_import.parse {
            match job.rx.try_recv() {
                Ok(Ok(model)) => {
                    let summary = MidiImportSummary::from_model(&model);
                    let include = vec![true; summary.tracks.len()];
                    self.midi_import.pending = Some(PendingImport {
                        path: job.path.clone(),
                        model,
                        summary,
                        include,
                        allocate_free: false,
                    });
                    self.midi_import.parse = None;
                }
                Ok(Err(e)) => {
                    self.midi_import.parse = None;
                    self.show_error(t!("midi_import.failed"), e);
                }
                Err(TryRecvError::Disconnected) => {
                    self.midi_import.parse = None;
                    self.show_error(t!("midi_import.failed"), t!("midi_import.thread_failed"));
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                }
            }
        }

        if let Some(job) = self.midi_import.apply.take() {
            let mut keep = true;
            match job.rx.try_recv() {
                Ok((model, Some(merged))) => {
                    let outcome =
                        self.workspace.documents.get_mut(job.doc_idx).map(|doc| {
                            doc.install_imported_model(model, merged, job.before.clone())
                        });
                    let Some(outcome) = outcome else {
                        self.show_error(t!("midi_import.failed"), t!("midi_import.doc_closed"));
                        return;
                    };
                    if self.workspace.active_doc == Some(job.doc_idx) {
                        self.view_mode = ViewMode::Arrange;
                        if let Some(doc) = self.workspace.documents.get_mut(job.doc_idx) {
                            let ppt = doc.edit.arrange_view.base.pixels_per_tick.max(0.001);
                            doc.edit.arrange_view.base.scroll_x =
                                (outcome.start_tick as f32 * ppt - 160.0).max(0.0);
                            let row_h = doc.edit.arrange_view.lane_height();
                            doc.edit.arrange_view.base.scroll_y =
                                (outcome.first_track as f32 * row_h - 120.0).max(0.0);
                            doc.edit.arrange_view.base.track_panel_scroll_y =
                                doc.edit.arrange_view.base.scroll_y;
                            doc.edit.arrange_view.base.dirty = true;
                        }
                        self.notify_audio_model_changed();
                    }
                    self.notifications.success(
                        t!("midi_import.done"),
                        t!(
                            "midi_import.done_detail",
                            notes = outcome.note_count,
                            tracks = outcome.last_track - outcome.first_track + 1
                        ),
                    );
                }
                Ok((_, None)) => {
                    self.show_error(t!("midi_import.failed"), t!("midi_import.empty"));
                }
                Err(TryRecvError::Disconnected) => {
                    self.show_error(t!("midi_import.failed"), t!("midi_import.thread_failed"));
                }
                Err(TryRecvError::Empty) => {
                    keep = true;
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                }
            }
            if keep {
                self.midi_import.apply = Some(job);
            }
        }
    }

    /// 解析中 / 确认框 / 合并中的模态 UI。
    pub(crate) fn show_midi_import_ui(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.midi_import.parse {
            let f = frac(&job.progress);
            egui::Modal::new(egui::Id::unique("midi_import_parsing")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading(t!("midi_import.parsing"));
                ui.label(&job.path);
                ui.add(
                    egui::ProgressBar::new(f)
                        .show_percentage()
                        .desired_width(320.0),
                );
            });
            return;
        }

        if self.midi_import.apply.is_some() {
            let f = self
                .midi_import
                .apply
                .as_ref()
                .map(|j| frac(&j.progress))
                .unwrap_or(0.0);
            egui::Modal::new(egui::Id::unique("midi_import_applying")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading(t!("midi_import.applying"));
                ui.add(
                    egui::ProgressBar::new(f)
                        .show_percentage()
                        .desired_width(320.0),
                );
            });
            return;
        }

        let Some(pending) = self.midi_import.pending.as_mut() else {
            return;
        };
        let mut action = None;
        let response = egui::Modal::new(egui::Id::unique("midi_import_confirm")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(t!("midi_import.title"));
            ui.label(&pending.path);
            ui.label(t!(
                "midi_import.summary",
                tracks = pending.summary.tracks.len(),
                notes = pending.summary.note_count,
                ppq = pending.summary.ppq
            ));
            ui.label(t!("midi_import.position"));
            if pending.summary.tempo_events > 0 || pending.summary.time_sig_events > 0 {
                ui.label(t!(
                    "midi_import.global_events",
                    tempo = pending.summary.tempo_events,
                    sig = pending.summary.time_sig_events
                ));
            }
            ui.separator();
            ui.label(t!("midi_import.tracks"));
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, track) in pending.summary.tracks.iter().enumerate() {
                        let mut checked = pending.include.get(i).copied().unwrap_or(false);
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut checked, "").changed()
                                && let Some(slot) = pending.include.get_mut(i)
                            {
                                *slot = checked;
                            }
                            ui.label(
                                egui::RichText::new(if track.name.is_empty() {
                                    format!("Track {}", i + 1)
                                } else {
                                    track.name.clone()
                                })
                                .strong(),
                            );
                            ui.weak(format!(
                                "CH {}{} · {} notes{}",
                                (b'A' + track.port.min(15)) as char,
                                track.channel + 1,
                                track.note_count,
                                if track.has_automation { " · CC" } else { "" }
                            ));
                        });
                    }
                });
            ui.separator();
            ui.radio_value(
                &mut pending.allocate_free,
                false,
                t!("midi_import.keep_channels"),
            );
            ui.radio_value(
                &mut pending.allocate_free,
                true,
                t!("midi_import.free_channels"),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(t!("common.cancel")).clicked() {
                    action = Some(false);
                }
                let any = pending.include.iter().any(|v| *v);
                if ui
                    .add_enabled(any, egui::Button::new(t!("midi_import.import")))
                    .clicked()
                {
                    action = Some(true);
                }
            });
        });

        if response.should_close() {
            action = Some(false);
        }
        match action {
            Some(true) => self.start_midi_import_apply(),
            Some(false) => self.midi_import.pending = None,
            None => {}
        }
    }

    fn start_midi_import_apply(&mut self) {
        let Some(pending) = self.midi_import.pending.take() else {
            return;
        };
        let Some(idx) = self.workspace.active_doc else {
            self.show_error(t!("midi_import.failed"), t!("midi_import.no_doc"));
            return;
        };
        let before = self.workspace.documents[idx].capture_snapshot();
        let model = Arc::clone(&self.workspace.documents[idx].data.model);
        let opts = MidiImportOptions {
            include: pending.include,
            channel_mode: if pending.allocate_free {
                ImportChannelMode::AllocateFree
            } else {
                ImportChannelMode::KeepSource
            },
        };
        let src = *pending.model;
        let progress = Arc::new(AtomicU32::new(0));
        let p = Arc::clone(&progress);
        let (tx, rx) = mpsc::channel();
        let spawn = std::thread::Builder::new()
            .name("yinhe-midi-import-apply".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let mut new_model = (*model).clone();
                let merged = merge_midi_into_model(&mut new_model, src, &opts, |f| {
                    p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
                });
                let _ = tx.send((new_model, merged));
            });
        if spawn.is_err() {
            self.show_error(t!("midi_import.failed"), t!("midi_import.thread_failed"));
            return;
        }
        self.midi_import.apply = Some(ApplyJob {
            doc_idx: idx,
            rx,
            progress,
            before,
        });
    }
}
