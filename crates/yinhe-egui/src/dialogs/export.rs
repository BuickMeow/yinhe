use std::sync::{Arc, Mutex};

use eframe::egui;

pub use yinhe_audio::export::ExportProgress;

/// Captured state when an export finishes successfully, used to show the
/// completion dialog with elapsed time, overall speed, and an "open folder" button.
pub(crate) struct ExportCompleted {
    pub file_path: String,
    pub elapsed_secs: f64,
    pub overall_speed: f64,
}

pub(crate) fn show_progress_viewport(
    ctx: &egui::Context,
    export_progress: Arc<Mutex<ExportProgress>>,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
) {
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        egui::ViewportId::from_hash_of("export_progress_dialog"),
        crate::chrome::dialog::viewport_builder("导出音频中", [320.0, 310.0], false),
        move |vctx, _class| {
            let state = match export_progress.lock() {
                Ok(s) => s.clone(),
                Err(_) => return,
            };
            if !state.visible {
                return;
            }
            let close_requested = vctx.input(|i| i.viewport().close_requested());
            let mut close = close_requested;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, "导出音频中", &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(4.0);
                                ui.add(
                                    egui::ProgressBar::new(state.progress)
                                        .desired_width(280.0),
                                );
                                ui.add_space(8.0);

                                egui::Grid::new("export_progress_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("总时长");
                                        ui.label(format_duration(state.total_duration_secs));
                                        ui.end_row();

                                        ui.label("已渲染");
                                        ui.label(format_duration(state.rendered_secs));
                                        ui.end_row();

                                        ui.label("已用时间");
                                        let elapsed = state
                                            .started_at
                                            .map(|t| t.elapsed().as_secs_f64())
                                            .unwrap_or(0.0);
                                        ui.label(format_duration(elapsed));
                                        ui.end_row();

                                        ui.label("复音数");
                                        ui.label(format!("{}", state.voice_count));
                                        ui.end_row();

                                        ui.label("实时倍速");
                                        if state.render_speed > 0.0 {
                                            ui.label(format!("{:.2}x", state.render_speed));
                                        } else {
                                            ui.label("—");
                                        }
                                        ui.end_row();

                                        ui.label("整体倍速");
                                        if state.overall_speed > 0.0 {
                                            ui.label(format!("{:.2}x", state.overall_speed));
                                        } else {
                                            ui.label("—");
                                        }
                                        ui.end_row();
                                    });

                                ui.add_space(4.0);
                                if !state.status.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&state.status)
                                            .size(12.0)
                                            .color(egui::Color32::LIGHT_GRAY),
                                    );
                                }
                            });
                        });
                });
            if close {
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    ctx.request_repaint();
}

pub(crate) fn show_completed_viewport(ctx: &egui::Context, completed: &mut Option<ExportCompleted>) {
    let file_path = match completed {
        Some(c) => c.file_path.clone(),
        None => return,
    };
    let elapsed = completed.as_ref().unwrap().elapsed_secs;
    let speed = completed.as_ref().unwrap().overall_speed;

    let open = std::rc::Rc::new(std::cell::RefCell::new(true));
    let open_cb = open.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        egui::ViewportId::from_hash_of("export_completed_dialog"),
        crate::chrome::dialog::viewport_builder("导出完成", [320.0, 200.0], false),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, "导出完成", &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(4.0);
                                egui::Grid::new("export_completed_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("已用时间");
                                        ui.label(format_duration(elapsed));
                                        ui.end_row();

                                        ui.label("整体倍速");
                                        if speed > 0.0 {
                                            ui.label(format!("{:.2}x", speed));
                                        } else {
                                            ui.label("—");
                                        }
                                        ui.end_row();
                                    });

                                ui.add_space(12.0);
                                if ui.button("打开所在文件夹").clicked() {
                                    let parent = std::path::Path::new(&file_path)
                                        .parent()
                                        .map(|p| p.to_path_buf());
                                    if let Some(dir) = parent {
                                        #[cfg(target_os = "macos")]
                                        let _ = std::process::Command::new("open")
                                            .arg(&dir)
                                            .spawn();
                                        #[cfg(target_os = "windows")]
                                        let _ = std::process::Command::new("explorer")
                                            .arg(&dir)
                                            .spawn();
                                        #[cfg(target_os = "linux")]
                                        let _ = std::process::Command::new("xdg-open")
                                            .arg(&dir)
                                            .spawn();
                                    }
                                }
                                ui.add_space(4.0);
                            });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_cb.borrow_mut() = false;
            }
        },
    );

    if !*open.borrow() {
        *completed = None;
    }
}

