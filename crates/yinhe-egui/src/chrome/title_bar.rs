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
    tab_scroll_offset: &mut f32,
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
        .show(ui, |ui| {
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

            let tmp_docs: Vec<(&Document, bool)> = documents
                .iter()
                .enumerate()
                .map(|(i, d)| (d, *active_doc == Some(i)))
                .collect();

            // Collect tab_rects and close_rects for manual click detection
            let mut click_targets: Vec<(usize, egui::Rect, egui::Rect)> = Vec::new();

            let font_id = egui::FontId::proportional(12.0);
            let close_w = 20.0;
            let padding = 6.0;
            let tab_gap = 2.0;

            // Uniform tab width: fixed 120px for compact tabs
            let tab_w = 120.0;
            let text_max_w = tab_w - close_w - padding * 2.0;

            // ── Handle mouse wheel / trackpad scroll for tab overflow ──
            let pointer_in_bar = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .is_some_and(|p| bar_rect.contains(p))
            });
            if pointer_in_bar {
                let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
                let zoom_delta = ui.input(|i| i.zoom_delta());

                // Mouse wheel horizontal scroll: Cmd+scroll or plain horizontal scroll
                let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                if cmd && scroll_delta.y.abs() > 0.5 {
                    // Cmd+vertical scroll → tab horizontal scroll
                    *tab_scroll_offset -= scroll_delta.y * 2.0;
                } else if scroll_delta.x.abs() > 0.5 {
                    // Trackpad horizontal swipe → tab scroll
                    *tab_scroll_offset -= scroll_delta.x;
                } else if (zoom_delta - 1.0).abs() > 0.001 {
                    // Trackpad pinch → tab scroll (zoom gesture repurposed for tab scroll)
                    *tab_scroll_offset -= (zoom_delta - 1.0) * 100.0;
                } else if !cmd && scroll_delta.y.abs() > 0.5 && cfg!(target_os = "macos") {
                    // Plain vertical scroll on macOS → also scroll tabs if overflow
                    *tab_scroll_offset -= scroll_delta.y * 2.0;
                }

                // Clamp scroll offset
                let total_tab_w = tmp_docs.len() as f32 * (tab_w + tab_gap);
                let available_w = bar_rect.width() - left_padding;
                let max_offset = (total_tab_w - available_w).max(0.0);
                *tab_scroll_offset = tab_scroll_offset.clamp(0.0, max_offset);

                if scroll_delta != egui::Vec2::ZERO || (zoom_delta - 1.0).abs() > 0.001 {
                    ui.ctx().request_repaint();
                }
            }

            // ── Draw title BEHIND tabs (lower z-order) ──
            painter.text(
                egui::pos2(bar_rect.center().x, bar_rect.center().y),
                egui::Align2::CENTER_CENTER,
                "Yinhe MIDI Editor",
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(180),
            );

            let mut tab_x = bar_rect.min.x + left_padding - *tab_scroll_offset;

            let hover_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();

            for (i, (doc, is_active)) in tmp_docs.iter().enumerate() {
                let tab_rect = egui::Rect::from_min_max(
                    egui::pos2(tab_x, tab_y),
                    egui::pos2(tab_x + tab_w, tab_y + tab_h),
                );

                // Skip tabs entirely outside the visible area
                if tab_rect.max.x < bar_rect.min.x + left_padding || tab_rect.min.x > bar_rect.max.x {
                    click_targets.push((i, tab_rect, egui::Rect::NOTHING));
                    tab_x += tab_w + tab_gap;
                    continue;
                }

                // Tab background — active / hover / inactive
                let is_hovered = tab_rect.contains(hover_pos) && !*is_active;
                let bg = if *is_active {
                    crate::theme::TAB_ACTIVE_BG
                } else if is_hovered {
                    crate::theme::TAB_HOVER_BG
                } else {
                    crate::theme::TAB_INACTIVE_BG
                };
                painter.rect_filled(tab_rect, 4.0, bg);

                // Build display name with dirty indicator
                let file_name = doc.file_name.as_str();
                let display_name = if doc.is_dirty() {
                    format!("*{}", file_name)
                } else {
                    file_name.to_string()
                };

                // Tab text with ellipsis truncation
                let text_color = if *is_active {
                    egui::Color32::from_gray(220)
                } else {
                    egui::Color32::from_gray(180)
                };
                let text_to_draw = {
                    let full_w = painter
                        .layout_no_wrap(display_name.clone(), font_id.clone(), text_color)
                        .size()
                        .x;
                    if full_w <= text_max_w {
                        display_name
                    } else {
                        let ellipsis = "\u{2026}";
                        let mut truncated = String::new();
                        for c in display_name.chars() {
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
                let close_hover = tab_close_rect.contains(hover_pos);
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

                tab_x += tab_w + tab_gap;
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
                        if tab_close_rect == egui::Rect::NOTHING {
                            continue;
                        }
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

            // ── Paint window buttons (non-macOS, visual only) ──
            #[cfg(not(target_os = "macos"))]
            {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                paint_window_buttons(ui, &win_btn_rects, maximized);
            }

            // ── Collect all tab rects for drag-exclusion ──
            let tab_rects: Vec<egui::Rect> = click_targets
                .iter()
                .map(|(_, tab_rect, _)| *tab_rect)
                .collect();

            // ── Window drag region (after the tabs, excluding window buttons) ──
            // Use manual pointer check to ensure clicks on tab rects never start a drag.
            let drag_right = if cfg!(target_os = "macos") {
                bar_rect.max.x
            } else {
                bar_rect.max.x - 138.0
            };
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(bar_rect.min.x + left_padding, bar_rect.min.y),
                egui::pos2(drag_right, bar_rect.max.y),
            );

            // Only start drag if press position is not inside any tab rect.
            let press_pos = ui.input(|i| i.pointer.press_origin());
            let on_tab = press_pos.is_some_and(|pos| tab_rects.iter().any(|r| r.contains(pos)));
            if !on_tab {
                let drag_resp = ui.interact(drag_rect, ui.next_auto_id(), egui::Sense::drag());
                if drag_resp.dragged_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            }

            // Double-click title bar drag area to toggle maximize/restore
            const DOUBLE_CLICK_MS: f64 = 400.0;
            let dbl_id = ui.id().with("title_bar_dbl_click");
            if !on_tab
                && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
                && drag_rect.contains(pos)
            {
                let now = ui.input(|i| i.time);
                let last_click: f64 = ui.data_mut(|d| d.get_persisted(dbl_id)).unwrap_or(0.0);
                if now - last_click < DOUBLE_CLICK_MS / 1000.0 {
                    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    ui.data_mut(|d| d.insert_persisted(dbl_id, 0.0)); // reset
                } else {
                    ui.data_mut(|d| d.insert_persisted(dbl_id, now));
                }
            }

            // Reserve space for title bar height
            ui.allocate_space(egui::vec2(0.0, TITLE_BAR_HEIGHT));
        });
    action
}

