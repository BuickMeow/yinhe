use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_types::AutomationLane;
use yinhe_types::AutomationTarget;

use yinhe_automation::{AutomationPanelView, PianorollRenderer, prepare_automation};

/// Curated list of known automation targets shown in the dropdown.
const AUTOMATION_TARGETS: &[AutomationTarget] = &[
    AutomationTarget::PitchBend,
    AutomationTarget::CC { controller: 7 },  // Volume
    AutomationTarget::CC { controller: 10 }, // Pan
    AutomationTarget::CC { controller: 11 }, // Expression
    AutomationTarget::CC { controller: 64 }, // Sustain
    AutomationTarget::CC { controller: 71 }, // Resonance
    AutomationTarget::CC { controller: 72 }, // Release
    AutomationTarget::CC { controller: 73 }, // Attack
    AutomationTarget::CC { controller: 74 }, // Cutoff
    AutomationTarget::Rpn { parameter: 0 },  // PB Sensitivity
    AutomationTarget::Rpn { parameter: 1 },  // Fine Tune
    AutomationTarget::Rpn { parameter: 2 },  // Coarse Tune
];

use crate::render_context::RenderContext;
use crate::theme;

/// Height of the split/handle between automation panels.
pub(crate) const SPLIT_H: f32 = theme::AUTO_PANEL_SPLIT_H;

/// Ensure `renderers` has the same count as `panels`, creating/destroying as needed.
fn sync_renderer_count(
    renderers: &mut Vec<(PianorollRenderer, RenderContext)>,
    panels: &[AutomationPanelView],
    wgpu_state: &Arc<eframe::egui_wgpu::RenderState>,
    default_w: u32,
    default_h: u32,
) {
    while renderers.len() < panels.len() {
        let renderer = PianorollRenderer::new(
            wgpu_state.device.clone(),
            wgpu_state.queue.clone(),
            wgpu_state.target_format,
        );
        let ctx = RenderContext::from_render_state(Arc::clone(wgpu_state), default_w, default_h);
        renderers.push((renderer, ctx));
    }
    while renderers.len() > panels.len() {
        renderers.pop();
    }
}