pub(crate) fn show_settings_viewport(
    ctx: &egui::Context,
    show: &mut bool,
    sample_rate: u32,
    bit_depth: &mut yinhe_audio::export::WavBitDepth,
    layer_count: &mut u32,
    export_sample_rate: &mut u32,
) -> bool {
    if !*show {
        return false;
    }

    let bd = std::rc::Rc::new(std::cell::Cell::new(*bit_depth));
    let lc = std::rc::Rc::new(std::cell::Cell::new(*layer_count));
    let sr = std::rc::Rc::new(std::cell::Cell::new(*export_sample_rate));
    let open = std::rc::Rc::new(std::cell::RefCell::new(true));
    let started = std::rc::Rc::new(std::cell::RefCell::new(false));
    let ctx_clone = ctx.clone();
    let open_cb = open.clone();
    let started_cb = started.clone();
    let bd_cb = bd.clone();
    let lc_cb = lc.clone();
    let sr_cb = sr.clone();

    ctx_clone.show_viewport_immediate(
        egui::ViewportId::from_hash_of("export_settings_dialog"),
        crate::chrome::dialog::viewport_builder("导出音频", [320.0, 260.0], false),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, "导出音频", &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(280.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);

                                ui.horizontal(|ui| {
                                    ui.label("位深度：");
                                    let bd = bd_cb.get();
                                    let current = match bd {
                                        yinhe_audio::export::WavBitDepth::Bit16 => "16-bit",
                                        yinhe_audio::export::WavBitDepth::Bit24 => "24-bit",
                                        yinhe_audio::export::WavBitDepth::Bit32Float => {
                                            "32-bit float"
                                        }
                                    };
                                    egui::ComboBox::from_id_salt("export_bit_depth")
                                        .selected_text(current)
                                        .show_ui(ui, |ui| {
                                            if ui
                                                .selectable_label(
                                                    bd == yinhe_audio::export::WavBitDepth::Bit16,
                                                    "16-bit",
                                                )
                                                .clicked()
                                            {
                                                bd_cb.set(yinhe_audio::export::WavBitDepth::Bit16);
                                            }
                                            if ui
                                                .selectable_label(
                                                    bd == yinhe_audio::export::WavBitDepth::Bit24,
                                                    "24-bit",
                                                )
                                                .clicked()
                                            {
                                                bd_cb.set(yinhe_audio::export::WavBitDepth::Bit24);
                                            }
                                            if ui
                                                .selectable_label(
                                                    bd == yinhe_audio::export::WavBitDepth::Bit32Float,
                                                    "32-bit float",
                                                )
                                                .clicked()
                                            {
                                                bd_cb.set(
                                                    yinhe_audio::export::WavBitDepth::Bit32Float,
                                                );
                                            }
                                        });
                                });

                                ui.horizontal(|ui| {
                                    ui.label("采样率：");
                                    let r = sr_cb.get();
                                    let sr_text = if r == 0 {
                                        format!("跟随全局 ({} Hz)", sample_rate)
                                    } else {
                                        format!("{} Hz", r)
                                    };
                                    let sample_rates: [u32; 5] = [0, 44100, 48000, 96000, 192000];
                                    egui::ComboBox::from_id_salt("export_sample_rate")
                                        .selected_text(&sr_text)
                                        .show_ui(ui, |ui| {
                                            for &rate in &sample_rates {
                                                let label = if rate == 0 {
                                                    format!(
                                                        "跟随全局 ({} Hz)",
                                                        sample_rate
                                                    )
                                                } else {
                                                    format!("{} Hz", rate)
                                                };
                                                let selected = r == rate;
                                                if ui.selectable_label(selected, label).clicked() {
                                                    sr_cb.set(rate);
                                                }
                                            }
                                        });
                                });

                                ui.horizontal(|ui| {
                                    ui.label("XSynth层数：");
                                    let mut layers = lc_cb.get() as usize;
                                    ui.add(
                                        egui::DragValue::new(&mut layers)
                                            .range(0..=128)
                                            .speed(1.0),
                                    );
                                    lc_cb.set(layers as u32);
                                    if lc_cb.get() == 0 {
                                        ui.label("无限制");
                                    }
                                });

                                ui.add_space(12.0);

                                if ui.button("导出").clicked() {
                                    *started_cb.borrow_mut() = true;
                                    close = true;
                                }

                                ui.add_space(8.0);
                            });
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                *open_cb.borrow_mut() = false;
            }
        },
    );

    if !*open.borrow() {
        *show = false;
    }
    *bit_depth = bd.get();
    *layer_count = lc.get();
    *export_sample_rate = sr.get();

    *started.borrow()
}

pub(crate) fn format_duration(secs: f64) -> String {
    if secs < 0.0 {
        return "—".into();
    }
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}