/// Paint the three window control buttons (close, maximize, minimize) for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
fn paint_window_buttons(
    ui: &egui::Ui,
    rects: &(egui::Rect, egui::Rect, egui::Rect),
    maximized: bool,
) {
    let painter = ui.painter();
    let (close_rect, maximize_rect, minimize_rect) = rects;

    // ── Close button (red on hover) ──
    let close_hover = close_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let close_bg = if close_hover {
        egui::Color32::from_rgb(232, 17, 35)
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*close_rect, 0.0, close_bg);
    // X icon
    let x_color = if close_hover {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(120)
    };
    let cx = close_rect.center();
    let x_size = 8.0;
    let x_half = x_size / 2.0;
    let x1 = egui::pos2(cx.x - x_half, cx.y - x_half);
    let x2 = egui::pos2(cx.x + x_half, cx.y + x_half);
    let x3 = egui::pos2(cx.x + x_half, cx.y - x_half);
    let x4 = egui::pos2(cx.x - x_half, cx.y + x_half);
    painter.line_segment([x1, x2], (1.5, x_color));
    painter.line_segment([x3, x4], (1.5, x_color));

    // ── Maximize button ──
    let max_hover = maximize_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let max_bg = if max_hover {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*maximize_rect, 0.0, max_bg);
    let max_color = if max_hover {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(120)
    };
    let mcx = maximize_rect.center();
    let m_size = 9.0;
    if maximized {
        let r1 = egui::Rect::from_center_size(
            egui::pos2(mcx.x - 2.0, mcx.y - 2.0),
            egui::vec2(m_size - 2.0, m_size - 2.0),
        );
        let r2 = egui::Rect::from_center_size(
            egui::pos2(mcx.x + 2.0, mcx.y + 2.0),
            egui::vec2(m_size - 2.0, m_size - 2.0),
        );
        painter.rect_stroke(r1, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
        painter.rect_stroke(r2, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
    } else {
        let r = egui::Rect::from_center_size(mcx, egui::vec2(m_size, m_size));
        painter.rect_stroke(r, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
    }

    // ── Minimize button ──
    let min_hover = minimize_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let min_bg = if min_hover {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*minimize_rect, 0.0, min_bg);
    let min_color = if min_hover {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(120)
    };
    let mn_cx = minimize_rect.center();
    let line_y = mn_cx.y;
    painter.line_segment(
        [
            egui::pos2(mn_cx.x - 5.0, line_y),
            egui::pos2(mn_cx.x + 5.0, line_y),
        ],
        (1.5, min_color),
    );
}
