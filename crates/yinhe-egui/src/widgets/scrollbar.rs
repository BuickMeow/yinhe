use eframe::egui;

use crate::theme;

// ── Constants ──

/// Height of the horizontal scrollbar band.
pub(crate) const SCROLLBAR_H: f32 = theme::SCROLLBAR_H;
/// Width of the vertical scrollbar band.
pub(crate) const SCROLLBAR_W: f32 = theme::SCROLLBAR_W;

const BG_COLOR: egui::Color32 = theme::SCROLLBAR_BG;
const RECT_COLOR: egui::Color32 = theme::SCROLLBAR_RECT;
const RECT_HOVER_COLOR: egui::Color32 = theme::SCROLLBAR_HOVER;
const RECT_DRAG_COLOR: egui::Color32 = theme::SCROLLBAR_DRAG;
const EDGE_WIDTH: f32 = 4.0;

/// Pixel-range allowed for `pixels_per_tick`.
const PPT_MIN: f32 = 0.001;
const PPT_MAX: f32 = 10.0;

// ── Horizontal scrollbar ──

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
    let rect_color = if left_resp.dragged() || right_resp.dragged() || middle_resp.dragged() {
        RECT_DRAG_COLOR
    } else if middle_hovered || left_hovered || right_hovered {
        RECT_HOVER_COLOR
    } else {
        RECT_COLOR
    };
    ui.painter().rect_filled(rect_rect, 2.0, rect_color);

    // ── Cursor ──
    if left_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest);
    } else if right_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast);
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

// ── Vertical scrollbar (pixel-space) ──