/// Render all automation panels between the pianoroll content and the scrollbar.
///
/// The first panel sits flush against the content above. Each subsequent panel
/// has a `SPLIT_H` drag handle at its top edge.
///
/// Returns the total height consumed by all panels (including split handles
/// between them, but no leading handle for the first panel).
pub fn show_panels(
    ui: &mut egui::Ui,
    panels: &mut Vec<AutomationPanelView>,
    renderers: &mut Vec<(PianorollRenderer, RenderContext)>,
    automation_lanes: &[AutomationLane],
    show_panels: &mut bool,
    wgpu_state: &Arc<eframe::egui_wgpu::RenderState>,
    combo_width: f32,
    pianoroll_scroll_x: f32,
    pianoroll_ppt: f32,
    content_rect_right: f32,
    content_top_y: f32,
    panels_visible_h: f32,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[yinhe_types::TimeSigEvent],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    scroll_mode: u32,
    min_border_width: f32,
    midi: Option<&dyn yinhe_automation::NoteSource>,
    velocity_display_mode: &mut u32,
    automation_display_mode: &mut u32,
    automation_show_dots: &mut bool,
    tempo_events: &[(u32, f64)],
) -> f32 {
    if !*show_panels || panels.is_empty() {
        return 0.0;
    }

    // Sync scroll state from pianoroll
    for panel in panels.iter_mut() {
        panel.sync_from_pianoroll(pianoroll_scroll_x, pianoroll_ppt, combo_width);
    }

    // Ensure renderer count matches panel count
    sync_renderer_count(renderers, panels, wgpu_state, 640, 200);

    // Snapshot pre-drag heights so rendering stays consistent with the
    // pre-computed panels_total_h layout. Drag writes to panel_height for
    // the next frame instead of mid-frame, avoiding one-frame overlap jitter.
    let orig_heights: Vec<f32> = panels.iter().map(|p| p.panel_height).collect();

    // ── Scroll state for overflow ──
    let panels_natural_h: f32 =
        orig_heights.iter().sum::<f32>() + (panels.len() as f32 * SPLIT_H);
    let max_scroll = (panels_natural_h - panels_visible_h).max(0.0);

    let scroll_id = ui.id().with("auto_panel_scroll_y");
    let mut scroll_y: f32 = ui.data_mut(|d| d.get_persisted(scroll_id)).unwrap_or(0.0);
    scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Panels area rect (visible portion only)
    let panels_area_rect = egui::Rect::from_min_max(
        egui::pos2(0.0, content_top_y),
        egui::pos2(content_rect_right, content_top_y + panels_visible_h),
    );

    // Handle mouse wheel / trackpad scroll in the panels area
    let pointer_in_panels = ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| panels_area_rect.contains(p))
    });
    if pointer_in_panels && max_scroll > 0.0 {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        scroll_y = (scroll_y - scroll_delta.y).clamp(0.0, max_scroll);
    }
    ui.data_mut(|d| d.insert_persisted(scroll_id, scroll_y));

    // Clip all painting to the panels area
    let old_clip = ui.clip_rect();
    ui.set_clip_rect(panels_area_rect.intersect(old_clip));

    let mut y_offset = content_top_y - scroll_y;
    let visible_top = content_top_y;
    let visible_bottom = content_top_y + panels_visible_h;

    for (i, panel) in panels.iter_mut().enumerate() {
        // Split handle before every panel (first = divider from pianoroll)
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, y_offset),
            egui::pos2(content_rect_right, y_offset + SPLIT_H),
        );
        let handle_resp =
            crate::widgets::split_handle::horizontal(ui, format!("auto_handle_{}", i), handle_rect);
        if handle_resp.dragged() {
            let delta = handle_resp.drag_delta().y;
            let new_h = (panel.panel_height - delta).clamp(
                yinhe_automation::automation_view::MIN_PANEL_HEIGHT,
                yinhe_automation::automation_view::MAX_PANEL_HEIGHT,
            );
            panel.panel_height = new_h;
            panel.dirty = true;
            ui.ctx().request_repaint();
        }
        y_offset += SPLIT_H;

        // Render at original height (consistent with pre-computed layout)
        let panel_h = orig_heights[i];
        let panel_top = y_offset;
        let panel_bottom = y_offset + panel_h;
        let panel_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, panel_top),
            egui::pos2(content_rect_right, panel_bottom),
        );

        // Skip heavy rendering for panels entirely outside the visible area
        let is_visible = panel_bottom >= visible_top && panel_top <= visible_bottom;
        if !is_visible {
            y_offset += panel_h;
            continue;
        }

        // ── wgpu automation content (full width, from x=0) ──
        let grid_rect = egui::Rect::from_min_max(panel_rect.min, panel_rect.max);

        let gw = grid_rect.width() as u32;
        let gh = grid_rect.height() as u32;

        if gw > 0 && gh > 0 {
            if let Some((renderer, render_ctx)) = renderers.get_mut(i) {
                render_ctx.ensure_size(gw, gh);

                let lanes: Vec<&AutomationLane> = automation_lanes
                    .iter()
                    .filter(|l| l.target == panel.selected_target)
                    .collect();

                let force_rebuild = panel.dirty;
                let gpu_dirty = prepare_automation(
                    renderer,
                    gw,
                    gh,
                    panel,
                    &lanes,
                    midi,
                    tpb,
                    default_num,
                    default_den,
                    time_sig_events,
                    track_visible,
                    track_colors,
                    force_rebuild,
                    scroll_mode,
                    min_border_width,
                    *velocity_display_mode,
                    *automation_display_mode,
                    *automation_show_dots,
                    tempo_events,
                );

                let content_changed = panel.dirty || gpu_dirty;
                panel.dirty = false;

                let painter = ui.painter();
                render_ctx.paint(
                    renderer,
                    gw,
                    gh,
                    &format!("auto_panel_{}", i),
                    painter,
                    grid_rect,
                    content_changed,
                );
            }
        }

        // ── Left side: target selector + display mode buttons ──
        let combo_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + combo_width, panel_rect.max.y),
        );

        // Draw left panel background (covers the grid underneath)
        ui.painter().rect_filled(combo_rect, 0.0, theme::APP_BG);

        let combo_inner = combo_rect.shrink(4.0);

        ui.scope_builder(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
            ui.set_clip_rect(combo_inner.intersect(panels_area_rect));
            let layout = egui::Layout::top_down(egui::Align::Center);
            ui.with_layout(layout, |ui| {
                // ── Target selector button (tools panel style) ──
                let target_resp = ui.add(
                    egui::Label::new(ICON_AUTOMATION.rich_text().size(14.0).color(egui::Color32::GRAY))
                        .sense(egui::Sense::click())
                        .selectable(false),
                );
                crate::widgets::hover::hover_highlight(
                    ui,
                    &target_resp,
                    ICON_AUTOMATION.codepoint,
                    egui::FontId::new(14.0, ICON_AUTOMATION.font_family()),
                    false,
                );

                // ── Popup menu (manually managed Area to support DragValue interaction) ──
                let popup_id = ui.id().with("auto_target_popup");
                let is_open = ui.data_mut(|d| d.get_persisted::<bool>(popup_id)).unwrap_or(false);

                if target_resp.clicked() {
                    ui.data_mut(|d| d.insert_persisted(popup_id, !is_open));
                }

                if is_open {
                    let popup_pos = egui::pos2(target_resp.rect.left(), target_resp.rect.bottom());
                    let area_resp = egui::Area::new(popup_id)
                        .order(egui::Order::Foreground)
                        .fixed_pos(popup_pos)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::menu(ui.style()).show(ui, |ui| {
                                ui.set_min_width(120.0);
                                // Velocity (special: not an AutomationTarget, renders from notes)
                                let vel_selected = panel.show_velocity;
                                if ui.add(egui::Button::selectable(vel_selected, "Velocity")).clicked() {
                                    panel.show_velocity = true;
                                    panel.show_tempo = false;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                                // Tempo (special: renders from conductor tempo events)
                                let tempo_selected = panel.show_tempo;
                                if ui.add(egui::Button::selectable(tempo_selected, "Tempo")).clicked() {
                                    panel.show_tempo = true;
                                    panel.show_velocity = false;
                                    panel.dirty = true;
                                    ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                }
                                ui.separator();
                                for target in AUTOMATION_TARGETS {
                                    let name = target.display_name();
                                    let selected = !panel.show_velocity && !panel.show_tempo && panel.selected_target == *target;
                                    if ui.add(egui::Button::selectable(selected, &name)).clicked() {
                                        panel.selected_target = target.clone();
                                        panel.show_velocity = false;
                                        panel.show_tempo = false;
                                        panel.dirty = true;
                                        ui.ctx().data_mut(|d| d.insert_persisted(popup_id, false));
                                    }
                                }
                                ui.separator();
                                ui.label("自定义 CC:");
                                let mut cc_input = match &panel.selected_target {
                                    AutomationTarget::CC { controller } => *controller as i32,
                                    _ => 0,
                                };
                                let old_cc = cc_input;
                                ui.add(egui::DragValue::new(&mut cc_input).range(0..=127).speed(1));
                                if cc_input != old_cc {
                                    panel.selected_target = AutomationTarget::CC { controller: cc_input as u8 };
                                    panel.show_velocity = false;
                                    panel.show_tempo = false;
                                    panel.dirty = true;
                                }
                            });
                        });

                    // Close only when clicking outside the popup area (not on any interactive element)
                    if ui.input(|i| i.pointer.any_pressed()) {
                        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                            if !area_resp.response.rect.contains(pos) && !target_resp.rect.contains(pos) {
                                ui.data_mut(|d| d.insert_persisted(popup_id, false));
                            }
                        }
                    }
                }

                ui.add_space(4.0);

                // ── Display mode buttons ──
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if panel.show_velocity {
                        // Velocity display mode: three text buttons
                        let vel_modes = [(0u32, "柱"), (1u32, "矩"), (2u32, "空")];
                        for &(mode, label) in &vel_modes {
                            let is_active = *velocity_display_mode == mode;
                            let color = if is_active {
                                crate::theme::ACCENT_ACTIVE
                            } else {
                                egui::Color32::GRAY
                            };
                            let resp = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(label).size(11.0).color(color)
                                )
                                .sense(egui::Sense::click())
                                .selectable(false),
                            );
                            if resp.clicked() {
                                *velocity_display_mode = mode;
                            }
                            crate::widgets::hover::hover_highlight(
                                ui,
                                &resp,
                                label,
                                egui::FontId::proportional(11.0),
                                is_active,
                            );
                        }
                    } else if panel.show_tempo {
                        // Tempo only uses line mode — no display mode buttons
                    } else {
                        // Automation display mode: bar chart / line chart icons
                        let auto_modes = [(0u32, ICON_BAR_CHART), (1u32, ICON_SHOW_CHART)];
                        for &(mode, icon) in &auto_modes {
                            let is_active = *automation_display_mode == mode;
                            let color = if is_active {
                                crate::theme::ACCENT_ACTIVE
                            } else {
                                egui::Color32::GRAY
                            };
                            let resp = ui.add(
                                egui::Label::new(icon.rich_text().size(14.0).color(color))
                                    .sense(egui::Sense::click())
                                    .selectable(false),
                            );
                            if resp.clicked() {
                                *automation_display_mode = mode;
                            }
                            crate::widgets::hover::hover_highlight(
                                ui,
                                &resp,
                                icon.codepoint,
                                egui::FontId::new(14.0, icon.font_family()),
                                is_active,
                            );
                        }

                        // Dots toggle (only in折线 mode)
                        if *automation_display_mode == 1 {
                            let dot_color = if *automation_show_dots {
                                crate::theme::ACCENT_ACTIVE
                            } else {
                                egui::Color32::GRAY
                            };
                            let dot_resp = ui.add(
                                egui::Label::new(ICON_STEPPERS.rich_text().size(14.0).color(dot_color))
                                    .sense(egui::Sense::click())
                                    .selectable(false),
                            );
                            if dot_resp.clicked() {
                                *automation_show_dots = !*automation_show_dots;
                            }
                            crate::widgets::hover::hover_highlight(
                                ui,
                                &dot_resp,
                                ICON_STEPPERS.codepoint,
                                egui::FontId::new(14.0, ICON_STEPPERS.font_family()),
                                *automation_show_dots,
                            );
                            dot_resp.on_hover_text(if *automation_show_dots { "隐藏圆点" } else { "显示圆点" });
                        }
                    }
                });

            });
        });

        // ── Grid overlay: value labels + target name ──
        let name = if panel.show_velocity {
            "Velocity".to_string()
        } else if panel.show_tempo {
            "Tempo".to_string()
        } else {
            panel.selected_target.display_name()
        };
        let label_color = theme::MEASURE_LABEL;
        let font_id = egui::FontId::proportional(10.0);
        let pad_x = 4.0;

        let (top_val, mid_val, bot_val) = if panel.show_velocity {
            ("127".to_string(), "64".to_string(), "0".to_string())
        } else if panel.show_tempo {
            let max_bpm = tempo_events
                .iter()
                .map(|(_, bpm)| *bpm)
                .fold(0.0f64, f64::max);
            (format!("{:.1}", max_bpm), format!("{:.1}", max_bpm / 2.0), "0.0".into())
        } else {
            let target = &panel.selected_target;
            let max = target.max_value();
            let def = target.default_value();
            match target {
                AutomationTarget::PitchBend => {
                    let half = max - def; // 8191
                    (half.to_string(), "0".into(), (-(half as i32)).to_string())
                }
                _ if target.has_center_line() => {
                    (max.to_string(), def.to_string(), "0".into())
                }
                _ => {
                    (max.to_string(), (max / 2).to_string(), "0".into())
                }
            }
        };

        let text_x = panel_rect.min.x + combo_width + pad_x;
        let top_y = panel_rect.min.y + 4.0;
        let mid_y = panel_rect.center().y;
        let bot_y = panel_rect.max.y - 4.0;

        let painter = ui.painter();
        painter.text(
            egui::pos2(text_x, top_y),
            egui::Align2::LEFT_TOP,
            top_val,
            font_id.clone(),
            label_color,
        );
        painter.text(
            egui::pos2(text_x, mid_y),
            egui::Align2::LEFT_CENTER,
            mid_val,
            font_id.clone(),
            label_color,
        );
        painter.text(
            egui::pos2(text_x, bot_y),
            egui::Align2::LEFT_BOTTOM,
            bot_val,
            font_id.clone(),
            label_color,
        );

        // Target name: bottom-left, 100px from grid left edge, same row as bottom value
        let name_x = panel_rect.min.x + combo_width + 40.0;
        painter.text(
            egui::pos2(name_x, bot_y),
            egui::Align2::LEFT_BOTTOM,
            &name,
            font_id.clone(),
            label_color,
        );

        y_offset += panel_h;
    }

    // Restore clip rect
    ui.set_clip_rect(old_clip);

    panels_visible_h
}

