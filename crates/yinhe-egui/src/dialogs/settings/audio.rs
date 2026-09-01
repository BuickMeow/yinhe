use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;

pub fn show_audio_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.audio.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("audio_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.audio.output_device").as_ref());
            let default_device = t!("settings.audio.default_device").to_string();
            let current_device = settings
                .output_device_name
                .clone()
                .unwrap_or(default_device);
            crate::widgets::combo::combo_box(ui, "output_device", current_device, 200.0, |ui| {
                for device_name in settings.available_devices().to_vec() {
                    let selected = settings.output_device_name.as_ref() == Some(&device_name);
                    if crate::widgets::combo::combo_item(ui, selected, &device_name).clicked() {
                        settings.output_device_name = Some(device_name);
                        changed = true;
                    }
                }
                let is_default = settings.output_device_name.is_none();
                if crate::widgets::combo::combo_item(
                    ui,
                    is_default,
                    t!("settings.audio.default_device").as_ref(),
                )
                .clicked()
                {
                    settings.output_device_name = None;
                    changed = true;
                }
            });
            ui.end_row();

            // ── MIDI 输入 ──
            ui.label(t!("settings.audio.midi_input_device").as_ref());
            let no_device = t!("settings.audio.midi_no_device").to_string();
            let current_midi = settings.midi_input_device.clone().unwrap_or(no_device);
            crate::widgets::combo::combo_box(ui, "midi_input_device", current_midi, 200.0, |ui| {
                for device_name in settings.available_midi_inputs.clone() {
                    let selected = settings.midi_input_device.as_ref() == Some(&device_name);
                    if crate::widgets::combo::combo_item(ui, selected, &device_name).clicked() {
                        settings.midi_input_device = Some(device_name);
                        changed = true;
                    }
                }
                let is_none = settings.midi_input_device.is_none();
                if crate::widgets::combo::combo_item(
                    ui,
                    is_none,
                    t!("settings.audio.midi_no_device").as_ref(),
                )
                .clicked()
                {
                    settings.midi_input_device = None;
                    changed = true;
                }
            });
            ui.end_row();

            ui.label(t!("settings.audio.midi_thru").as_ref());
            if crate::widgets::checkbox::checkbox(
                ui,
                &mut settings.midi_thru,
                t!("settings.audio.midi_thru_hint").as_ref(),
            )
            .changed()
            {
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.audio.sample_rate").as_ref());
            let sr_opt: Vec<(u32, String)> = settings
                .available_sample_rates()
                .iter()
                .map(|&sr| (sr, format!("{} Hz", sr)))
                .collect();
            if crate::widgets::combo::combo_select(
                ui,
                "sample_rate",
                &mut settings.sample_rate,
                160.0,
                &sr_opt,
            ) {
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.audio.buffer_size").as_ref());
            let buf_sizes: &[(u32, String)] = &[
                (0, t!("settings.audio.buffer.default").to_string()),
                (128, t!("settings.audio.buffer.frames", n = 128).to_string()),
                (256, t!("settings.audio.buffer.frames", n = 256).to_string()),
                (512, t!("settings.audio.buffer.frames", n = 512).to_string()),
                (
                    1024,
                    t!("settings.audio.buffer.frames", n = 1024).to_string(),
                ),
                (
                    2048,
                    t!("settings.audio.buffer.frames", n = 2048).to_string(),
                ),
                (
                    4096,
                    t!("settings.audio.buffer.frames", n = 4096).to_string(),
                ),
            ];
            let buf_opt: Vec<(u32, String)> =
                buf_sizes.iter().map(|(v, l)| (*v, l.clone())).collect();
            if crate::widgets::combo::combo_select(
                ui,
                "buffer_size",
                &mut settings.buffer_size,
                160.0,
                &buf_opt,
            ) {
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.audio.xsynth_layers").as_ref());
            let mut layers = settings.xsynth_layers as usize;
            if ui
                .add(
                    crate::widgets::numeric_input::decimal_drag_value(&mut layers)
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
            let engine_opt = vec![
                (false, t!("settings.audio.engine_cpu").to_string()),
                (true, t!("settings.audio.engine_gpu").to_string()),
            ];
            if crate::widgets::combo::combo_select(
                ui,
                "synth_engine",
                &mut settings.use_gpu_synth,
                160.0,
                &engine_opt,
            ) {
                changed = true;
            }
            ui.end_row();
        });

    // 设备列表变更（热插拔等）后手动刷新
    if ui.button(t!("settings.refresh_devices").as_ref()).clicked() {
        let devices = crate::audio_settings::list_output_devices();
        let (default_rate, rates) = crate::audio_settings::discover_sample_rates();
        settings.refresh_devices(devices, rates, default_rate);
        crate::audio_settings::refresh_midi_inputs(settings);
        changed = true;
    }
    changed
}