/// 垂直滚动条（像素空间）：用于 AR（lane_height + scroll_y）和 PR（key_height + scroll_y）。
///
/// 总范围 = `total_pixels`（如 `num_tracks * lane_height` 或 `128 * key_height`）。
/// 视口 = `view_height` 像素。`cell_size` = 每个单元的像素高度（lane_height / key_height）。
///
/// 三区交互（与水平滚动条对称）：
/// - 中间拖动 → 平移 scroll_y
/// - 顶边拖动 → 缩放 cell_size，锚定底边
/// - 底边拖动 → 缩放 cell_size，锚定顶边
///
/// `cell_min` / `cell_max` = cell_size 的最小/最大值。
/// `scroll_y` / `cell_size` 会被原地修改；`dirty` 标记视图为脏。
///
/// 当 `total_pixels <= view_height`（内容能一屏装下）时，不绘制任何内容。
pub(crate) fn show_vertical(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view_height: f32,
    scroll_y: &mut f32,
    cell_size: &mut f32,
    total_pixels: f32,
    cell_min: f32,
    cell_max: f32,
    dirty: &mut bool,
) {
    let sb_h = rect.height();
    if sb_h <= 0.0 || view_height <= 0.0 || total_pixels <= view_height {
        return;
    }

    // Clamp scroll_y BEFORE computing the rectangle visual.
    // max_scroll_y 只与 total_pixels 和 view_height 相关（与 cell_size 无关）。
    let max_scroll_y = (total_pixels - view_height).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll_y);

    // Scale: scrollbar pixels per content pixel.
    let scale = sb_h / total_pixels;

    // ── Rectangle position and size (derived from current view state) ──
    let start_pixel = *scroll_y;
    let viewport_pixels = view_height;

    let rect_top = (start_pixel * scale).max(0.0);
    let rect_height = (viewport_pixels * scale).min(sb_h - rect_top);
    let rect_bottom = rect_top + rect_height;

    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, BG_COLOR);

    // ── Rectangle visual ──
    let rect_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y + rect_top),
        egui::pos2(rect.max.x, (rect.min.y + rect_bottom).min(rect.max.y)),
    );

    // Three interaction zones (top edge / middle / bottom edge)
    let top_edge_rect = egui::Rect::from_min_max(
        rect_rect.min,
        egui::pos2(
            rect_rect.max.x,
            (rect_rect.min.y + EDGE_WIDTH).min(rect_rect.max.y),
        ),
    );
    let bottom_edge_rect = egui::Rect::from_min_max(
        egui::pos2(
            rect_rect.min.x,
            (rect_rect.max.y - EDGE_WIDTH).max(rect_rect.min.y),
        ),
        rect_rect.max,
    );
    let middle_rect = egui::Rect::from_min_max(
        egui::pos2(rect_rect.min.x, top_edge_rect.max.y),
        egui::pos2(rect_rect.max.x, bottom_edge_rect.min.y),
    );

    let edge_id_top = ui.id().with("__vsb_top__");
    let edge_id_bottom = ui.id().with("__vsb_bottom__");
    let middle_id = ui.id().with("__vsb_mid__");

    let top_resp = ui.interact(top_edge_rect, edge_id_top, egui::Sense::click_and_drag());
    let bottom_resp = ui.interact(
        bottom_edge_rect,
        edge_id_bottom,
        egui::Sense::click_and_drag(),
    );
    let middle_resp = ui.interact(middle_rect, middle_id, egui::Sense::click_and_drag());

    let top_hovered = top_resp.hovered() || top_resp.dragged();
    let bottom_hovered = bottom_resp.hovered() || bottom_resp.dragged();
    let middle_hovered = middle_resp.hovered() || middle_resp.dragged();

    // Paint rectangle with appropriate color
    let rect_color = if top_resp.dragged() || bottom_resp.dragged() || middle_resp.dragged() {
        RECT_DRAG_COLOR
    } else if middle_hovered || top_hovered || bottom_hovered {
        RECT_HOVER_COLOR
    } else {
        RECT_COLOR
    };
    ui.painter().rect_filled(rect_rect, 2.0, rect_color);

    // ── Cursor ──
    if top_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNorth);
    } else if bottom_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouth);
    } else if middle_hovered || middle_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──

    // Drag middle → pan
    if middle_resp.dragged() {
        let delta = middle_resp.drag_delta().y;
        *scroll_y = (*scroll_y + delta) .clamp(0.0, max_scroll_y);
        *dirty = true;
        ui.ctx().request_repaint();
        return;
    }

    // Apply zoom: 新的 cell_size 和 scroll_y，确保 thumb 不超出滚动条。
    // 锚定策略：拖顶边时固定底边内容位置；拖底边时固定顶边内容位置。
    // 注意：cell_size 改变后 total_pixels 也会变（因为 total_pixels = num_cells * cell_size），
    // 但我们在调用前已经传入当前的 total_pixels，所以这里只用 clamping 防止 scroll_y 越界。
    let mut apply_zoom = |scroll_y: &mut f32, cs: &mut f32, new_start_pixel: f32, new_viewport_pixels: f32| {
        let new_cs = (view_height / new_viewport_pixels).clamp(cell_min, cell_max);
        // 缩放后总像素可能变化，重新计算 scroll_y
        let new_scroll_y = new_start_pixel.clamp(0.0, max_scroll_y);
        *cs = new_cs;
        *scroll_y = new_scroll_y;
        *dirty = true;
    };

    // Drag top edge → zoom, anchoring at bottom edge
    if top_resp.dragged() {
        let new_top =
            (rect_top + top_resp.drag_delta().y).clamp(0.0, rect_bottom - 2.0 * EDGE_WIDTH);
        let new_start_pixel = new_top / scale;
        let bottom_pixel = start_pixel + viewport_pixels;
        let new_viewport_pixels = (bottom_pixel - new_start_pixel).max(1.0);
        apply_zoom(scroll_y, cell_size, new_start_pixel, new_viewport_pixels);
        ui.ctx().request_repaint();
        return;
    }

    // Drag bottom edge → zoom, anchoring at top edge
    if bottom_resp.dragged() {
        let new_bottom =
            (rect_bottom + bottom_resp.drag_delta().y).clamp(rect_top + 2.0 * EDGE_WIDTH, sb_h);
        let new_bottom_pixel = new_bottom / scale;
        let new_viewport_pixels = (new_bottom_pixel - start_pixel).max(1.0);
        apply_zoom(scroll_y, cell_size, start_pixel, new_viewport_pixels);
        ui.ctx().request_repaint();
    }
}

// ── Vertical scrollbar (value-space, for automation panels) ──