/// Show the toggle / add / remove buttons horizontally.
///
/// Designed to be called inside a `ui.horizontal()` or `ui.horizontal_centered()`
/// scope (e.g. inside the scrollbar left blank area).
pub fn show_toggle_buttons(ui: &mut egui::Ui, show_panels: &mut bool, panel_count: &mut usize) {
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.add_space(6.0);

    // Toggle button
    let toggle_color = if *show_panels {
        theme::ACCENT_ACTIVE
    } else {
        egui::Color32::GRAY
    };
    let toggle_label = ICON_SIGNAL_CELLULAR_ALT
        .rich_text()
        .size(theme::MODE_LABEL_FONT + 2.0)
        .color(toggle_color);
    let toggle_resp = ui.add(
        egui::Label::new(toggle_label)
            .sense(egui::Sense::click())
            .selectable(false),
    );
    crate::widgets::hover::hover_highlight(
        ui,
        &toggle_resp,
        ICON_SIGNAL_CELLULAR_ALT.codepoint,
        egui::FontId::new(
            theme::MODE_LABEL_FONT + 2.0,
            ICON_SIGNAL_CELLULAR_ALT.font_family(),
        ),
        *show_panels,
    );
    if toggle_resp.clicked() {
        *show_panels = !*show_panels;
        if *show_panels && *panel_count == 0 {
            *panel_count = 1;
        }
    }

    if *show_panels {
        // + button (add panel)
        let plus_color = egui::Color32::GRAY;
        let plus_resp = ui.add(
            egui::Label::new(
                ICON_ADD
                    .rich_text()
                    .size(theme::MODE_LABEL_FONT + 2.0)
                    .color(plus_color),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        crate::widgets::hover::hover_highlight(
            ui,
            &plus_resp,
            ICON_ADD.codepoint,
            egui::FontId::new(theme::MODE_LABEL_FONT + 2.0, ICON_ADD.font_family()),
            false,
        );
        if plus_resp.clicked() {
            *panel_count += 1;
        }

        // - button (remove panel)
        let minus_resp = ui.add(
            egui::Label::new(
                ICON_REMOVE
                    .rich_text()
                    .size(theme::MODE_LABEL_FONT + 2.0)
                    .color(plus_color),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        crate::widgets::hover::hover_highlight(
            ui,
            &minus_resp,
            ICON_REMOVE.codepoint,
            egui::FontId::new(theme::MODE_LABEL_FONT + 2.0, ICON_REMOVE.font_family()),
            false,
        );
        if minus_resp.clicked() && *panel_count > 0 {
            *panel_count -= 1;
        }
    }
}
