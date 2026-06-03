use eframe::egui;

use yinhe_types::TRACK_PALETTE;

use crate::document::Document;

/// Render the track list inside a Ui that is already clipped and positioned.
/// Takes explicit references to avoid borrow-checker conflicts with App.
pub(crate) fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
) {
    let info = &doc.track_info_cache;
    let pc_map = &doc.pc_map_cache;
    let track_visible = &mut doc.track_visible;
    let track_selected = &mut doc.track_selected;

    egui::ScrollArea::vertical().show(ui, |ui| {
        for ti in info {
            let idx = ti.index as usize;
            let color = TRACK_PALETTE[idx % TRACK_PALETTE.len()];
            let color32 = egui::Color32::from_rgb(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            );

            let selected = *track_selected == Some(ti.index);
            let bg = if selected {
                ui.visuals().selection.bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            let frame = egui::Frame::default()
                .fill(bg)
                .inner_margin(egui::Margin::symmetric(6, 3));
            frame.show(ui, |ui| {
                // ── Line 1: channel badge + track name ──
                ui.horizontal(|ui| {
                    // Visibility checkbox
                    let mut vis = track_visible.get(idx).copied().unwrap_or(true);
                    if ui.checkbox(&mut vis, "").changed() {
                        if idx < track_visible.len() {
                            track_visible[idx] = vis;
                        }
                    }

                    // Channel badge: small rounded rect
                    let channel = ti.channel;
                    let port_letter = match ti.port {
                        0 => 'A',
                        1 => 'B',
                        2 => 'C',
                        3 => 'D',
                        4 => 'E',
                        5 => 'F',
                        6 => 'G',
                        7 => 'H',
                        _ => '?',
                    };
                    let badge_text = format!("{}{:02}", port_letter, channel);
                    let (_badge, _) =
                        ui.allocate_exact_size(egui::vec2(28.0, 16.0), egui::Sense::hover());
                    let badge_rect = ui.min_rect();
                    let badge_rect = egui::Rect::from_min_size(
                        egui::pos2(badge_rect.min.x, badge_rect.min.y + 2.0),
                        egui::vec2(28.0, 14.0),
                    );
                    ui.painter().rect_filled(badge_rect, 3.0, color32);
                    ui.painter().text(
                        badge_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        badge_text,
                        egui::FontId::monospace(10.0),
                        egui::Color32::WHITE,
                    );

                    // Track name with ellipsis truncation
                    let name_w = ui.available_width().max(10.0);
                    let name = egui::RichText::new(&ti.name).size(13.0);
                    let label =
                        ui.add_sized([name_w, 16.0], egui::Label::new(name).truncate());
                    if label.clicked() {
                        *track_selected = Some(ti.index);
                    }
                });

                // ── Line 2: note count + optional PC ──
                {
                    let global_ch = ti.port * 16 + (ti.channel - 1);
                    let mut line2 = format!("{} notes", ti.note_count);
                    if let Some(pc) = pc_map.get(&global_ch) {
                        line2.push_str(&format!(" | PC:{}", pc));
                    }
                    let w2 = ui.available_width().max(10.0);
                    ui.add_sized(
                        [w2, 14.0],
                        egui::Label::new(
                            egui::RichText::new(line2)
                                .size(11.0)
                                .color(egui::Color32::GRAY),
                        )
                        .truncate(),
                    );
                }
            });
        }
    });
}
