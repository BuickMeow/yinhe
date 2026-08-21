use std::sync::Arc;

use eframe::egui;

use yinhe_types::AutomationPanelView;
use yinhe_wgpu::InstanceRenderer;

use crate::render_context::RenderContext;
use crate::theme;

use super::constants::SPLIT_H;
use super::types::PanelsLayout;

/// Ensure `renderers` has the same count as `panels`, creating/destroying as needed.
pub(crate) fn sync_renderer_count(
    renderers: &mut Vec<(InstanceRenderer, RenderContext)>,
    panels: &[AutomationPanelView],
    wgpu_state: &Arc<eframe::egui_wgpu::RenderState>,
    default_w: u32,
    default_h: u32,
) {
    while renderers.len() < panels.len() {
        let renderer = InstanceRenderer::new(
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

#[allow(dead_code)]
pub(crate) struct FrameCtx {
    pub orig_heights: Vec<f32>,
    pub max_scroll: f32,
    pub scroll_y: f32,
    pub panels_area_rect: egui::Rect,
    pub old_clip: egui::Rect,
    pub vbar_rect: egui::Rect,
    pub y_offset: f32,
    pub visible_top: f32,
    pub visible_bottom: f32,
}

/// Prepare scroll state, clip rect and background for the panels area.
///
/// Handles renderer count sync, scroll overflow, clip and scrollbar background.
/// Returns frame context used by the per-panel loop.
pub(crate) fn begin_frame(
    ui: &mut egui::Ui,
    panels: &[AutomationPanelView],
    layout: PanelsLayout,
) -> FrameCtx {
    let orig_heights: Vec<f32> = panels.iter().map(|p| p.panel_height).collect();
    let panels_natural_h: f32 = orig_heights.iter().sum::<f32>() + (panels.len() as f32 * SPLIT_H);
    let max_scroll = (panels_natural_h - layout.panels_visible_h).max(0.0);
    let scroll_id = ui.id().with("auto_panel_scroll_y");
    let mut scroll_y: f32 = ui.data_mut(|d| d.get_persisted(scroll_id)).unwrap_or(0.0);
    scroll_y = scroll_y.clamp(0.0, max_scroll);
    let panels_area_rect = egui::Rect::from_min_max(
        egui::pos2(0.0, layout.content_top_y),
        egui::pos2(
            layout.content_rect_right,
            layout.content_top_y + layout.panels_visible_h,
        ),
    );
    let pointer_in_panels = crate::view_interaction::pointer_hits(ui, panels_area_rect);
    if pointer_in_panels && max_scroll > 0.0 {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        scroll_y = (scroll_y - scroll_delta.y).clamp(0.0, max_scroll);
    }
    ui.data_mut(|d| d.insert_persisted(scroll_id, scroll_y));
    let old_clip = ui.clip_rect();
    ui.set_clip_rect(panels_area_rect.intersect(old_clip));
    let vbar_rect = egui::Rect::from_min_max(
        egui::pos2(
            layout.content_rect_right - crate::widgets::scrollbar::SCROLLBAR_W,
            layout.content_top_y,
        ),
        egui::pos2(
            layout.content_rect_right,
            layout.content_top_y + layout.panels_visible_h,
        ),
    );
    ui.painter().rect_filled(vbar_rect, 0.0, theme::track_bg());
    let y_offset = layout.content_top_y - scroll_y;
    let visible_top = layout.content_top_y;
    let visible_bottom = layout.content_top_y + layout.panels_visible_h;
    FrameCtx {
        orig_heights,
        max_scroll,
        scroll_y,
        panels_area_rect,
        old_clip,
        vbar_rect,
        y_offset,
        visible_top,
        visible_bottom,
    }
}
