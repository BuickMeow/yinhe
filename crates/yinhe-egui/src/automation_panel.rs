use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::*;

use yinhe_types::AutomationLane;

use yinhe_automation::{AutomationPanelView, PianorollRenderer, prepare_automation};

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
            crate::split_handle::horizontal(ui, format!("auto_handle_{}", i), handle_rect);
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

        // ── Left side: dropdown area (drawn on top of grid) ──
        let combo_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(panel_rect.min.x + combo_width, panel_rect.max.y),
        );

        // Draw left panel background (covers the grid underneath)
        ui.painter().rect_filled(combo_rect, 0.0, theme::APP_BG);

        // ComboBox for target selection
        let combo_inner = combo_rect.shrink(4.0);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(combo_inner), |ui| {
            egui::ComboBox::from_id_salt(ui.id().with(format!("auto_combo_{}", i)))
                .selected_text(panel.selected_target.display_name())
                .width(combo_inner.width())
                .show_ui(ui, |ui| {
                    for (idx, lane) in automation_lanes.iter().enumerate() {
                        let name = lane.target.display_name();
                        if ui
                            .selectable_label(panel.selected_target == lane.target, &name)
                            .clicked()
                        {
                            panel.selected_target = lane.target.clone();
                            panel.lane_index = idx;
                            panel.dirty = true;
                        }
                    }
                });
        });

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
    if !*show_panels && toggle_resp.hovered() {
        ui.painter().text(
            toggle_resp.rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_SIGNAL_CELLULAR_ALT.codepoint,
            egui::FontId::new(
                theme::MODE_LABEL_FONT + 2.0,
                ICON_SIGNAL_CELLULAR_ALT.font_family(),
            ),
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
        if plus_resp.hovered() {
            ui.painter().text(
                plus_resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                ICON_ADD.codepoint,
                egui::FontId::new(theme::MODE_LABEL_FONT + 2.0, ICON_ADD.font_family()),
                egui::Color32::WHITE,
            );
        }
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
        if minus_resp.hovered() {
            ui.painter().text(
                minus_resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                ICON_REMOVE.codepoint,
                egui::FontId::new(theme::MODE_LABEL_FONT + 2.0, ICON_REMOVE.font_family()),
                egui::Color32::WHITE,
            );
        }
        if minus_resp.clicked() && *panel_count > 0 {
            *panel_count -= 1;
        }
    }
}
