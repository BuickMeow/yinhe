use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_editor_core::document::Document;

/// Height of the custom title bar.
pub(crate) const TITLE_BAR_HEIGHT: f32 = crate::theme::TITLE_BAR_H;

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
            fill: crate::theme::APP_BG,
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

            let tmp_docs: Vec<(bool, &str)> = documents
                .iter()
                .enumerate()
                .map(|(i, d)| (*active_doc == Some(i), d.file_name.as_str()))
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
                        .layout_no_wrap((*name).to_string(), font_id.clone(), egui::Color32::WHITE)
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
                    crate::theme::TAB_ACTIVE_BG
                } else {
                    crate::theme::TAB_INACTIVE_BG
                };
                painter.rect_filled(tab_rect, 4.0, bg);

                // Tab text with ellipsis truncation
                let text_color = egui::Color32::from_gray(200);
                let text_to_draw = {
                    let full_w = painter
                        .layout_no_wrap((*file_name).to_string(), font_id.clone(), text_color)
                        .size()
                        .x;
                    if full_w <= text_max_w {
                        file_name.to_string()
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
                let tab_close_rect = egui::Rect::from_min_size(
                    egui::pos2(tab_rect.max.x - close_w, tab_rect.min.y),
                    egui::vec2(close_w, tab_h),
                );
                let close_hover = tab_close_rect
                    .contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
                if close_hover {
                    painter.rect_filled(tab_close_rect, 4.0, crate::theme::DANGER_HOVER);
                }
                painter.text(
                    tab_close_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ICON_CLOSE.codepoint,
                    egui::FontId::new(12.0, ICON_CLOSE.font_family()),
                    if close_hover {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_gray(160)
                    },
                );

                click_targets.push((i, tab_rect, tab_close_rect));

                tab_x += tab_w + 4.0;
            }

            // ── Window button rects (non-macOS) for manual click detection ──
            #[cfg(not(target_os = "macos"))]
            let win_btn_rects = {
                let btn_w = 46.0;
                let btn_h = TITLE_BAR_HEIGHT;
                let btn_y = bar_rect.min.y;

                let c = egui::Rect::from_min_size(
                    egui::pos2(bar_rect.max.x - btn_w, btn_y),
                    egui::vec2(btn_w, btn_h),
                );
                let mx = egui::Rect::from_min_size(
                    egui::pos2(c.min.x - btn_w, btn_y),
                    egui::vec2(btn_w, btn_h),
                );
                let mn = egui::Rect::from_min_size(
                    egui::pos2(mx.min.x - btn_w, btn_y),
                    egui::vec2(btn_w, btn_h),
                );
                (c, mx, mn)
            };

            // ── Manual click detection (avoid egui interaction system quirks in Panel::top) ──
            if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
                *title_bar_press_pos = ui.input(|i| i.pointer.interact_pos());
            }

            // On button release, detect which tab/close rect or window button was clicked
            let pointer_released =
                ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            if pointer_released
                && let Some(press) = title_bar_press_pos.take()
                && let Some(release) = ui.input(|i| i.pointer.interact_pos())
            {
                let dist = (release - press).length();
                if dist < 8.0 {
                    // Check document tab buttons
                    for &(idx, tab_rect, tab_close_rect) in click_targets.iter().rev() {
                        if tab_close_rect.contains(press) && tab_close_rect.contains(release) {
                            action = Some(TitleBarAction::CloseDocument(idx));
                            break;
                        }
                        if tab_rect.contains(press) && tab_rect.contains(release) {
                            *active_doc = Some(idx);
                            break;
                        }
                    }

                    // Check window title bar buttons (non-macOS only)
                    #[cfg(not(target_os = "macos"))]
                    {
                        if win_btn_rects.0.contains(press) && win_btn_rects.0.contains(release) {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        } else if win_btn_rects.1.contains(press)
                            && win_btn_rects.1.contains(release)
                        {
                            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        } else if win_btn_rects.2.contains(press)
                            && win_btn_rects.2.contains(release)
                        {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    }
                }
            }

            // ── Draw centered title ──
            painter.text(
                egui::pos2(bar_rect.center().x, bar_rect.center().y),
                egui::Align2::CENTER_CENTER,
                "Yinhe MIDI Editor",
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(180),
            );

            // ── Paint window buttons (non-macOS, visual only) ──
            #[cfg(not(target_os = "macos"))]
            {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                paint_window_buttons(ui, win_btn_rects, maximized);
            }

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
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }

            // Double-click title bar drag area to toggle maximize/restore
            // (same pattern as transport_bar's working implementation)
            if ui.input(|i| {
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary)
            }) && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
                && drag_rect.contains(pos)
            {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }

            // Reserve space for title bar height
            ui.allocate_space(egui::vec2(0.0, TITLE_BAR_HEIGHT));
        });
    action
}