/// 垂直滚动条（值空间）：用于 AM 自动化面板（value_zoom + value_scroll）。
///
/// 与像素空间不同，自动化面板的"总范围"是 `total_value`（如 CC=127, Tempo=60M），
/// `value_zoom` 是倍数（visible_range = total_value / value_zoom）。
///
/// 三区交互：
/// - 中间拖动 → 平移 value_scroll
/// - 顶边/底边拖动 → 缩放 value_zoom（顶边固定底部值，底边固定顶部值）
///
/// `zoom_min` / `zoom_max` = value_zoom 的范围。`total_value` = 值上限（upper_bound）。
pub(crate) fn show_vertical_value(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    panel_height: f32,
    value_scroll: &mut f32,
    value_zoom: &mut f32,
    total_value: f32,
    zoom_min: f32,
    zoom_max: f32,
    dirty: &mut bool,
) {
    let sb_h = rect.height();
    if sb_h <= 0.0 || panel_height <= 0.0 || total_value <= 0.0 {
        return;
    }

    // visible_range 必须小于 total_value 才有滚动需求
    let visible_range = total_value / *value_zoom;
    if visible_range >= total_value {
        return;
    }

    // Clamp value_scroll
    let max_scroll = (total_value - visible_range).max(0.0);
    *value_scroll = value_scroll.clamp(0.0, max_scroll);

    // Scale: scrollbar pixels per value unit.
    let scale = sb_h / total_value;

    // ── Rectangle position and size ──
    // value 0 在底部，total_value 在顶部（与面板渲染一致：value_to_y 中 h - (...)）
    let top_value = *value_scroll + visible_range; // 面板顶部对应的值
    let bottom_value = *value_scroll; // 面板底部对应的值

    let rect_top = ((total_value - top_value) * scale).max(0.0);
    let rect_bottom = ((total_value - bottom_value) * scale).min(sb_h);
    let rect_height = (rect_bottom - rect_top).max(0.0);

    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, BG_COLOR);

    let rect_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y + rect_top),
        egui::pos2(rect.max.x, rect.min.y + rect_top + rect_height),
    );

    // Three interaction zones
    let top_edge_rect = egui::Rect::from_min_max(
        rect_rect.min,
        egui::pos2(
            rect_rect.max.x,
            (rect_rect.min.y + EDGE_WIDTH).min(rect_rect.max.y),
        ),
    );
    let bottom_edge_rect = egui::Rect::from_min_max(
        egui::pos2(
            rect_rect.min.x,
            (rect_rect.max.y - EDGE_WIDTH).max(rect_rect.min.y),
        ),
        rect_rect.max,
    );
    let middle_rect = egui::Rect::from_min_max(
        egui::pos2(rect_rect.min.x, top_edge_rect.max.y),
        egui::pos2(rect_rect.max.x, bottom_edge_rect.min.y),
    );

    let edge_id_top = ui.id().with("__vsb_v_top__");
    let edge_id_bottom = ui.id().with("__vsb_v_bottom__");
    let middle_id = ui.id().with("__vsb_v_mid__");

    let top_resp = ui.interact(top_edge_rect, edge_id_top, egui::Sense::click_and_drag());
    let bottom_resp = ui.interact(
        bottom_edge_rect,
        edge_id_bottom,
        egui::Sense::click_and_drag(),
    );
    let middle_resp = ui.interact(middle_rect, middle_id, egui::Sense::click_and_drag());

    let top_hovered = top_resp.hovered() || top_resp.dragged();
    let bottom_hovered = bottom_resp.hovered() || bottom_resp.dragged();
    let middle_hovered = middle_resp.hovered() || middle_resp.dragged();

    let rect_color = if top_resp.dragged() || bottom_resp.dragged() || middle_resp.dragged() {
        RECT_DRAG_COLOR
    } else if middle_hovered || top_hovered || bottom_hovered {
        RECT_HOVER_COLOR
    } else {
        RECT_COLOR
    };
    ui.painter().rect_filled(rect_rect, 2.0, rect_color);

    if top_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNorth);
    } else if bottom_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouth);
    } else if middle_hovered || middle_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──

    // Drag middle → pan value_scroll
    if middle_resp.dragged() {
        let delta = middle_resp.drag_delta().y;
        // y 增加 = 向下滚 = value_scroll 减小
        *value_scroll = (*value_scroll - delta / scale).clamp(0.0, max_scroll);
        *dirty = true;
        ui.ctx().request_repaint();
        return;
    }

    // Drag top edge → zoom, anchoring at bottom edge (固定 bottom_value)
    if top_resp.dragged() {
        let new_top_pixel = (rect_top + top_resp.drag_delta().y).clamp(0.0, rect_bottom - 2.0 * EDGE_WIDTH);
        let new_top_value = total_value - new_top_pixel / scale;
        let new_visible = (new_top_value - bottom_value).max(0.01);
        let new_z = (total_value / new_visible).clamp(zoom_min, zoom_max);
        let new_visible_clamped = total_value / new_z;
        // 固定底边，scroll = bottom_value
        let new_scroll = bottom_value.clamp(0.0, (total_value - new_visible_clamped).max(0.0));
        *value_zoom = new_z;
        *value_scroll = new_scroll;
        *dirty = true;
        ui.ctx().request_repaint();
        return;
    }

    // Drag bottom edge → zoom, anchoring at top edge (固定 top_value)
    if bottom_resp.dragged() {
        let new_bottom_pixel = (rect_bottom + bottom_resp.drag_delta().y).clamp(rect_top + 2.0 * EDGE_WIDTH, sb_h);
        let new_bottom_value = total_value - new_bottom_pixel / scale;
        let new_visible = (top_value - new_bottom_value).max(0.01);
        let new_z = (total_value / new_visible).clamp(zoom_min, zoom_max);
        let new_visible_clamped = total_value / new_z;
        // 固定顶边，scroll = top_value - new_visible
        let new_scroll = (top_value - new_visible_clamped).clamp(0.0, (total_value - new_visible_clamped).max(0.0));
        *value_zoom = new_z;
        *value_scroll = new_scroll;
        *dirty = true;
        ui.ctx().request_repaint();
    }
}
