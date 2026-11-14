//! 桌面端「导出压缩 MIDI（.7z）」：后台生成 MIDI + LZMA2 压缩。
//!
//! 生成/压缩期间用模态进度挡住编辑；模型在开始时 `Arc` 快照，之后的编辑
//! 不影响本次导出。

use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex};

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::midi_7z::{MidiArchiveFormat, export_midi_archive};
use yinhe_midi::MidiExportOptions;

use crate::app::App;

struct Job {
    path: PathBuf,
    rx: mpsc::Receiver<Result<(), String>>,
    progress: Arc<Mutex<f32>>,
}

#[derive(Default)]
pub(crate) struct Midi7zState {
    /// 等待用户选择 7z / zip。
    pub choose: bool,
    pub format: MidiArchiveFormat,
    job: Option<Job>,
    /// 进度子窗口是否仍显示（用户关掉后不再强制弹出）。
    progress_open: bool,
}

impl App {
    /// 文件菜单入口：先选 7z / zip，再选保存路径。
    pub(crate) fn export_midi_archive_dialog(&mut self) {
        if self.midi7z.job.is_some() || self.midi7z.choose {
            return;
        }
        if self.workspace.active_doc.is_none() {
            return;
        }
        self.midi7z.format = MidiArchiveFormat::SevenZ;
        self.midi7z.choose = true;
    }

