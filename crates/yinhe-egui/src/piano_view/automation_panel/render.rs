use super::types::PanelOverlayData;
use crate::render_context::RenderContext;
use crate::right_panel::InfoContent;
use eframe::egui;
use rust_i18n::t;
use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};
use yinhe_wgpu::{AutomationGhost, InstanceRenderer, prepare_automation};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_panel_content(
    ui: &mut egui::Ui,
    renderer: &mut InstanceRenderer,
    render_ctx: &mut RenderContext,
    panel: &mut AutomationPanelView,
    grid_rect: egui::Rect,
    gpw: u32,
    gph: u32,
    render_lanes: &[&AutomationLane],
    tempo_lane: &AutomationLane,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    min_border_width: f32,
    show_anchors: bool,
    max_val_f: f32,
    panel_ghost: Option<AutomationGhost>,
    revision: u64,
    info_content: &Option<InfoContent>,
    panel_index: usize,
    combo_width: f32,
) {
    let gw = grid_rect.width() as u32;
    let gh = grid_rect.height() as u32;
    render_ctx.ensure_size(gpw, gph);
    let lanes: Vec<&AutomationLane> = if panel.selected_target == AutomationTarget::Tempo {
        vec![tempo_lane]
    } else {
        render_lanes
            .iter()
            .filter(|l| l.target == panel.selected_target)
            .copied()
            .collect()
    };
    let mut highlight_ticks: Vec<u32> = Vec::new();
    for l in &lanes {
        for e in &l.events {
            if panel
                .anchor_sel_rects
                .iter()
                .any(|r| r.contains(e.tick, e.value))
            {
                highlight_ticks.push(e.tick);
            }
        }
    }
    if let Some(InfoContent::Anchor {
        target: anchor_target,
        track_idx,
        event_idx,
        ..
    }) = info_content
        && *anchor_target == panel.selected_target
        && let Some(tick) = lanes
            .iter()
            .find(|l| {
                l.target == panel.selected_target
                    && (l.target == AutomationTarget::Tempo || l.track == *track_idx)
            })
            .and_then(|l| l.events.get(*event_idx))
            .map(|e| e.tick)
        && !highlight_ticks.contains(&tick)
    {
        highlight_ticks.push(tick);
    }
    let gpu_dirty = prepare_automation(
        renderer,
        gw,
        gh,
        panel,
        &lanes,
        midi,
        track_visible,
        track_colors,
        min_border_width,
        show_anchors,
        max_val_f,
        panel_ghost,
        revision,
        &highlight_ticks,
    );
    let content_changed = panel.dirty || gpu_dirty;
    panel.dirty = false;
    let painter = ui.painter();
    let theme = renderer.theme();
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(grid_rect.min.x + combo_width, grid_rect.min.y),
        grid_rect.max,
    );
    painter.rect_filled(content_rect, 0.0, crate::theme::stripe_bg());
    if !panel.show_velocity {
        let target = &panel.selected_target;
        let max_val = target.max_value();
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value();
            let y_center = panel.value_to_y(center_val, max_val);
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(content_rect.min.x, content_rect.min.y + y_center - 0.5),
                    egui::vec2(content_rect.width(), 1.0),
                ),
                0.0,
                crate::theme::rgba_to_color32(theme.center_line),
            );
        }
    }
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let grid_draw_rect = egui::Rect::from_min_max(
            egui::pos2(grid_rect.min.x + combo_width, grid_rect.min.y),
            grid_rect.max,
        );
        crate::widgets::grid_lines::paint_grid_lines(
            painter,
            grid_draw_rect,
            &panel.base,
            tpb,
            def_num,
            def_den,
            sig_events,
            &crate::widgets::grid_lines::GridColors::pianoroll(),
            yinhe_types::Orientation::Horizontal,
        );
    }
    render_ctx.paint(
        renderer,
        gpw,
        gph,
        &format!("auto_panel_{}", panel_index),
        painter,
        grid_rect,
        content_changed,
    );
}
pub(crate) fn draw_panel_overlay(
    ui: &mut egui::Ui,
    panel: &AutomationPanelView,
    panel_rect: egui::Rect,
    grid_area: egui::Rect,
    max_val_f: f32,
    panel_idx: usize,
    overlay: &PanelOverlayData,
) {
    if let Some(preview) = &overlay.velocity_preview {
        let painter = ui.painter();
        for bar in &preview.bars {
            painter.rect_filled(*bar, 0.0, preview.color.gamma_multiply(0.85));
            painter.line_segment(
                [bar.left_top(), bar.right_top()],
                egui::Stroke::new(1.0, crate::theme::contrast_fg()),
            );
        }
    }
    if overlay.marquee_rect.is_none() {
        let x_offset = grid_area.min.x - panel.base.scroll_x;
        let ppu = panel.base.pixels_per_tick;
        let move_offset_id = ui.id().with("auto_move_offset").with(panel_idx);
        let (d_tick, d_value) = ui
            .ctx()
            .data(|d| d.get_temp::<(i64, f32)>(move_offset_id))
            .unwrap_or((0, 0.0));
        let painter = ui.painter();
        for sel_rect in &panel.anchor_sel_rects {
            let ts = (sel_rect.tick_start.min(sel_rect.tick_end) + d_tick as f64).max(0.0);
            let te = (sel_rect.tick_start.max(sel_rect.tick_end) + d_tick as f64).max(0.0);
            let x1 = x_offset + (ts as f32) * ppu;
            let x2 = x_offset + (te as f32) * ppu;
            let (y1, y2) = match sel_rect.value_range {
                None => (grid_area.min.y, grid_area.max.y),
                Some((vmin, vmax)) => {
                    let v1 = (vmin + d_value).clamp(0.0, max_val_f);
                    let v2 = (vmax + d_value).clamp(0.0, max_val_f);
                    let ya = panel_rect.min.y + panel.value_to_y(v2, max_val_f);
                    let yb = panel_rect.min.y + panel.value_to_y(v1, max_val_f);
                    (ya.min(yb), ya.max(yb))
                }
            };
            let rect = egui::Rect::from_min_max(egui::pos2(x1, y1), egui::pos2(x2, y2))
                .intersect(grid_area);
            painter.rect_filled(
                rect,
                0.0,
                crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_fill_alpha()),
            );
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(
                    1.0,
                    crate::theme::contrast_fg()
                        .gamma_multiply(crate::theme::marquee_stroke_alpha()),
                ),
                egui::StrokeKind::Inside,
            );
        }
    }
    if let Some(rect) = overlay.marquee_rect {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            0.0,
            crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_fill_alpha()),
        );
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                1.0,
                crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_stroke_alpha()),
            ),
            egui::StrokeKind::Inside,
        );
    }
}
pub(crate) fn draw_value_labels(
    ui: &mut egui::Ui,
    panel: &AutomationPanelView,
    panel_rect: egui::Rect,
    combo_width: f32,
    max_val_f: f32,
) {
    let name = if panel.show_velocity {
        t!("automation.velocity").to_string()
    } else {
        panel.selected_target.display_name()
    };
    let label_color = crate::theme::measure_label();
    let font_id = egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT);
    let pad_x = 4.0;
    let label_max = if panel.show_velocity || panel.selected_target == AutomationTarget::Tempo {
        max_val_f
    } else {
        panel.selected_target.max_value()
    };
    let h = panel_rect.height();
    let (top_val, mid_val, bot_val) = (
        panel.y_to_value(0.0, label_max).round() as u32,
        panel.y_to_value(h * 0.5, label_max).round() as u32,
        panel.y_to_value(h, label_max).round() as u32,
    );
    let (top_val, mid_val, bot_val) = (
        top_val.to_string(),
        mid_val.to_string(),
        bot_val.to_string(),
    );
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
    let name_x = panel_rect.min.x + combo_width + 40.0;
    painter.text(
        egui::pos2(name_x, bot_y),
        egui::Align2::LEFT_BOTTOM,
        &name,
        font_id.clone(),
        label_color,
    );
}
