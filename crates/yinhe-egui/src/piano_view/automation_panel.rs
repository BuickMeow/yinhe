use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_types::AutomationLane;
use yinhe_types::AutomationTarget;

use yinhe_automation::{AutomationPanelView, PianorollRenderer, prepare_automation};

/// Curated list of known automation targets shown in the dropdown.
const AUTOMATION_TARGETS: &[AutomationTarget] = &[
    AutomationTarget::Velocity,
    AutomationTarget::PitchBend,
    AutomationTarget::CC { controller: 7 },  // Volume
    AutomationTarget::CC { controller: 10 }, // Pan
    AutomationTarget::CC { controller: 11 }, // Expression
    AutomationTarget::CC { controller: 64 }, // Sustain
    AutomationTarget::CC { controller: 71 }, // Resonance
    AutomationTarget::CC { controller: 72 }, // Release
    AutomationTarget::CC { controller: 73 }, // Attack
    AutomationTarget::CC { controller: 74 }, // Cutoff
    AutomationTarget::PitchBendSensitivity,  // RPN 0
    AutomationTarget::FineTune,              // RPN 1
    AutomationTarget::CoarseTune,            // RPN 2
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

    let mut y_offset = content_top_y;

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
        let panel_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, y_offset),
            egui::pos2(content_rect_right, y_offset + panel_h),
        );

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
        let is_velocity = panel.selected_target == AutomationTarget::Velocity;

        ui.scope_builder(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
            ui.set_clip_rect(combo_inner);
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

                egui::Popup::menu(&target_resp)
                    .align(egui::RectAlign::TOP_START)
                    .show(|ui| {
                        ui.set_min_width(120.0);
                        for target in AUTOMATION_TARGETS {
                            let name = target.display_name();
                            let selected = panel.selected_target == *target;
                            if ui.add(egui::Button::selectable(selected, &name)).clicked() {
                                panel.selected_target = target.clone();
                                panel.dirty = true;
                                ui.close();
                            }
                        }
                        ui.separator();
                        ui.label("自定义 CC:");
                        let mut cc_input = match &panel.selected_target {
                            AutomationTarget::CC { controller } => *controller as i32,
                            _ => 0,
                        };
                        let resp = ui.add(egui::DragValue::new(&mut cc_input).range(0..=127).speed(1));
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            panel.selected_target = AutomationTarget::CC { controller: cc_input as u8 };
                            panel.dirty = true;
                            ui.close();
                        }
                    });

                ui.add_space(4.0);

                // ── Second row: display mode buttons ──
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if is_velocity {
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
        let name = panel.selected_target.display_name();
        let label_color = theme::MEASURE_LABEL;
        let font_id = egui::FontId::proportional(10.0);
        let pad_x = 4.0;

        let (top_val, mid_val, bot_val) = {
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

    y_offset - content_top_y
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