    fn start_midi_archive_export(&mut self, format: MidiArchiveFormat) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let default_name = {
            let name = &self.workspace.documents[idx].file_name;
            let stem = std::path::Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("export");
            format!("{stem}.{}", format.extension())
        };
        let filter_name = if format == MidiArchiveFormat::Zip {
            "ZIP"
        } else {
            "7z"
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(filter_name, &[format.extension()])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        let path = path.with_extension(format.extension());
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
            .name("yinhe-midi-archive".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = export_midi_archive(&model, &out, format, &opts, |f| {
                    if let Ok(mut p) = p.lock() {
                        *p = f;
                    }
                });
                let _ = tx.send(result);
            });
        if spawn.is_err() {
            self.midi7z.progress_open = false;
            self.show_error(t!("midi7z.failed"), t!("midi7z.thread_failed"));
            return;
        }
        self.midi7z.progress_open = true;
        self.midi7z.job = Some(Job { path, rx, progress });
    }

    pub(crate) fn poll_midi7z(&mut self, ctx: &egui::Context) {
        let Some(job) = self.midi7z.job.take() else {
            return;
        };
        // 只有“还在跑”才把 job 放回去。旧实现默认 keep=true，成功/失败后
        // 仍把 job 塞回状态，下一帧 `try_recv` 变成 Disconnected，于是每帧
        // 都弹一次“后台线程创建失败”，直到永远。
        let mut keep = false;
        match job.rx.try_recv() {
            Ok(Ok(())) => {
                self.midi7z.progress_open = false;
                crate::chrome::dialog::close_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("midi7z_export"),
                );
                self.notifications.success(
                    t!("midi7z.done"),
                    job.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            Ok(Err(e)) => {
                self.midi7z.progress_open = false;
                crate::chrome::dialog::close_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("midi7z_export"),
                );
                self.show_error(t!("midi7z.failed"), e);
            }
            Err(TryRecvError::Disconnected) => {
                self.midi7z.progress_open = false;
                crate::chrome::dialog::close_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("midi7z_export"),
                );
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
        use std::cell::Cell;
        use std::rc::Rc;

        if self.midi7z.choose {
            let viewport_id = egui::ViewportId::from_hash_of("midi7z_choose");
            crate::chrome::dialog::raise_on_open(ctx, viewport_id);
            let title = t!("midi7z.choose_title").to_string();
            let format = Rc::new(Cell::new(self.midi7z.format));
            let chosen = Rc::new(Cell::new(None::<MidiArchiveFormat>));
            let cancelled = Rc::new(Cell::new(false));
            let open = Rc::new(Cell::new(true));
            let ctx_clone = ctx.clone();
            let format_cb = Rc::clone(&format);
            let chosen_cb = Rc::clone(&chosen);
            let cancelled_cb = Rc::clone(&cancelled);
            let open_cb = Rc::clone(&open);
            let title_cb = title.clone();
            ctx_clone.show_viewport_immediate(
                viewport_id,
                crate::chrome::dialog::viewport_builder(&title, [360.0, 250.0], false),
                move |vctx, _class| {
                    let mut close = vctx.input(|i| i.viewport().close_requested());
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::app_bg(),
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                            crate::chrome::dialog::title_bar(ui, &title_cb, &mut close, false);
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: 12,
                                    right: 12,
                                    top: 0,
                                    bottom: 12,
                                })
                                .show(ui, |ui| {
                                    let btn_zone_h =
                                        crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                                    crate::chrome::dialog::content_with_bottom_buttons(
                                        ui,
                                        btn_zone_h,
                                        |ui| {
                                            ui.add_space(8.0);
                                            ui.label(t!("midi7z.choose_hint").as_ref());
                                            ui.add_space(8.0);
                                            let mut f = format_cb.get();
                                            ui.radio_value(
                                                &mut f,
                                                MidiArchiveFormat::SevenZ,
                                                t!("midi7z.format_7z").as_ref(),
                                            );
                                            ui.radio_value(
                                                &mut f,
                                                MidiArchiveFormat::Zip,
                                                t!("midi7z.format_zip").as_ref(),
                                            );
                                            format_cb.set(f);
                                        },
                                        |ui| {
                                            use crate::chrome::dialog_buttons::{
                                                DialogButton, dialog_button_row,
                                            };
                                            let cancel = t!("common.cancel");
                                            let export = t!("midi7z.export");
                                            if let Some(idx) = dialog_button_row(
                                                ui,
                                                &[
                                                    DialogButton::secondary(cancel.as_ref()),
                                                    DialogButton::primary(export.as_ref()),
                                                ],
                                            ) {
                                                if idx == 1 {
                                                    chosen_cb.set(Some(format_cb.get()));
                                                } else {
                                                    cancelled_cb.set(true);
                                                }
                                                close = true;
                                            }
                                        },
                                    );
                                });
                        });
                    if close {
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        open_cb.set(false);
                    }
                },
            );
            if !open.get() {
                cancelled.set(true);
            }
            self.midi7z.format = format.get();
            if let Some(f) = chosen.get() {
                self.midi7z.choose = false;
                self.start_midi_archive_export(f);
            } else if cancelled.get() {
                self.midi7z.choose = false;
            }
            return;
        }

        let Some(job) = &self.midi7z.job else {
            return;
        };
        if !self.midi7z.progress_open {
            return;
        }
        let viewport_id = egui::ViewportId::from_hash_of("midi7z_export");
        crate::chrome::dialog::raise_on_open(ctx, viewport_id);
        let title = t!("midi7z.working").to_string();
        let progress = Arc::clone(&job.progress);
        let path = job.path.clone();
        let open = Rc::new(Cell::new(true));
        let ctx_clone = ctx.clone();
        let open_cb = Rc::clone(&open);
        let title_cb = title.clone();
        ctx_clone.show_viewport_immediate(
            viewport_id,
            crate::chrome::dialog::viewport_builder(&title, [420.0, 170.0], false),
            move |vctx, _class| {
                let mut close = vctx.input(|i| i.viewport().close_requested());
                egui::CentralPanel::default()
                    .frame(egui::Frame {
                        fill: crate::theme::app_bg(),
                        ..Default::default()
                    })
                    .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(ui, &title_cb, &mut close, false);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 16,
                                right: 16,
                                top: 0,
                                bottom: 16,
                            })
                            .show(ui, |ui| {
                                ui.add_space(8.0);
                                ui.label(
                                    path.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or_default(),
                                );
                                ui.add_space(6.0);
                                let f = progress.lock().map(|p| p.clamp(0.0, 1.0)).unwrap_or(0.0);
                                ui.add(
                                    egui::ProgressBar::new(f)
                                        .show_percentage()
                                        .desired_width(360.0),
                                );
                            });
                    });
                if close {
                    vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    open_cb.set(false);
                }
            },
        );
        if !open.get() {
            // 用户关掉进度窗口：任务继续，完成后出通知。
            self.midi7z.progress_open = false;
        }
    }
}
