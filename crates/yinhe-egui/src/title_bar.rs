use eframe::egui;

use crate::document::Document;

/// Height of the custom title bar.
pub(crate) const TITLE_BAR_HEIGHT: f32 = 32.0;

/// Action to be performed by the caller after title bar rendering.
pub(crate) enum TitleBarAction {
    CloseDocument(usize),
}



/// Draw the custom title bar at the top of the window.
/// Returns an optional action for the caller to perform (e.g. close a document).
pub(crate) fn show(
    ui: &mut egui::Ui,
    documents: &[Document],
    active_doc: &mut Option<usize>,
    title_bar_press_pos: &mut Option<egui::Pos2>,
) -> Option<TitleBarAction> {
    let mut action = None;
    egui::Panel::top("title_bar")
        .show_separator_line(false)
        .frame(egui::Frame {
            fill: egui::Color32::from_rgb(25, 25, 28),
            inner_margin: egui::Margin::ZERO,
            outer_margin: egui::Margin::ZERO,
            ..Default::default()
        })
        .show_inside(ui, |ui| {
            let bar_rect = ui.max_rect();
            let painter = ui.painter();

            // macOS: leave ~80px on the left for traffic lights
            let left_padding = if cfg!(target_os = "macos") {
                80.0
            } else {
                10.0
            };

            // ── Draw tabs (left side) ──
            let tab_h = 24.0;
            let tab_y = bar_rect.center().y - tab_h / 2.0;
            let mut tab_x = bar_rect.min.x + left_padding;

            let tmp_docs: Vec<(bool, String)> = documents
                .iter()
                .enumerate()
                .map(|(i, d)| (*active_doc == Some(i), d.file_name.clone()))
                .collect();

            // Collect tab_rects and close_rects for manual click detection
            let mut click_targets: Vec<(usize, egui::Rect, egui::Rect)> = Vec::new();

            let font_id = egui::FontId::proportional(12.0);
            let close_w = 20.0;
            let padding = 8.0;

            // Compute uniform tab width: text area capped at 160px, min 40px
            let max_text_w = tmp_docs
                .iter()
                .map(|(_, name)| {
                    painter
                        .layout_no_wrap(name.clone(), font_id.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
                .fold(0.0f32, f32::max)
                .max(40.0)
                .min(160.0);
            let tab_w = max_text_w + padding * 2.0 + close_w;
            let text_max_w = tab_w - close_w - padding * 2.0;

            for (i, (is_active, file_name)) in tmp_docs.iter().enumerate() {
                let tab_rect = egui::Rect::from_min_max(
                    egui::pos2(tab_x, tab_y),
                    egui::pos2(tab_x + tab_w, tab_y + tab_h),
                );

                // Tab background
                let bg = if *is_active {
                    egui::Color32::from_rgb(55, 55, 60)
                } else {
                    egui::Color32::from_rgb(35, 35, 38)
                };
                painter.rect_filled(tab_rect, 4.0, bg);

                // Tab text with ellipsis truncation
                let text_color = egui::Color32::from_gray(200);
                let text_to_draw = {
                    let full_w = painter
                        .layout_no_wrap(file_name.clone(), font_id.clone(), text_color)
                        .size()
                        .x;
                    if full_w <= text_max_w {
                        file_name.clone()
                    } else {
                        // Truncate with "..." suffix
                        let ellipsis = "\u{2026}";
                        let mut truncated = String::new();
                        for c in file_name.chars() {
                            let test_w = painter
                                .layout_no_wrap(
                                    format!("{}{}{}", truncated, c, ellipsis),
                                    font_id.clone(),
                                    text_color,
                                )
                                .size()
                                .x;
                            if test_w > text_max_w {
                                break;
                            }
                            truncated.push(c);
                        }
                        format!("{}{}", truncated, ellipsis)
                    }
                };
                let text_pos = egui::pos2(tab_rect.min.x + padding, tab_rect.center().y);
                painter.text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    text_to_draw,
                    font_id.clone(),
                    text_color,
                );

                // Close button
                let close_rect = egui::Rect::from_min_size(
                    egui::pos2(tab_rect.max.x - close_w, tab_rect.min.y),
                    egui::vec2(close_w, tab_h),
                );
                let close_hover = close_rect
                    .contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()));
                if close_hover {
                    painter.rect_filled(close_rect, 4.0, egui::Color32::from_rgb(200, 50, 50));
                }
                painter.text(
                    close_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{00d7}",
                    egui::FontId::proportional(14.0),
                    if close_hover {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_gray(160)
                    },
                );

                click_targets.push((i, tab_rect, close_rect));

                tab_x += tab_w + 4.0;
            }

            // ── Manual click detection (avoid egui interaction system quirks in Panel::top) ──
            if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
                *title_bar_press_pos = ui.input(|i| i.pointer.interact_pos());
            }

            // On button release, detect which tab/close rect was clicked
            let pointer_released = ui
                .input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            if pointer_released {
                if let Some(press) = title_bar_press_pos.take() {
                    if let Some(release) = ui.input(|i| i.pointer.interact_pos()) {
                        let dist = (release - press).length();
                        // Only treat as click if the pointer barely moved
                        if dist < 8.0 {
                            for &(idx, tab_rect, close_rect) in click_targets.iter().rev() {
                                if close_rect.contains(press) && close_rect.contains(release) {
                                    action = Some(TitleBarAction::CloseDocument(idx));
                                    break;
                                }
                                if tab_rect.contains(press) && tab_rect.contains(release) {
                                    *active_doc = Some(idx);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // ── Draw centered title ──
            let right_limit = if cfg!(target_os = "macos") {
                bar_rect.max.x
            } else {
                bar_rect.max.x - 138.0
            };
            let title_x = (bar_rect.min.x + right_limit) / 2.0;
            painter.text(
                egui::pos2(title_x, bar_rect.center().y),
                egui::Align2::CENTER_CENTER,
                "Yinhe MIDI Editor",
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(180),
            );

            // Non-macOS: draw -口x buttons
            #[cfg(not(target_os = "macos"))]
            draw_window_buttons(ui, bar_rect);

            // ── Window drag region (after the tabs, excluding window buttons) ──
            let drag_rect_left = tab_x.max(bar_rect.min.x + left_padding);
            let drag_right = if cfg!(target_os = "macos") {
                bar_rect.max.x
            } else {
                bar_rect.max.x - 138.0
            };
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(drag_rect_left, bar_rect.min.y),
                egui::pos2(drag_right, bar_rect.max.y),
            );

            let drag_resp = ui.interact(drag_rect, ui.next_auto_id(), egui::Sense::drag());

            if drag_resp.dragged_by(egui::PointerButton::Primary) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }

            // Double-click title bar to toggle maximize/restore
            let pointer_double_clicked = ui.input(|i| {
                i.pointer.button_double_clicked(egui::PointerButton::Primary)
            });
            if pointer_double_clicked {
                let pos_in_drag = ui
                    .input(|i| i.pointer.interact_pos())
                    .map(|p| drag_rect.contains(p))
                    .unwrap_or(false);
                if pos_in_drag {
                    let maximized = ui
                        .input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
            }

            // Reserve space for title bar height
            ui.allocate_space(egui::vec2(0.0, TITLE_BAR_HEIGHT));
        });
    action
}

#[cfg(not(target_os = "macos"))]
fn draw_window_buttons(ui: &mut egui::Ui, bar_rect: egui::Rect) {
    let btn_w = 46.0;
    let btn_h = TITLE_BAR_HEIGHT;
    let btn_y = bar_rect.min.y;

    let close_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.max.x - btn_w, btn_y),
        egui::vec2(btn_w, btn_h),
    );
    let max_rect = egui::Rect::from_min_size(
        egui::pos2(close_rect.min.x - btn_w, btn_y),
        egui::vec2(btn_w, btn_h),
    );
    let min_rect = egui::Rect::from_min_size(
        egui::pos2(max_rect.min.x - btn_w, btn_y),
        egui::vec2(btn_w, btn_h),
    );

    // Close button
    let close_hover = close_rect
        .contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()));
    if close_hover {
        ui.painter()
            .rect_filled(close_rect, 0.0, egui::Color32::from_rgb(200, 50, 50));
    }
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{2715}",
        egui::FontId::proportional(14.0),
        if close_hover {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(180)
        },
    );

    // Maximize
    ui.painter().text(
        max_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{25a1}",
        egui::FontId::proportional(16.0),
        egui::Color32::from_gray(180),
    );

    // Minimize
    ui.painter().text(
        min_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{2500}",
        egui::FontId::proportional(16.0),
        egui::Color32::from_gray(180),
    );

    // Interaction
    let close_resp = ui.interact(close_rect, ui.next_auto_id(), egui::Sense::click());
    let _max_resp = ui.interact(max_rect, ui.next_auto_id(), egui::Sense::click());
    let _min_resp = ui.interact(min_rect, ui.next_auto_id(), egui::Sense::click());

    if close_resp.clicked() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
