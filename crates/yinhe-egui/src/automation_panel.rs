use std::sync::Arc;

use eframe::egui;

use yinhe_types::AutomationLane;

use yinhe_pianoroll::{
    AutomationPanelView, PianorollRenderer, prepare_automation,
};

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
/// Returns the total height consumed by all panels (including split handles).
pub fn show_panels(
    ui: &mut egui::Ui,
    panels: &mut Vec<AutomationPanelView>,
    renderers: &mut Vec<(PianorollRenderer, RenderContext)>,
    automation_lanes: &[AutomationLane],
    show_panels: &mut bool,
    wgpu_state: &Arc<eframe::egui_wgpu::RenderState>,
    left_panel_width: f32,
    pianoroll_scroll_x: f32,
    pianoroll_ppt: f32,
    content_rect_right: f32,
    content_top_y: f32,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[yinhe_types::TimeSigEvent],
) -> f32 {
    if !*show_panels || panels.is_empty() {
        return 0.0;
    }

    // Sync scroll state from pianoroll
    for panel in panels.iter_mut() {
        panel.sync_from_pianoroll(pianoroll_scroll_x, pianoroll_ppt, left_panel_width);
    }

    // Ensure renderer count matches panel count
    sync_renderer_count(renderers, panels, wgpu_state, 640, 200);

    let mut y_offset = content_top_y;

    for (i, panel) in panels.iter_mut().enumerate() {
        let panel_h = panel.panel_height;
        let panel_rect = egui::Rect::from_min_max(
            egui::pos2(content_rect_right - (content_rect_right - left_panel_width), y_offset),
            egui::pos2(content_rect_right, y_offset + panel_h),
        );

        // Left side: dropdown area
        let combo_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + left_panel_width, panel_rect.max.y),
        );

        // Draw left panel background
        ui.painter()
            .rect_filled(combo_rect, 0.0, theme::APP_BG);

        // ComboBox for target selection
        if let Some((_, _render_ctx)) = renderers.get_mut(i) {
            let combo_inner = combo_rect.shrink(4.0);
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
                egui::ComboBox::from_id_salt(ui.id().with(format!("auto_combo_{}", i)))
                    .selected_text(panel.selected_target.display_name())
                    .width(combo_inner.width())
                    .show_ui(ui, |ui| {
                        for (idx, lane) in automation_lanes.iter().enumerate() {
                            let name = lane.target.display_name();
                            if ui
                                .selectable_label(
                                    panel.selected_target == lane.target,
                                    &name,
                                )
                                .clicked()
                            {
                                panel.selected_target = lane.target.clone();
                                panel.lane_index = idx;
                                panel.dirty = true;
                            }
                        }
                    });
            });
        }

        // Right side: wgpu automation content
        let grid_rect = egui::Rect::from_min_max(
            egui::pos2(panel_rect.min.x + left_panel_width, panel_rect.min.y),
            panel_rect.max,
        );

        let gw = grid_rect.width() as u32;
        let gh = grid_rect.height() as u32;

        if gw > 0 && gh > 0 {
            if let Some((renderer, render_ctx)) = renderers.get_mut(i) {
                render_ctx.ensure_size(gw, gh);

                let lane = automation_lanes
                    .iter()
                    .find(|l| l.target == panel.selected_target);

                let force_rebuild = panel.dirty;
                let gpu_dirty = prepare_automation(
                    renderer,
                    gw,
                    gh,
                    panel,
                    lane,
                    tpb,
                    default_num,
                    default_den,
                    time_sig_events,
                    force_rebuild,
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

        // Drag handle at bottom of panel
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(panel_rect.min.x + left_panel_width, panel_rect.max.y - SPLIT_H),
            panel_rect.max,
        );
        let handle_id = ui.id().with(format!("auto_handle_{}", i));
        let handle_resp = ui.interact(handle_rect, handle_id, egui::Sense::click_and_drag());
        ui.painter().rect_filled(
            handle_rect,
            0.0,
            if handle_resp.hovered() || handle_resp.dragged() {
                theme::SPLIT_HOVER
            } else {
                theme::SPLIT_DEFAULT
            },
        );
        if handle_resp.hovered() || handle_resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if handle_resp.dragged() {
            let delta = handle_resp.drag_delta().y;
            let new_h = (panel.panel_height + delta).clamp(
                yinhe_pianoroll::automation_view::MIN_PANEL_HEIGHT,
                yinhe_pianoroll::automation_view::MAX_PANEL_HEIGHT,
            );
            panel.panel_height = new_h;
            panel.dirty = true;
            ui.ctx().request_repaint();
        }

        y_offset += panel.panel_height;
    }

    y_offset - content_top_y
}

/// Show the toggle / add / remove buttons in the keyboard area below the scrollbar.
pub fn show_toggle_buttons(
    ui: &mut egui::Ui,
    show_panels: &mut bool,
    panel_count: &mut usize,
    btn_rect: egui::Rect,
) {
    // Toggle button
    let toggle_color = if *show_panels {
        theme::ACCENT_ACTIVE
    } else {
        egui::Color32::GRAY
    };
    let toggle_label = egui::RichText::new("AUTO").size(theme::MODE_LABEL_FONT).color(toggle_color);
    let toggle_resp = ui.add(
        egui::Label::new(toggle_label)
            .sense(egui::Sense::click())
            .selectable(false),
    );
    if !*show_panels && toggle_resp.hovered() {
        ui.painter().text(
            toggle_resp.rect.center(),
            egui::Align2::CENTER_CENTER,
            "AUTO",
            egui::FontId::proportional(theme::MODE_LABEL_FONT),
            egui::Color32::WHITE,
        );
    }
    if toggle_resp.clicked() {
        *show_panels = !*show_panels;
        if *show_panels && *panel_count == 0 {
            *panel_count = 1;
        }
    }

    if *show_panels {
        ui.add_space(4.0);

        // + button
        let plus_color = egui::Color32::GRAY;
        let plus_resp = ui.add(
            egui::Label::new(
                egui::RichText::new("+").size(theme::MODE_LABEL_FONT + 2.0).color(plus_color),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        if plus_resp.hovered() {
            ui.painter().text(
                plus_resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(theme::MODE_LABEL_FONT + 2.0),
                egui::Color32::WHITE,
            );
        }
        if plus_resp.clicked() {
            *panel_count += 1;
        }

        // - button
        let minus_resp = ui.add(
            egui::Label::new(
                egui::RichText::new("-").size(theme::MODE_LABEL_FONT + 2.0).color(plus_color),
            )
            .sense(egui::Sense::click())
            .selectable(false),
        );
        if minus_resp.hovered() {
            ui.painter().text(
                minus_resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                "-",
                egui::FontId::proportional(theme::MODE_LABEL_FONT + 2.0),
                egui::Color32::WHITE,
            );
        }
        if minus_resp.clicked() && *panel_count > 0 {
            *panel_count -= 1;
        }
    }
}
