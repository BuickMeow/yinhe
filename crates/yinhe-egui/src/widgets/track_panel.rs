use std::collections::HashMap;

use eframe::egui;

use yinhe_midi::TrackInfo;
use yinhe_types::TRACK_PALETTE;

/// Render the track list using a painter (unified component for both
/// pianoroll and transport contexts).
///
/// - `row_height`: current row height (determines badge mode)
/// - `scroll_y`: mutable scroll offset
/// - `show_details`: when true show two-row badge + note count + PC;
///   when false show single-row badge (track number only)
pub(crate) fn show(
    ui: &mut egui::Ui,
    track_info: &[TrackInfo],
    track_visible: &mut [bool],
    track_selected: &mut Option<u16>,
    pc_map: &HashMap<u8, u8>,
    row_height: &mut f32,
    scroll_y: &mut f32,
) {
    let panel_rect = ui.max_rect();
    let panel_w = panel_rect.width();
    let panel_h = panel_rect.height();
    let num_tracks = track_info.len();

    if num_tracks == 0 || panel_w < 1.0 || panel_h < 1.0 {
        return;
    }

    let show_details = *row_height >= 30.0;

    // Clamp scroll_y
    let max_scroll = (num_tracks as f32 * *row_height - panel_h).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Visible track range
    let first = (*scroll_y / *row_height).floor() as usize;
    let visible_count = (panel_h / *row_height).ceil() as usize + 2;
    let last = (first + visible_count).min(num_tracks);

    let painter = ui.painter().clone();
    let interact_id = egui::Id::new("track_panel_area");
    let resp = ui.interact(panel_rect, interact_id, egui::Sense::click_and_drag());

    for idx in first..last {
        if !track_visible.get(idx).copied().unwrap_or(true) {
            continue;
        }
        let ti = &track_info[idx];
        let y = panel_rect.min.y + idx as f32 * *row_height - *scroll_y;
        if y > panel_rect.max.y || y + *row_height < panel_rect.min.y {
            continue;
        }

        let row_rect = egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, *row_height),
        );

        // Selection / hover background
        let selected = *track_selected == Some(ti.index);
        if selected {
            painter.rect_filled(
                row_rect,
                0.0,
                ui.visuals().selection.bg_fill,
            );
        } else if row_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
            painter.rect_filled(
                row_rect,
                0.0,
                egui::Color32::WHITE.gamma_multiply(0.03),
            );
        }

        // Badge color strip (thin vertical bar)
        let color = TRACK_PALETTE[idx % TRACK_PALETTE.len()];
        let color32 = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );

        let badge_w = 8.0_f32;
        let badge_rect = egui::Rect::from_min_size(
            row_rect.min,
            egui::vec2(badge_w, *row_height),
        );
        painter.rect_filled(badge_rect, 0.0, color32);

        // Text area starts after badge
        let text_x = badge_rect.max.x + 6.0;
        let track_num_text = format!("{:03}", ti.index + 1);

        if show_details && *row_height >= 30.0 {
            let port_letter = match ti.port {
                0 => 'A', 1 => 'B', 2 => 'C', 3 => 'D',
                4 => 'E', 5 => 'F', 6 => 'G', 7 => 'H',
                _ => '?',
            };
            let font = egui::FontId::proportional((*row_height * 0.25).clamp(8.0, 13.0));

            // Row 1: track number + port/channel + note count
            painter.text(
                egui::pos2(text_x, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font.clone(),
                egui::Color32::WHITE.gamma_multiply(0.85),
            );
            let badge_text = format!("{}{:02}", port_letter, ti.channel);
            painter.text(
                egui::pos2(text_x + 32.0, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &badge_text,
                font.clone(),
                egui::Color32::WHITE.gamma_multiply(0.85),
            );
            let global_ch = ti.port * 16 + (ti.channel - 1);
            let mut detail = format!("{} notes", ti.note_count);
            if let Some(pc) = pc_map.get(&global_ch) {
                detail.push_str(&format!(" | PC:{}", pc));
            }
            let detail_font = egui::FontId::proportional((*row_height * 0.20).clamp(8.0, 11.0));
            painter.text(
                egui::pos2(text_x + 62.0, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &detail,
                detail_font,
                egui::Color32::GRAY,
            );

            // Row 2: track name
            let name = &ti.name;
            let name_font = egui::FontId::proportional((*row_height * 0.25).clamp(9.0, 13.0));
            painter.text(
                egui::pos2(text_x, badge_rect.min.y + *row_height * 0.70),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                egui::Color32::WHITE.gamma_multiply(0.85),
            );
        } else {
            // Single-row: track number + track name
            let font = egui::FontId::proportional((*row_height * 0.45).clamp(8.0, 14.0));
            painter.text(
                egui::pos2(text_x, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font,
                egui::Color32::WHITE.gamma_multiply(0.85),
            );

            let name = &ti.name;
            let name_font = egui::FontId::proportional((*row_height * 0.45).clamp(8.0, 14.0));
            painter.text(
                egui::pos2(text_x + 40.0, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                egui::Color32::WHITE.gamma_multiply(0.85),
            );
        }
    }

    // ── Interaction: click to select, scroll, wheel ──

    // Click on a row → select track
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos() {
            let rel_y = pos.y - panel_rect.min.y + *scroll_y;
            let clicked_idx = (rel_y / *row_height).floor() as usize;
            if clicked_idx < num_tracks {
                *track_selected = Some(track_info[clicked_idx].index);
            }
        }

    // Mouse wheel → scroll
    if resp.hovered() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y.abs() > 0.5 {
            *scroll_y = (*scroll_y - scroll_delta.y).max(0.0);
        }
    }
}
