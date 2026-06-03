use eframe::egui;

use crate::document::Document;
use crate::file_loader::FileLoader;
use crate::time_format;

pub fn show(
    ui: &mut egui::Ui,
    file_loader: &mut FileLoader,
    toggle_play: &mut bool,
    stop_play: &mut bool,
    doc: Option<&Document>,
    cpu_usage: f32,
    mem_mb: f64,
) {
    let has_active = doc.is_some();

    egui::Panel::top("transport_bar")
        .frame(egui::Frame {
            fill: egui::Color32::from_rgb(25, 25, 28),
            inner_margin: egui::Margin::symmetric(8, 4),
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
                    ui.separator();
                    let is_playing = doc.map(|d| d.playback.is_playing()).unwrap_or(false);
                    let play_label = if is_playing { "Pause" } else { "Play" };
                    if ui.button(play_label).clicked() {
                        *toggle_play = true;
                    }
                    if ui.button("Stop").clicked() {
                        *stop_play = true;
                    }
                }

                ui.separator();

                if let Some(doc) = doc {
                    ui.label(egui::RichText::new(&doc.file_name).strong());
                }

                if let Some(doc) = doc {
                    ui.separator();

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

                    let rect_h = 36.0;
                    let rect_w = ui.available_width().min(380.0);
                    let bar_cx = ui.max_rect().center().x;
                    let cursor_x = ui.cursor().min.x;
                    let rect_cx = bar_cx;
                    let rect_l = rect_cx - rect_w * 0.5;
                    let pad = (rect_l - cursor_x).max(0.0);
                    ui.add_space(pad);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(rect_w, rect_h), egui::Sense::hover());

                    let c = egui::Color32::from_rgb(100, 180, 255);
                    let font = egui::FontId::proportional(12.0);
                    let grid = egui::Stroke::new(1.0, egui::Color32::from_gray(60));

                    ui.painter().rect_filled(rect, egui::CornerRadius::same(8), egui::Color32::BLACK);

                    let col_w = rect.width() / 3.0;
                    for i in 1..3 {
                        let x = rect.min.x + col_w * i as f32;
                        ui.painter().line_segment(
                            [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                            grid,
                        );
                    }

                    let cpu_str = format!("{:.1}%", cpu_usage);
                    let mem_str = format!("{:.1} MB", mem_mb);

                    ui.painter().text(
                        egui::pos2(rect.min.x + col_w * 0.5, rect.min.y + rect_h * 0.25),
                        egui::Align2::CENTER_CENTER,
                        cpu_str,
                        font.clone(),
                        c,
                    );
                    ui.painter().text(
                        egui::pos2(rect.min.x + col_w * 1.5, rect.min.y + rect_h * 0.25),
                        egui::Align2::CENTER_CENTER,
                        bpm_str,
                        font.clone(),
                        c,
                    );
                    ui.painter().text(
                        egui::pos2(rect.min.x + col_w * 2.5, rect.min.y + rect_h * 0.25),
                        egui::Align2::CENTER_CENTER,
                        pos_str,
                        font.clone(),
                        c,
                    );

                    ui.painter().text(
                        egui::pos2(rect.min.x + col_w * 0.5, rect.min.y + rect_h * 0.75),
                        egui::Align2::CENTER_CENTER,
                        mem_str,
                        font.clone(),
                        c,
                    );
                    ui.painter().text(
                        egui::pos2(rect.min.x + col_w * 1.5, rect.min.y + rect_h * 0.75),
                        egui::Align2::CENTER_CENTER,
                        ts_str,
                        font.clone(),
                        c,
                    );
                    ui.painter().text(
                        egui::pos2(rect.min.x + col_w * 2.5, rect.min.y + rect_h * 0.75),
                        egui::Align2::CENTER_CENTER,
                        time_str,
                        font.clone(),
                        c,
                    );
                }
            });
        });
}
