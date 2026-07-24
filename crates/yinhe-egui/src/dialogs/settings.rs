use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;

/// Show the settings dialog content inside an existing Ui.
/// Returns `true` if settings were changed.
pub fn show_content(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

    ui.heading(t!("settings.language").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("language_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.language").as_ref());
            let locales = [("zh-CN", "简体中文"), ("en", "English"), ("ja", "日本語"), ("ko", "한국어")];
            let current = locales
                .iter()
                .find(|(code, _)| *code == settings.locale)
                .map(|(_, name)| *name)
                .unwrap_or("简体中文");
            egui::ComboBox::from_id_salt("locale_select")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (code, name) in locales {
                        let selected = settings.locale == code;
                        if ui.selectable_label(selected, name).clicked() {
                            settings.locale = code.to_string();
                            rust_i18n::set_locale(code);
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.heading(t!("settings.audio.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("audio_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.audio.output_device").as_ref());
            let default_device = t!("settings.audio.default_device").to_string();
            let current_device =
                settings.output_device_name.as_deref().unwrap_or(default_device.as_str());
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
                    if ui.selectable_label(is_default, t!("settings.audio.default_device").as_ref()).clicked() {
                        settings.output_device_name = None;
                        changed = true;
                    }
                });
            ui.end_row();

            ui.label(t!("settings.audio.sample_rate").as_ref());
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

            ui.label(t!("settings.audio.buffer_size").as_ref());
            let buf_sizes: &[(u32, String)] = &[
                (0, t!("settings.audio.buffer.default").to_string()),
                (128, t!("settings.audio.buffer.frames", n = 128).to_string()),
                (256, t!("settings.audio.buffer.frames", n = 256).to_string()),
                (512, t!("settings.audio.buffer.frames", n = 512).to_string()),
                (1024, t!("settings.audio.buffer.frames", n = 1024).to_string()),
                (2048, t!("settings.audio.buffer.frames", n = 2048).to_string()),
                (4096, t!("settings.audio.buffer.frames", n = 4096).to_string()),
            ];
            let custom_buf = t!("settings.audio.buffer.custom").to_string();
            let buf_label = buf_sizes
                .iter()
                .find(|(v, _)| *v == settings.buffer_size)
                .map(|(_, l)| l.as_str())
                .unwrap_or(custom_buf.as_str());
            egui::ComboBox::from_id_salt("buffer_size")
                .selected_text(buf_label)
                .show_ui(ui, |ui| {
                    for &(val, ref label) in buf_sizes {
                        let selected = settings.buffer_size == val;
                        if ui.selectable_label(selected, label).clicked() {
                            settings.buffer_size = val;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label(t!("settings.audio.xsynth_layers").as_ref());
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
                t!("common.unlimited").to_string()
            } else {
                String::new()
            };
            if !layer_label.is_empty() {
                ui.label(layer_label);
            }
            ui.end_row();

            ui.label(t!("settings.audio.synth_engine").as_ref());
            let engine_names = [t!("settings.audio.engine_cpu").to_string(), t!("settings.audio.engine_gpu").to_string()];
            let current_engine = if settings.use_gpu_synth { 1 } else { 0 };
            egui::ComboBox::from_id_salt("synth_engine")
                .selected_text(engine_names[current_engine].clone())
                .show_ui(ui, |ui| {
                    for (i, name) in engine_names.iter().enumerate() {
                        let selected = (i == 1) == settings.use_gpu_synth;
                        if ui.selectable_label(selected, name).clicked() {
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

    ui.heading(t!("settings.render.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("render_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.render.scroll_mode").as_ref());
            let mode_names = [t!("settings.render.scroll.raw").to_string(), t!("settings.render.scroll.integer").to_string(), t!("settings.render.scroll.subpixel").to_string()];
            let current = settings.scroll_mode as usize;
            egui::ComboBox::from_id_salt("scroll_mode")
                .selected_text(mode_names[current].clone())
                .show_ui(ui, |ui| {
                    for (i, name) in mode_names.iter().enumerate() {
                        let selected = settings.scroll_mode == i as u32;
                        if ui.selectable_label(selected, name).clicked() {
                            settings.scroll_mode = i as u32;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label(t!("settings.render.automation_density").as_ref());
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

            ui.label(t!("settings.render.note_outline").as_ref());
            if ui.checkbox(&mut settings.note_outline, "").changed() {
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.render.min_border_width").as_ref());
            let mut bw = settings.min_border_width;
            if ui
                .add(egui::Slider::new(&mut bw, 0.0..=5.0).step_by(0.5))
                .changed()
            {
                settings.min_border_width = bw;
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.render.gpu_cull").as_ref());
            if ui.checkbox(&mut settings.use_gpu_cull, "").changed() {
                changed = true;
            }
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.heading(t!("settings.midi_import.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("midi_import_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.midi_import.encoding").as_ref());
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
        if ui.button(t!("settings.refresh_devices").as_ref()).clicked() {
            let devices = crate::audio_settings::list_output_devices();
            let (default_rate, rates) = crate::audio_settings::discover_sample_rates();
            settings.refresh_devices(devices, rates, default_rate);
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.heading(t!("settings.haptic.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("haptic_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.haptic.enabled").as_ref());
            if ui.checkbox(&mut settings.haptic_enabled, "").changed() {
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.haptic.intensity").as_ref());
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
            .button(egui::RichText::new(t!("settings.factory_reset").as_ref()).color(egui::Color32::from_rgb(232, 80, 80)))
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
    let viewport_id = eframe::egui::ViewportId::from_hash_of("settings_dialog");
    if !settings.show_settings {
        return false;
    }

    let prev_xsynth_layers = settings.xsynth_layers;
    let settings_rc = std::rc::Rc::new(std::cell::RefCell::new(Some(std::mem::take(settings))));
    let ctx_clone = ctx.clone();
    let settings_cb = settings_rc.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(t!("settings.title").as_ref(), [480.0, 560.0], true),
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
                        crate::chrome::dialog::title_bar(ui, t!("settings.title").as_ref(), &mut close);
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
