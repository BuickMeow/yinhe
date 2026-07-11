use eframe::egui;
use egui_material_icons::icons::*;

use crate::app::App;

impl App {
    /// Show all overlay dialogs as independent OS windows.
    pub(in crate::app) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── Settings dialog (independent window) ──
        if self.audio_settings.show_settings {
            let prev_xsynth_layers = self.audio_settings.xsynth_layers;
            let settings = std::rc::Rc::new(std::cell::RefCell::new(Some(std::mem::take(&mut self.audio_settings))));
            let ctx_clone = ctx.clone();
            let settings_cb = settings.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("settings_dialog"),
                crate::chrome::dialog::viewport_builder("设置", [480.0, 520.0], true),
                move |vctx, _class| {
                    let mut slot = settings_cb.borrow_mut().take();
                    if let Some(ref mut s) = slot {
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
                            crate::chrome::dialog::title_bar(ui, "设置", &mut close);
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: 12,
                                    right: 12,
                                    top: 0,
                                    bottom: 12,
                                })
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false; 2])
                                        .show(ui, |ui| {
                                            let changed = crate::dialogs::settings::show_content(ui, s);
                                            if changed {
                                                s.save();
                                            }
                                        });
                                });
                        });
                        if close {
                            // Hide window before closing to prevent white flash
                            vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            s.show_settings = false;
                            // Keep slot = Some(s) — don't drop the settings
                        }
                    }
                    *settings_cb.borrow_mut() = slot;
                },
            );

            if let Some(s) = std::rc::Rc::into_inner(settings)
                .and_then(|rc| rc.into_inner())
            {
                self.audio_settings = s;
                // Sync haptic settings to the engine
                self.haptic_engine.apply_settings(
                    self.audio_settings.haptic_enabled,
                    self.audio_settings.haptic_intensity,
                );
                // Apply XSynth layer count change immediately so the user
                // can hear the difference without closing the dialog.
                if self.audio_settings.xsynth_layers != prev_xsynth_layers {
                    if let Some(ref audio) = self.audio {
                        let count = if self.audio_settings.xsynth_layers == 0 {
                            None
                        } else {
                            Some(self.audio_settings.xsynth_layers as usize)
                        };
                        audio.handle.send(yinhe_audio::AudioCommand::SetLayerCount { count });
                    }
                }
                if !self.audio_settings.show_settings {
                    self.teardown_audio();
                }
            }
        }

        // ── Memory breakdown (independent window) ──
        if self.show_mem_breakdown {
            let snapshot = yinhe_memtrace::Snapshot::capture();
            let mem_mb = self.sys_monitor.mem_mb;

            #[cfg(target_os = "macos")]
            let metal_size = self
                .render_ctx
                .metal_allocated_size()
                .unwrap_or(0)
                .saturating_add(self.arr_render_ctx.metal_allocated_size().unwrap_or(0));

            let open = std::rc::Rc::new(std::cell::RefCell::new(true));
            let ctx_clone = ctx.clone();
            let open_cb = open.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("memory_breakdown_dialog"),
                crate::chrome::dialog::viewport_builder(
                    "内存占用详情",
                    crate::theme::MEM_POPUP_SIZE,
                    false,
                ),
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
                        crate::chrome::dialog::title_bar(
                            ui,
                            "内存占用详情",
                            &mut close,
                        );
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        ui.label(format!("系统统计总内存: {:.1} MB", mem_mb));
                                        ui.label(format!("分配器追踪内存: {:.1} MB", snapshot.total_mb()));
                                        ui.label(format!("wgpu 显式 GPU 资源: {:.1} MB", snapshot.gpu_mb()));

                                        #[cfg(target_os = "macos")]
                                        ui.label(format!(
                                            "Metal 驱动真实显存: {:.1} MB",
                                            metal_size as f64 / 1_048_576.0
                                        ));

                                        ui.separator();

                                        ui.heading("按子系统分类");
                                        egui::Grid::new("mem_breakdown_grid")
                                            .num_columns(2)
                                            .spacing([12.0, 8.0])
                                            .show(ui, |ui| {
                                                for tag in yinhe_memtrace::AllocTag::ALL {
                                                    if tag == yinhe_memtrace::AllocTag::Unknown
                                                        && snapshot.get(tag) <= 0
                                                    {
                                                        continue;
                                                    }
                                                    ui.label(tag.name());
                                                    ui.label(format!("{:.1} MB", snapshot.mb(tag)));
                                                    ui.end_row();
                                                }
                                            });

                                        ui.separator();
                                        ui.small(
                                            "注：GPU 资源计数反映应用显式创建的 wgpu Texture/Buffer 大小；\
                                             驱动层额外开销（swapchain、depth、pipeline cache 等）\
                                             不纳入此项统计。",
                                        );

                                        if ui.button("关闭").clicked() {
                                            close = true;
                                        }
                                    });
                            });
                    });
                    if close {
                        // Hide window before closing to prevent white flash
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        *open_cb.borrow_mut() = false;
                    }
                },
            );

            if !*open.borrow() {
                self.show_mem_breakdown = false;
            }
        }

        // ── Loading overlay (independent window) ──
        if self.file_loader.is_loading() {
            let progress = self.file_loader.load_progress().clone();
            let ctx_clone = ctx.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("loading_overlay_dialog"),
                crate::chrome::dialog::viewport_builder("正在加载", [380.0, 160.0], false),
                move |vctx, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        let mut close = false;
                        crate::chrome::dialog::title_bar(ui, "正在加载", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                let p = match progress.lock() {
                                    Ok(p) => p.clone(),
                                    Err(_) => return,
                                };
                                if !p.visible {
                                    return;
                                }
                                for stage in &p.stages {
                                    ui.horizontal(|ui| {
                                        let icon = match stage.status {
                                            yinhe_editor_core::progress::StageStatus::Done => ICON_CHECK_CIRCLE,
                                            yinhe_editor_core::progress::StageStatus::Active => ICON_SYNC,
                                            yinhe_editor_core::progress::StageStatus::Pending => ICON_RADIO_BUTTON_UNCHECKED,
                                        };
                                        ui.label(icon.rich_text().size(14.0));
                                        ui.add(
                                            egui::ProgressBar::new(stage.progress)
                                                .desired_width(200.0)
                                                .show_percentage(),
                                        );
                                        ui.label(egui::RichText::new(&stage.label).size(12.0));
                                    });
                                    if !stage.detail.is_empty() {
                                        ui.label(
                                            egui::RichText::new(&stage.detail)
                                                .size(10.0)
                                                .color(egui::Color32::GRAY),
                                        );
                                    }
                                }
                            });
                    });
                },
            );

            ctx.request_repaint();
        }

        // ── Archive picker dialog (independent window) ──
        if let Some(ref mut state) = self.file_loader.archive_picker {
            use crate::dialogs::archive_picker;

            let taken_state = std::rc::Rc::new(std::cell::RefCell::new(
                std::mem::replace(state, archive_picker::ArchivePickerState::Opening {
                    path: String::new(),
                    rx: std::sync::mpsc::channel().1,
                })
            ));
            let action = std::rc::Rc::new(std::cell::RefCell::new(archive_picker::ArchivePickerAction::None));
            let ctx_clone = ctx.clone();
            let taken_state_cb = taken_state.clone();
            let action_cb = action.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("archive_picker_dialog"),
                crate::chrome::dialog::viewport_builder("选择 MIDI 文件", [500.0, 400.0], true),
                move |vctx, _class| {
                    let close_requested = vctx.input(|i| i.viewport().close_requested());
                    let vctx_cmd = vctx.clone();
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                            let mut close = close_requested;
                            crate::chrome::dialog::title_bar(ui, "选择 MIDI 文件", &mut close);
                            if close {
                                // Hide window before closing to prevent white flash
                                vctx_cmd.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                                *action_cb.borrow_mut() = archive_picker::ArchivePickerAction::Cancel;
                            } else {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin {
                                        left: 12,
                                        right: 12,
                                        top: 0,
                                        bottom: 12,
                                    })
                                    .show(ui, |ui| {
                                        let result = archive_picker::show(
                                            &mut *taken_state_cb.borrow_mut(),
                                            ui,
                                        );
                                        *action_cb.borrow_mut() = result;
                                    });
                            }
                        });
                },
            );

            if let Some(taken_state) = std::rc::Rc::into_inner(taken_state) {
                *state = taken_state.into_inner();
            }

            if let Some(action) = std::rc::Rc::into_inner(action) {
                match action.into_inner() {
                    archive_picker::ArchivePickerAction::LoadFile { archive, entry } => {
                        self.file_loader.start_load_from_archive(archive, entry);
                        self.file_loader.archive_picker = None;
                    }
                    archive_picker::ArchivePickerAction::Cancel => {
                        self.file_loader.archive_picker = None;
                    }
                    archive_picker::ArchivePickerAction::Error(msg) => {
                        self.load_error = Some(msg);
                        self.file_loader.archive_picker = None;
                    }
                    archive_picker::ArchivePickerAction::None => {}
                }
                ctx.request_repaint();
            }
        }

        // ── Export progress overlay (independent window) ──
        if self.export_rx.is_some() {
            let export_progress = self.export_progress.clone();
            let ctx_clone = ctx.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("export_progress_dialog"),
                crate::chrome::dialog::viewport_builder("导出音频", [300.0, 160.0], false),
                move |vctx, _class| {
                    let state = match export_progress.lock() {
                        Ok(s) => s.clone(),
                        Err(_) => return,
                    };
                    if !state.visible {
                        return;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        let mut close = false;
                        crate::chrome::dialog::title_bar(ui, "导出音频", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new("导出音频中…")
                                            .size(18.0)
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.add_space(12.0);
                                    ui.add(
                                        egui::ProgressBar::new(state.progress)
                                            .desired_width(240.0)
                                            .fill(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
                                    );
                                    ui.add_space(8.0);
                                    if !state.status.is_empty() {
                                        ui.label(
                                            egui::RichText::new(&state.status)
                                                .size(13.0)
                                                .color(egui::Color32::LIGHT_GRAY),
                                        );
                                    }
                                });
                            });
                    });
                },
            );

            ctx.request_repaint();
        }

        // ── Export settings dialog (independent window) ──
        if self.show_export_bit_depth {
            let global_sr = self.audio_settings.sample_rate;
            // Use Rc<Cell> so the move closure can write settings back
            let bit_depth = std::rc::Rc::new(std::cell::Cell::new(self.export_bit_depth));
            let layer_count = std::rc::Rc::new(std::cell::Cell::new(self.export_layer_count));
            let sample_rate = std::rc::Rc::new(std::cell::Cell::new(self.export_sample_rate));
            let open = std::rc::Rc::new(std::cell::RefCell::new(true));
            let started = std::rc::Rc::new(std::cell::RefCell::new(false));
            let ctx_clone = ctx.clone();
            let open_cb = open.clone();
            let started_cb = started.clone();
            let bd_cb = bit_depth.clone();
            let lc_cb = layer_count.clone();
            let sr_cb = sample_rate.clone();

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
                                            yinhe_audio::export::WavBitDepth::Bit32Float => "32-bit float",
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
                                                    bd_cb.set(yinhe_audio::export::WavBitDepth::Bit32Float);
                                                }
                                            });
                                    });

                                    ui.horizontal(|ui| {
                                        ui.label("采样率：");
                                        let sr = sr_cb.get();
                                        let sr_text = if sr == 0 {
                                            format!("跟随全局 ({} Hz)", global_sr)
                                        } else {
                                            format!("{} Hz", sr)
                                        };
                                        let sample_rates: [u32; 5] = [0, 44100, 48000, 96000, 192000];
                                        egui::ComboBox::from_id_salt("export_sample_rate")
                                            .selected_text(&sr_text)
                                            .show_ui(ui, |ui| {
                                                for &rate in &sample_rates {
                                                    let label = if rate == 0 {
                                                        format!("跟随全局 ({} Hz)", global_sr)
                                                    } else {
                                                        format!("{} Hz", rate)
                                                    };
                                                    let selected = sr == rate;
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
                        // Hide window before closing to prevent white flash
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        *open_cb.borrow_mut() = false;
                    }
                },
            );

            if !*open.borrow() {
                self.show_export_bit_depth = false;
            }
            self.export_bit_depth = bit_depth.get();
            self.export_layer_count = layer_count.get();
            self.export_sample_rate = sample_rate.get();

            if *started.borrow() {
                self.start_export();
            }
        }
    }

    /// Show the load-error modal as an independent window.
    pub(in crate::app) fn show_load_error_modal(&mut self, ui: &mut egui::Ui) {
        if self.load_error.is_some() {
            let msg = self.load_error.clone().unwrap_or_default();
            let open = std::rc::Rc::new(std::cell::RefCell::new(true));
            let ctx = ui.ctx().clone();
            let open_cb = open.clone();

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("load_error_dialog"),
                crate::chrome::dialog::viewport_builder("无法打开文件", [420.0, 120.0], false),
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
                        crate::chrome::dialog::title_bar(ui, "无法打开文件", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.label(&msg);
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() {
                                        close = true;
                                    }
                                });
                            });
                    });
                    if close {
                        // Hide window before closing to prevent white flash
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        *open_cb.borrow_mut() = false;
                    }
                },
            );

            if !*open.borrow() {
                self.load_error = None;
            }
        }

        // ── Unsaved changes confirmation (independent window) ──
        if self.pending_unsaved.is_some() && self.save_rx.is_none() {
            let ctx = ui.ctx().clone();
            let ctx_clone = ctx.clone();

            // Use Rc<RefCell> to communicate the button action back from the closure
            let action_rc: std::rc::Rc<std::cell::RefCell<Option<UnsavedDialogAction>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
            let action_cb = action_rc.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("unsaved_dialog"),
                crate::chrome::dialog::viewport_builder("尚未保存", [380.0, 170.0], false),
                move |vctx, _class| {
                    let mut close = false;
                    if vctx.input(|i| i.viewport().close_requested()) {
                        *action_cb.borrow_mut() = Some(UnsavedDialogAction::Cancel);
                        close = true;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                            crate::chrome::dialog::title_bar(ui, "尚未保存", &mut close);
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: 12,
                                    right: 12,
                                    top: 0,
                                    bottom: 12,
                                })
                                .show(ui, |ui| {
                                    ui.set_max_width(360.0);
                                    ui.label("当前工程尚未保存，是否保存？");
                                    ui.add_space(16.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .button("保存")
                                            .clicked()
                                        {
                                            *action_cb.borrow_mut() =
                                                Some(UnsavedDialogAction::Save);
                                            close = true;
                                        }
                                        ui.add_space(8.0);
                                        let discard_btn =
                                            ui.button(egui::RichText::new("不保存").color(egui::Color32::from_rgb(255, 80, 80)));
                                        if discard_btn.clicked() {
                                            *action_cb.borrow_mut() =
                                                Some(UnsavedDialogAction::Discard);
                                            close = true;
                                        }
                                        ui.add_space(8.0);
                                        if ui.button("返回").clicked() {
                                            *action_cb.borrow_mut() =
                                                Some(UnsavedDialogAction::Cancel);
                                            close = true;
                                        }
                                    });
                                });
                        });
                    if close {
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    }
                },
            );

            // Process the button action outside the closure
            if let Some(action) = action_rc.borrow_mut().take() {
                match action {
                    UnsavedDialogAction::Save => {
                        if let Some(idx) = self.active_doc {
                            if let Some(path) = self.documents[idx].file_path.clone() {
                                self.save_project_async(idx, path);
                            } else {
                                self.save_as_dialog();
                            }
                        }
                        // pending_unsaved stays — will be executed after save completes
                    }
                    UnsavedDialogAction::Discard => {
                        let ctx = ui.ctx().clone();
                        self.execute_pending_file_action(&ctx);
                    }
                    UnsavedDialogAction::Cancel => {
                        self.pending_unsaved = None;
                    }
                }
            }
        }
    }
}

/// Internal: which button was pressed in the unsaved dialog.
enum UnsavedDialogAction {
    Save,
    Discard,
    Cancel,
}
