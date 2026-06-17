use eframe::egui;

use crate::audio_settings::AudioSettings;

pub fn show(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    if !settings.show_settings {
        return false;
    }

    let mut changed = false;
    let mut should_close = false;

    let resp = egui::Window::new("设置")
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .default_height(400.0)
        .show(ui.ctx(), |ui| {
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

                    ui.label("自动化显示");
                    let auto_names = ["柱状", "折线"];
                    let current_auto = settings.automation_display_mode as usize;
                    egui::ComboBox::from_id_salt("automation_display_mode")
                        .selected_text(auto_names[current_auto])
                        .show_ui(ui, |ui| {
                            for (i, name) in auto_names.iter().enumerate() {
                                let selected = settings.automation_display_mode == i as u32;
                                if ui.selectable_label(selected, *name).clicked() {
                                    settings.automation_display_mode = i as u32;
                                    changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    ui.label("折线圆点");
                    if ui.checkbox(&mut settings.automation_show_dots, "").changed() {
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
                if ui.button("关闭").clicked() {
                    should_close = true;
                }
            });
        });

    if should_close || resp.as_ref().is_some_and(|r| r.response.should_close()) {
        settings.show_settings = false;
    }

    if changed {
        settings.save();
    }

    changed
}