// ── Dialog viewport helpers ──

/// Build a `ViewportBuilder` for a dialog window, matching the main window's
/// custom chrome style (no native title bar, transparent background).
pub(crate) fn dialog_viewport_builder(
    title: &str,
    size: [f32; 2],
    resizable: bool,
) -> egui::ViewportBuilder {
    let mut vb = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_transparent(true);

    #[cfg(target_os = "macos")]
    {
        vb = vb
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        vb = vb.with_decorations(false);
    }

    vb
}

/// Draw a custom title bar for a dialog window.
///
/// - macOS: draws a background colour strip, centered title, and drag
///   region. The native traffic-light buttons remain visible and functional
///   (via `with_fullsize_content_view`).
/// - Other platforms: draws an X close button on the right, centered title,
///   and drag region.
///
/// Sets `*close = true` when the close button is clicked.
pub(crate) fn dialog_title_bar(ui: &mut egui::Ui, title: &str, close: &mut bool) {
    #[cfg(target_os = "macos")]
    let _ = close;

    let height = 28.0;
    let bar_rect = ui.max_rect();

    // Background strip
    ui.painter().rect_filled(
        egui::Rect::from_min_size(bar_rect.min, egui::vec2(bar_rect.max.x, height)),
        0.0,
        crate::theme::APP_BG,
    );

    // ── Close button (right side, non-macOS only) ──
    #[cfg(not(target_os = "macos"))]
    {
        let close_rect = egui::Rect::from_min_size(
            egui::pos2(bar_rect.max.x - height, bar_rect.min.y),
            egui::vec2(height, height),
        );
        let close_hover =
            close_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
        if close_hover {
            ui.painter()
                .rect_filled(close_rect, 0.0, crate::theme::DANGER_HOVER);
        }
        if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
            && close_rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default())
        {
            *close = true;
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_CLOSE.codepoint,
            egui::FontId::new(12.0, ICON_CLOSE.font_family()),
            if close_hover {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_gray(160)
            },
        );
    }

    // ── Centered title (both platforms) ──
    ui.painter().text(
        egui::pos2(bar_rect.center().x, bar_rect.min.y + height / 2.0),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(12.0),
        egui::Color32::from_gray(200),
    );

    // ── Drag region (both platforms) ──
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(bar_rect.min.x, bar_rect.min.y),
        egui::pos2(bar_rect.max.x, bar_rect.min.y + height),
    );
    let drag_resp = ui.interact(drag_rect, ui.next_auto_id(), egui::Sense::drag());
    if drag_resp.dragged_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    // Reserve space
    ui.allocate_space(egui::vec2(0.0, height));
}

/// Paint window buttons (close/maximize/minimize) on non-macOS platforms.
/// Visual only — interactions are handled via manual click detection.
#[cfg(not(target_os = "macos"))]
fn paint_window_buttons(
    ui: &mut egui::Ui,
    rects: (egui::Rect, egui::Rect, egui::Rect),
    maximized: bool,
) {
    let (close_rect, max_rect, min_rect) = rects;
    let hover_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();

    // Close button
    let close_hover = close_rect.contains(hover_pos);
    if close_hover {
        ui.painter()
            .rect_filled(close_rect, 0.0, crate::theme::DANGER_HOVER);
    }
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        ICON_CLOSE.codepoint,
        egui::FontId::new(12.0, ICON_CLOSE.font_family()),
        if close_hover {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(180)
        },
    );

    // Maximize / Restore (show restore icon when window is maximized)
    let max_icon = if maximized {
        ICON_FILTER_NONE
    } else {
        ICON_CHECK_BOX_OUTLINE_BLANK
    };
    let max_hover = max_rect.contains(hover_pos);
    if max_hover {
        ui.painter()
            .rect_filled(max_rect, 0.0, crate::theme::WIN_BTN_HOVER);
    }
    ui.painter().text(
        max_rect.center(),
        egui::Align2::CENTER_CENTER,
        max_icon.codepoint,
        egui::FontId::new(12.0, max_icon.font_family()),
        if max_hover {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(180)
        },
    );

    // Minimize
    let min_hover = min_rect.contains(hover_pos);
    if min_hover {
        ui.painter()
            .rect_filled(min_rect, 0.0, crate::theme::WIN_BTN_HOVER);
    }
    ui.painter().text(
        min_rect.center(),
        egui::Align2::CENTER_CENTER,
        ICON_HORIZONTAL_RULE.codepoint,
        egui::FontId::new(12.0, ICON_HORIZONTAL_RULE.font_family()),
        if min_hover {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(180)
        },
    );
}
