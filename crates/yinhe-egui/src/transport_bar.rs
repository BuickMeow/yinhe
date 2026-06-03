use eframe::egui;

use crate::document::Document;
use crate::file_loader::FileLoader;
use crate::time_format;

pub fn show(
    ui: &mut egui::Ui,
    file_loader: &mut FileLoader,
    toggle_play: &mut bool,
    pause_return: &mut bool,
    stop_play: &mut bool,
    doc: Option<&Document>,
    cpu_usage: f32,
    mem_mb: f64,
) {
    let has_active = doc.is_some();

    egui::Panel::top("transport_bar")
        .frame(egui::Frame {
            fill: egui::Color32::from_rgb(25, 25, 28),
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 0,
                bottom: 8,
            },
            stroke: egui::Stroke::NONE,
            ..Default::default()
        })
        .show_inside(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let open_btn =
                    ui.add_enabled(!file_loader.is_loading(), egui::Button::new("Open MIDI"));
                if open_btn.clicked() {
                    file_loader.pick_midi_file();
                }

                if has_active {
                    let is_playing = doc.map(|d| d.playback.is_playing()).unwrap_or(false);
                    if ui.button(if is_playing { "⏸" } else { "▶" }).clicked() {
                        if is_playing {
                            *pause_return = true;
                        } else {
                            *toggle_play = true;
                        }
                    }
                    if ui.button("⏹").clicked() {
                        *stop_play = true;
                    }
                }

                if let Some(doc) = doc {
                    let tick = doc.cursor_tick.unwrap_or(0.0);
                    let tick_u = tick as u32;
                    let seconds = doc.midi.tick_to_seconds(tick_u);
                    let bpm = doc.midi.bpm_at_time(seconds);
                    let (num, _denom_power) = doc.midi.time_sig_at_tick(tick_u);
                    let ppq = doc.midi.ticks_per_beat;

                    let bpm_str = time_format::format_bpm(bpm);
                    let ts_str = time_format::format_time_sig(num, _denom_power);
                    let time_str = time_format::format_time(seconds);
                    let pos_str = time_format::format_tick_bar_beat(tick, ppq, num);

                    let col_widths = [70.0, 76.0, 90.0];
                    let rect_h = 36.0;
                    let rect_w = col_widths.iter().sum::<f32>();
                    let bar_cx = ui.max_rect().center().x;
                    let cursor_x = ui.cursor().min.x;
                    let rect_l = bar_cx - rect_w * 0.5;
                    let pad = (rect_l - cursor_x).max(0.0);
                    ui.add_space(pad);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(rect_w, rect_h), egui::Sense::hover());

                    let c = egui::Color32::from_rgb(100, 180, 255);
                    let font = egui::FontId::proportional(12.0);
                    let grid = egui::Stroke::new(1.0, egui::Color32::from_gray(60));

                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(8),
                        egui::Color32::BLACK,
                    );

                    let mut col_x = rect.min.x;
                    for i in 0..3 {
                        let cx = col_x + col_widths[i] * 0.5;
                        if i > 0 {
                            ui.painter().line_segment(
                                [egui::pos2(col_x, rect.min.y), egui::pos2(col_x, rect.max.y)],
                                grid,
                            );
                        }
                        let text = match i {
                            0 => format!("{:.1}%", cpu_usage),
                            1 => bpm_str.clone(),
                            2 => pos_str.clone(),
                            _ => unreachable!(),
                        };
                        ui.painter().text(
                            egui::pos2(cx, rect.min.y + rect_h * 0.25),
                            egui::Align2::CENTER_CENTER,
                            text,
                            font.clone(),
                            c,
                        );
                        let text2 = match i {
                            0 => format!("{:.1} MB", mem_mb),
                            1 => ts_str.clone(),
                            2 => time_str.clone(),
                            _ => unreachable!(),
                        };
                        ui.painter().text(
                            egui::pos2(cx, rect.min.y + rect_h * 0.75),
                            egui::Align2::CENTER_CENTER,
                            text2,
                            font.clone(),
                            c,
                        );
                        col_x += col_widths[i];
                    }
                }
            });
        });
}
