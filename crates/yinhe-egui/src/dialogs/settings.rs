use eframe::egui;

use crate::audio_settings::AudioSettings;

/// Show the settings dialog content inside an existing Ui.
/// Returns `true` if settings were changed.
pub fn show_content(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

    ui.heading("音频");
    ui.add_space(8.0);

    egui::Grid::new("audio_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("输出设备");
            let current_device =
                settings.output_device_name.as_deref().unwrap_or("默认设备");
            egui::ComboBox::from_id_salt("output_device")
                .selected_text(current_device)
                .show_ui(ui, |ui| {
                    for device_name in settings.available_devices().to_vec() {
                        let selected =
                            settings.output_device_name.as_ref() == Some(&device_name);
                        if ui.selectable_label(selected, &device_name).clicked() {
                            settings.output_device_name = Some(device_name);
                            changed = true;
                        }
                    }
                    let is_default = settings.output_device_name.is_none();
                    if ui.selectable_label(is_default, "默认设备").clicked() {
                        settings.output_device_name = None;
                        changed = true;
                    }
                });
            ui.end_row();

            ui.label("采样率");
            let sr_label = format!("{} Hz", settings.sample_rate);
            egui::ComboBox::from_id_salt("sample_rate")
                .selected_text(&sr_label)
                .show_ui(ui, |ui| {
                    for sr in settings.available_sample_rates().to_vec() {
                        let selected = settings.sample_rate == sr;
                        if ui
                            .selectable_label(selected, format!("{} Hz", sr))
                            .clicked()
                        {
                            settings.sample_rate = sr;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("缓冲区大小");
            let buf_sizes: &[(u32, &str)] = &[
                (0, "默认 (系统)"),
                (128, "128 帧"),
                (256, "256 帧"),
                (512, "512 帧"),
                (1024, "1024 帧"),
                (2048, "2048 帧"),
                (4096, "4096 帧"),
            ];
            let buf_label = buf_sizes
                .iter()
                .find(|(v, _)| *v == settings.buffer_size)
                .map(|(_, l)| *l)
                .unwrap_or("自定义");
            egui::ComboBox::from_id_salt("buffer_size")
                .selected_text(buf_label)
                .show_ui(ui, |ui| {
                    for &(val, label) in buf_sizes {
                        let selected = settings.buffer_size == val;
                        if ui.selectable_label(selected, label).clicked() {
                            settings.buffer_size = val;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("XSynth层数");
            let mut layers = settings.xsynth_layers as usize;
            if ui
                .add(
                    egui::DragValue::new(&mut layers)
                        .range(0..=128)
                        .speed(1.0),
                )
                .changed()
            {
                settings.xsynth_layers = layers as u32;
                changed = true;
            }
            let layer_label = if settings.xsynth_layers == 0 {
                "无限制"
            } else {
                ""
            };
            if !layer_label.is_empty() {
                ui.label(layer_label);
            }
            ui.end_row();

            ui.label("合成引擎");
            let engine_names = ["XSynth (CPU)", "Yinhe-Synth (GPU)"];
            let current_engine = if settings.use_gpu_synth { 1 } else { 0 };
            egui::ComboBox::from_id_salt("synth_engine")
                .selected_text(engine_names[current_engine])
                .show_ui(ui, |ui| {
                    for (i, name) in engine_names.iter().enumerate() {
                        let selected = (i == 1) == settings.use_gpu_synth;
                        if ui.selectable_label(selected, *name).clicked() {
                            settings.use_gpu_synth = i == 1;
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.heading("渲染");
    ui.add_space(8.0);

    egui::Grid::new("render_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("滚动模式");
            let mode_names = ["原始", "整数对齐", "子像素偏移"];
            let current = settings.scroll_mode as usize;
            egui::ComboBox::from_id_salt("scroll_mode")
                .selected_text(mode_names[current])
                .show_ui(ui, |ui| {
                    for (i, name) in mode_names.iter().enumerate() {
                        let selected = settings.scroll_mode == i as u32;
                        if ui.selectable_label(selected, *name).clicked() {
                            settings.scroll_mode = i as u32;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("Velocity显示");
            let vel_names = ["柱状", "矩形", "空心矩形"];
            let current_vel = settings.velocity_display_mode as usize;
            egui::ComboBox::from_id_salt("velocity_display_mode")
                .selected_text(vel_names[current_vel])
                .show_ui(ui, |ui| {
                    for (i, name) in vel_names.iter().enumerate() {
                        let selected = settings.velocity_display_mode == i as u32;
                        if ui.selectable_label(selected, *name).clicked() {
                            settings.velocity_display_mode = i as u32;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("自动化事件密度");
            let mut density = settings.automation_event_density as i32;
            let drag = ui.add(
                egui::DragValue::new(&mut density)
                    .range(1..=480)
                    .speed(0.2)
                    .suffix(" tick"),
            );
            if drag.changed() {
                settings.automation_event_density = density.max(1) as u32;
                changed = true;
            }
            ui.end_row();

            ui.label("选中音符变色");
            if ui.checkbox(&mut settings.note_selection_highlight, "").changed() {
                changed = true;
            }
            ui.end_row();

            ui.label("最小边框宽度");
            let mut bw = settings.min_border_width;
            if ui
                .add(egui::Slider::new(&mut bw, 0.0..=5.0).step_by(0.5))
                .changed()
            {
                settings.min_border_width = bw;
                changed = true;
            }
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.heading("MIDI 导入");
    ui.add_space(8.0);

    egui::Grid::new("midi_import_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("音轨名编码");
            egui::ComboBox::from_id_salt("midi_import_encoding")
                .selected_text(settings.midi_import_encoding.label())
                .show_ui(ui, |ui| {
                    for &enc in yinhe_mid2::MidiImportEncoding::ALL {
                        let selected = settings.midi_import_encoding == enc;
                        if ui.selectable_label(selected, enc.label()).clicked() {
                            settings.midi_import_encoding = enc;
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        if ui.button("刷新设备列表").clicked() {
            let devices = crate::audio_settings::list_output_devices();
            let (default_rate, rates) = crate::audio_settings::discover_sample_rates();
            settings.refresh_devices(devices, rates, default_rate);
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.heading("触控板震动");
    ui.add_space(8.0);

    egui::Grid::new("haptic_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("启用震动");
            if ui.checkbox(&mut settings.haptic_enabled, "").changed() {
                changed = true;
            }
            ui.end_row();

            ui.label("震动强度");
            let mut intensity = settings.haptic_intensity;
            if ui
                .add(egui::Slider::new(&mut intensity, 0.0..=1.0).step_by(0.05))
                .changed()
            {
                settings.haptic_intensity = intensity;
                changed = true;
            }
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    // ── Factory reset button ──
    ui.horizontal(|ui| {
        ui.add_space(ui.available_width() / 2.0 - 80.0);
        if ui
            .button(egui::RichText::new("恢复出厂设置").color(egui::Color32::from_rgb(232, 80, 80)))
            .clicked()
        {
            let default_settings = AudioSettings::default();
            // Preserve runtime fields (device lists, etc.)
            let devices = std::mem::take(&mut settings.available_devices);
            let rates = std::mem::take(&mut settings.available_sample_rates);
            *settings = default_settings;
            settings.available_devices = devices;
            settings.available_sample_rates = rates;
            changed = true;
        }
    });

    changed
}

pub(crate) fn show_viewport(
    ctx: &eframe::egui::Context,
    settings: &mut AudioSettings,
    haptic_engine: &mut yinhe_haptic::HapticEngine,
    audio: &Option<yinhe_audio::CpalAudioHandle>,
) -> bool {
    if !settings.show_settings {
        return false;
    }

    let prev_xsynth_layers = settings.xsynth_layers;
    let settings_rc = std::rc::Rc::new(std::cell::RefCell::new(Some(std::mem::take(settings))));
    let ctx_clone = ctx.clone();
    let settings_cb = settings_rc.clone();

    ctx_clone.show_viewport_immediate(
        eframe::egui::ViewportId::from_hash_of("settings_dialog"),
        crate::chrome::dialog::viewport_builder("设置", [480.0, 520.0], true),
        move |vctx, _class| {
            let mut slot = settings_cb.borrow_mut().take();
            if let Some(ref mut s) = slot {
                let mut close = false;
                if vctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                eframe::egui::CentralPanel::default()
                    .frame(eframe::egui::Frame {
                        fill: crate::theme::APP_BG,
                        ..Default::default()
                    })
                    .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(ui, "设置", &mut close);
                        eframe::egui::Frame::new()
                            .inner_margin(eframe::egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                eframe::egui::ScrollArea::vertical()
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        let changed = show_content(ui, s);
                                        if changed {
                                            s.save();
                                        }
                                    });
                            });
                    });
                if close {
                    vctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                    s.show_settings = false;
                }
            }
            *settings_cb.borrow_mut() = slot;
        },
    );

    let should_teardown = if let Some(s) = std::rc::Rc::into_inner(settings_rc)
        .and_then(|rc| rc.into_inner())
    {
        *settings = s;
        haptic_engine.apply_settings(
            settings.haptic_enabled,
            settings.haptic_intensity,
        );
        if settings.xsynth_layers != prev_xsynth_layers {
            if let Some(audio) = audio {
                let count = if settings.xsynth_layers == 0 {
                    None
                } else {
                    Some(settings.xsynth_layers as usize)
                };
                audio.handle.send(yinhe_audio::AudioCommand::SetLayerCount { count });
            }
        }
        !settings.show_settings
    } else {
        false
    };

    should_teardown
}
