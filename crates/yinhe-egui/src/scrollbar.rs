use eframe::egui;

use crate::theme;

// ── Constants ──

/// Height of the scrollbar band.
pub(crate) const SCROLLBAR_H: f32 = theme::SCROLLBAR_H;

const BG_COLOR: egui::Color32 = theme::SCROLLBAR_BG;
const RECT_COLOR: egui::Color32 = theme::SCROLLBAR_RECT;
const RECT_HOVER_COLOR: egui::Color32 = theme::SCROLLBAR_HOVER;
const RECT_DRAG_COLOR: egui::Color32 = theme::SCROLLBAR_DRAG;
const EDGE_WIDTH: f32 = 4.0;

/// Pixel-range allowed for `pixels_per_tick`.
const PPT_MIN: f32 = 0.001;
const PPT_MAX: f32 = 10.0;

// ── Public API ──

/// Paint a horizontal scrollbar into the given rect.
///
/// The scrollbar represents the full timeline; a draggable rectangle
/// shows the current viewport.  Dragging the middle pans, dragging
/// either edge zooms (anchored on the opposite edge).
///
/// `view_width` is the pixel-width of the content area (right of the
/// keyboard / track-panel).
pub(crate) fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view_width: f32,
    scroll_x: &mut f32,
    pixels_per_tick: &mut f32,
    total_ticks: f64,
    dirty: &mut bool,
) {
    let sb_w = rect.width();
    if sb_w <= 0.0 || total_ticks <= 0.0 {
        return;
    }

    // Clamp scroll_x BEFORE computing the rectangle visual, so the
    // scrollbar never renders an out-of-bounds position.  Without this,
    // momentum/inertia scrolling from `handle_input` can push scroll_x
    // past [0, max] after the caller's clamp_scroll, producing a visible
    // one-frame bounce-back effect (same root cause as the ruler bounce
    // fixed in arrange.rs).
    let max_scroll_x = |ppt: f32| (total_ticks as f32 * ppt - view_width).max(0.0);
    *scroll_x = scroll_x.clamp(0.0, max_scroll_x(*pixels_per_tick));

    // Scale: scrollbar pixels per MIDI tick.
    let scale = sb_w as f64 / total_ticks;

    // ── Rectangle position and size (derived from current view state) ──

    let start_tick = *scroll_x as f64 / *pixels_per_tick as f64;
    let viewport_ticks = view_width as f64 / *pixels_per_tick as f64;

    let rect_left = (start_tick * scale) as f32;
    let rect_width = (viewport_ticks * scale) as f32;
    let rect_right = rect_left + rect_width;

    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, BG_COLOR);

    // ── Rectangle visual ──
    let rect_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + rect_left, rect.min.y),
        egui::pos2((rect.min.x + rect_right).min(rect.max.x), rect.max.y),
    );

    // Three interaction zones
    let left_edge_rect = egui::Rect::from_min_max(
        rect_rect.min,
        egui::pos2(
            (rect_rect.min.x + EDGE_WIDTH).min(rect_rect.max.x),
            rect_rect.max.y,
        ),
    );
    let right_edge_rect = egui::Rect::from_min_max(
        egui::pos2(
            (rect_rect.max.x - EDGE_WIDTH).max(rect_rect.min.x),
            rect_rect.min.y,
        ),
        rect_rect.max,
    );
    let middle_rect = egui::Rect::from_min_max(
        egui::pos2(left_edge_rect.max.x, rect_rect.min.y),
        egui::pos2(right_edge_rect.min.x, rect_rect.max.y),
    );

    let edge_id_left = ui.id().with("__sb_left__");
    let edge_id_right = ui.id().with("__sb_right__");
    let middle_id = ui.id().with("__sb_mid__");

    let left_resp = ui.interact(left_edge_rect, edge_id_left, egui::Sense::click_and_drag());
    let right_resp = ui.interact(
        right_edge_rect,
        edge_id_right,
        egui::Sense::click_and_drag(),
    );
    let middle_resp = ui.interact(middle_rect, middle_id, egui::Sense::click_and_drag());

    let left_hovered = left_resp.hovered() || left_resp.dragged();
    let right_hovered = right_resp.hovered() || right_resp.dragged();
    let middle_hovered = middle_resp.hovered() || middle_resp.dragged();

    // Paint rectangle with appropriate color
    let rect_color = if left_resp.dragged() || right_resp.dragged() {
        RECT_DRAG_COLOR
    } else if middle_resp.dragged() {
        RECT_DRAG_COLOR
    } else if middle_hovered || left_hovered || right_hovered {
        RECT_HOVER_COLOR
    } else {
        RECT_COLOR
    };
    ui.painter().rect_filled(rect_rect, 2.0, rect_color);

    // ── Cursor ──
    if left_hovered || right_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    } else if middle_hovered || middle_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──

    // Drag middle → pan
    if middle_resp.dragged() {
        let delta = middle_resp.drag_delta().x;
        let delta_ticks = delta as f64 / scale;
        *scroll_x = (*scroll_x as f64 + delta_ticks * *pixels_per_tick as f64) as f32;
        *scroll_x = scroll_x.clamp(0.0, max_scroll_x(*pixels_per_tick));
        *dirty = true;
        ui.ctx().request_repaint();
        return;
    }

    // Apply zoom, clamping both ppt and scroll_x so the rectangle never
    // overshoots; this avoids a one-frame bounce when the caller's
    // clamp_scroll runs on the next frame.
    let mut apply_zoom =
        |scroll_x: &mut f32, ppt: &mut f32, new_start_tick: f64, new_viewport_ticks: f64| {
            let new_ppt = (view_width as f64 / new_viewport_ticks)
                .clamp(PPT_MIN as f64, PPT_MAX as f64) as f32;
            let new_scroll_x = (new_start_tick * new_ppt as f64) as f32;
            *ppt = new_ppt;
            *scroll_x = new_scroll_x.clamp(0.0, max_scroll_x(new_ppt));
            *dirty = true;
        };

    // Drag left edge → zoom, anchoring at right edge
    if left_resp.dragged() {
        let new_left =
            (rect_left + left_resp.drag_delta().x).clamp(0.0, rect_right - 2.0 * EDGE_WIDTH);
        let new_start_tick = new_left as f64 / scale;
        let right_tick = start_tick + viewport_ticks;
        let new_viewport_ticks = (right_tick - new_start_tick).max(1.0);
        apply_zoom(
            scroll_x,
            pixels_per_tick,
            new_start_tick,
            new_viewport_ticks,
        );
        ui.ctx().request_repaint();
        return;
    }

    // Drag right edge → zoom, anchoring at left edge
    if right_resp.dragged() {
        let new_right =
            (rect_right + right_resp.drag_delta().x).clamp(rect_left + 2.0 * EDGE_WIDTH, sb_w);
        let new_right_tick = new_right as f64 / scale;
        let new_viewport_ticks = (new_right_tick - start_tick).max(1.0);
        apply_zoom(scroll_x, pixels_per_tick, start_tick, new_viewport_ticks);
        ui.ctx().request_repaint();
    }
}
