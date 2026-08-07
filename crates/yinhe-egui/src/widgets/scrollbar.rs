use eframe::egui;

use crate::theme;

// ── Constants ──

/// Height of the horizontal scrollbar band.
pub(crate) const SCROLLBAR_H: f32 = theme::SCROLLBAR_H;
/// Width of the vertical scrollbar band.
pub(crate) const SCROLLBAR_W: f32 = theme::SCROLLBAR_W;

const EDGE_WIDTH: f32 = 4.0;

/// 滚动条四色（运行时读取当前主题，不能是 const——getter 非 const fn）。
fn colors() -> (egui::Color32, egui::Color32, egui::Color32, egui::Color32) {
    (
        theme::scrollbar_bg(),
        theme::scrollbar_rect(),
        theme::scrollbar_hover(),
        theme::scrollbar_drag(),
    )
}

/// Pixel-range allowed for `pixels_per_tick`.
const PPT_MIN: f32 = 0.001;
const PPT_MAX: f32 = 10.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// 跑一帧滚动条，返回背景拖拽返回值。
    fn run_frame(
        ctx: &egui::Context,
        raw: egui::RawInput,
        rect: egui::Rect,
        scroll_x: &mut f32,
        ppt: &mut f32,
        dirty: &mut bool,
    ) -> f32 {
        let mut out = 0.0f32;
        let _ = ctx.run_ui(raw, |ui| {
            out = show(ui, rect, 300.0, scroll_x, ppt, 1000.0, dirty);
        });
        out
    }

    fn press_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    fn drag_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw
    }

    /// 背景拖拽（thumb 之外）→ 返回垂直位移 dy，供调用处缩放对面轴。
    /// 配置：total=1000 tick，view_width=300，ppt=1 → thumb 占 [0, 90]，
    /// x=200 是背景区。
    /// egui 的 hit test 基于上一帧注册的 widgets：先 hover 注册一帧再 press。
    #[test]
    fn background_drag_returns_dy() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
        let mut scroll_x = 0.0f32;
        let mut ppt = 1.0f32;
        let mut dirty = false;

        let start = egui::pos2(200.0, 8.0);
        let end = egui::pos2(200.0, 28.0);
        // 帧1：hover 注册 widget（hit test 在下一帧生效）
        let _ = run_frame(
            &ctx,
            drag_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧2：press
        let _ = run_frame(
            &ctx,
            press_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧3：drag（本帧垂直移动 20px）→ 应返回 dy
        let dy = run_frame(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        assert!(dy > 0.0, "背景拖拽应返回非零 dy，实际 {dy}");
        assert_eq!(scroll_x, 0.0, "背景拖拽不应平移");
    }

    /// thumb 中间拖拽 = 平移，返回 0（不触发对面轴缩放）。
    #[test]
    fn thumb_drag_pans_and_returns_zero() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
        let mut scroll_x = 0.0f32;
        let mut ppt = 1.0f32;
        let mut dirty = false;

        // thumb 中间 (45, 8)，拖到 (95, 8)：平移 50px = 50/0.3 tick。
        let start = egui::pos2(45.0, 8.0);
        let end = egui::pos2(95.0, 8.0);
        // 帧1：hover 注册
        let _ = run_frame(
            &ctx,
            drag_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧2：press
        let _ = run_frame(
            &ctx,
            press_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧3：drag（本帧移动 50px）→ thumb 平移，返回 0
        let dx = run_frame(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        assert_eq!(dx, 0.0, "thumb 拖拽（平移）不应返回背景 dx");
        assert!(scroll_x > 0.0, "thumb 拖拽应平移 scroll_x");
    }

    /// thumb 上垂直拖动 → 返回 dy（缩放），不产生平移。
    #[test]
    fn thumb_vertical_drag_returns_dy() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
        let mut scroll_x = 0.0f32;
        let mut ppt = 1.0f32;
        let mut dirty = false;

        let start = egui::pos2(45.0, 8.0);
        let end = egui::pos2(45.0, 28.0);
        // 帧1：hover 注册
        let _ = run_frame(
            &ctx,
            drag_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧2：press
        let _ = run_frame(
            &ctx,
            press_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧3：drag（本帧垂直移动 20px）→ 返回 dy，x 未动 → 不平移
        let dy = run_frame(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        assert!(dy > 0.0, "thumb 上垂直拖应返回非零 dy，实际 {dy}");
        assert_eq!(scroll_x, 0.0, "纯垂直拖不得平移");
    }
}

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
) -> f32 {
    let sb_w = rect.width();
    if sb_w <= 0.0 || total_ticks <= 0.0 {
        return 0.0;
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

    let (bg_color, rect_color, rect_hover_color, rect_drag_color) = colors();
    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, bg_color);

    // ── Rectangle visual ──
    let rect_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + rect_left, rect.min.y),
        egui::pos2((rect.min.x + rect_right).min(rect.max.x), rect.max.y),
    );

    // 背景拖拽（thumb 之外的区域）→ 返回垂直位移 dy 供调用处缩放对面轴。
    // 注册在 thumb 交互之前：thumb 区域优先，背景兜底。
    let bg_id = ui.id().with("__sb_bg__");
    let bg_resp = ui.interact(rect, bg_id, egui::Sense::drag());

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
    let thumb_color = if left_resp.dragged() || right_resp.dragged() || middle_resp.dragged() {
        rect_drag_color
    } else if middle_hovered || left_hovered || right_hovered {
        rect_hover_color
    } else {
        rect_color
    };
    ui.painter().rect_filled(rect_rect, 2.0, thumb_color);

    // ── Cursor ──
    if left_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest);
    } else if right_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast);
    } else if middle_hovered || middle_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──

    // Drag middle → pan（x 方向）；垂直位移 → 返回 dy 供调用处缩放对面轴。
    // 斜拖 = 平移 + 缩放同时进行。
    if middle_resp.dragged() {
        let delta = middle_resp.drag_delta().x;
        let delta_ticks = delta as f64 / scale;
        *scroll_x = (*scroll_x as f64 + delta_ticks * *pixels_per_tick as f64) as f32;
        *scroll_x = scroll_x.clamp(0.0, max_scroll_x(*pixels_per_tick));
        *dirty = true;
        ui.ctx().request_repaint();
        return middle_resp.drag_delta().y;
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
        return 0.0;
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

    // 垂直位移 dy（thumb 中间或背景拖拽时）→ 调用处缩放对面轴（key 行高）
    if bg_resp.dragged() {
        bg_resp.drag_delta().y
    } else {
        0.0
    }
}

// ── Vertical scrollbar (pixel-space) ──

/// 垂直滚动条（像素空间）：用于 AR（lane_height + scroll_y）和 PR（key_height + scroll_y）。
///
/// 总范围 = `num_cells * cell_size`（如 `num_tracks * lane_height` 或 `128 * key_height`）。
/// 视口 = `view_height` 像素。`cell_size` = 每个单元的像素高度（lane_height / key_height）。
///
/// 三区交互（与水平滚动条对称）：
/// - 中间拖动 → 平移 scroll_y
/// - 顶边拖动 → 缩放 cell_size，锚定 thumb 底边 sb 位置
/// - 底边拖动 → 缩放 cell_size，锚定 thumb 顶边 sb 位置
///
/// `cell_min` / `cell_max` = cell_size 的最小/最大值。
/// `scroll_y` / `cell_size` 会被原地修改；`dirty` 标记视图为脏。
///
/// 即使 `total_pixels <= view_height`（内容一屏装下），也会绘制占满滚动条的 thumb，
/// 用户仍可拖动边缘缩放。只有 `max_scroll_y == 0` 时 pan 无效。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn show_vertical(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view_height: f32,
    scroll_y: &mut f32,
    cell_size: &mut f32,
    num_cells: usize,
    cell_min: f32,
    cell_max: f32,
    dirty: &mut bool,
) -> f32 {
    let sb_h = rect.height();
    if sb_h <= 0.0 || view_height <= 0.0 || num_cells == 0 {
        return 0.0;
    }

    let num_cells_f = num_cells as f32;
    let total_pixels = num_cells_f * *cell_size;

    // max_scroll_y：当 total_pixels <= view_height 时为 0（无滚动空间，但仍然绘制 thumb）
    let max_scroll_y = (total_pixels - view_height).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll_y);

    // Scale: scrollbar pixels per content pixel.
    // 当 total_pixels < view_height 时用 view_height 作为分母，让 thumb 占满整个滚动条。
    let scale = sb_h / total_pixels.max(view_height);

    // ── Rectangle position and size (derived from current view state) ──
    let rect_top = (*scroll_y * scale).clamp(0.0, sb_h);
    let rect_height = (view_height * scale).min(sb_h - rect_top);
    let rect_bottom = rect_top + rect_height;

    let (bg_color, rect_color, rect_hover_color, rect_drag_color) = colors();
    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, bg_color);

    // ── Rectangle visual ──
    let rect_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y + rect_top),
        egui::pos2(rect.max.x, (rect.min.y + rect_bottom).min(rect.max.y)),
    );

    // 背景拖拽（thumb 之外的区域）→ 返回水平位移 dx 供调用处缩放对面轴。
    // 注册在 thumb 交互之前：thumb 区域优先，背景兜底。
    let bg_id = ui.id().with("__vsb_bg__");
    let bg_resp = ui.interact(rect, bg_id, egui::Sense::drag());

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
    let thumb_color = if top_resp.dragged() || bottom_resp.dragged() || middle_resp.dragged() {
        rect_drag_color
    } else if middle_hovered || top_hovered || bottom_hovered {
        rect_hover_color
    } else {
        rect_color
    };
    ui.painter().rect_filled(rect_rect, 2.0, thumb_color);

    // ── Cursor ──
    if top_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNorth);
    } else if bottom_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouth);
    } else if middle_hovered || middle_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──
    //
    // 像素空间核心公式：thumb_height × cell_size = view_height × sb_h / num_cells = K
    // 因为 thumb_height = view_height × scale = view_height × sb_h / total_pixels
    //                  = view_height × sb_h / (num_cells × cell_size) = K / cell_size
    // 所以 cell_size 变化与 thumb_height 变化成反比。
    //
    // 拖边缘时，直接用 thumb_height 反比计算 new_cell_size，
    // 避免把 thumb 像素变化等同于 viewport_pixels 变化（那是 bug）。
    let k_constant = view_height * sb_h / num_cells_f;

    // Drag middle → pan（y 方向）；水平位移 → 返回 dx 供调用处缩放对面轴。
    // 斜拖 = 平移 + 缩放同时进行。
    if middle_resp.dragged() {
        if max_scroll_y > 0.0 {
            let delta = middle_resp.drag_delta().y;
            *scroll_y = (*scroll_y + delta / scale).clamp(0.0, max_scroll_y);
            *dirty = true;
            ui.ctx().request_repaint();
        }
        return middle_resp.drag_delta().x;
    }

    // Drag top edge → zoom，锚定 thumb 底边 sb 位置（rect_bottom 不动）
    if top_resp.dragged() {
        let new_thumb_top_sb =
            (rect_top + top_resp.drag_delta().y).clamp(0.0, rect_bottom - 2.0 * EDGE_WIDTH);
        let new_thumb_height_sb = (rect_bottom - new_thumb_top_sb).max(2.0 * EDGE_WIDTH);
        let new_cs = (k_constant / new_thumb_height_sb).clamp(cell_min, cell_max);
        // 重新计算（clamp 可能调整 cell_size）
        let new_scale = sb_h / (num_cells_f * new_cs);
        // 锚定 thumb 底边 sb 位置 = rect_bottom
        // (new_scroll_y + view_height) × new_scale = rect_bottom
        let new_scroll_y = rect_bottom / new_scale - view_height;
        let new_total_pixels = num_cells_f * new_cs;
        let max_sy = (new_total_pixels - view_height).max(0.0);
        *cell_size = new_cs;
        *scroll_y = new_scroll_y.clamp(0.0, max_sy);
        *dirty = true;
        ui.ctx().request_repaint();
        return 0.0;
    }

    // Drag bottom edge → zoom，锚定 thumb 顶边 sb 位置（rect_top 不动）
    if bottom_resp.dragged() {
        let new_thumb_bottom_sb =
            (rect_bottom + bottom_resp.drag_delta().y).clamp(rect_top + 2.0 * EDGE_WIDTH, sb_h);
        let new_thumb_height_sb = (new_thumb_bottom_sb - rect_top).max(2.0 * EDGE_WIDTH);
        let new_cs = (k_constant / new_thumb_height_sb).clamp(cell_min, cell_max);
        let new_scale = sb_h / (num_cells_f * new_cs);
        // 锚定 thumb 顶边 sb 位置 = rect_top
        // new_scroll_y × new_scale = rect_top
        let new_scroll_y = rect_top / new_scale;
        let new_total_pixels = num_cells_f * new_cs;
        let max_sy = (new_total_pixels - view_height).max(0.0);
        *cell_size = new_cs;
        *scroll_y = new_scroll_y.clamp(0.0, max_sy);
        *dirty = true;
        ui.ctx().request_repaint();
    }

    // 水平位移 dx（thumb 中间或背景拖拽时）→ 调用处缩放对面轴（tick 宽度）
    if bg_resp.dragged() {
        bg_resp.drag_delta().x
    } else {
        0.0
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
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
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

    let visible_range = total_value / *value_zoom;

    // Clamp value_scroll
    let max_scroll = (total_value - visible_range).max(0.0);
    *value_scroll = value_scroll.clamp(0.0, max_scroll);

    // Scale: scrollbar pixels per value unit.
    // 当 visible_range >= total_value（zoomed out）时用 visible_range 作分母，让 thumb 占满整个滚动条。
    let scale = sb_h / total_value.max(visible_range);

    // ── Rectangle position and size ──
    // value 0 在底部，total_value 在顶部（与面板渲染一致：value_to_y 中 h - (...)）
    let top_value = *value_scroll + visible_range; // 面板顶部对应的值
    let bottom_value = *value_scroll; // 面板底部对应的值

    let rect_top = ((total_value - top_value) * scale).max(0.0);
    let rect_bottom = ((total_value - bottom_value) * scale).min(sb_h);
    let rect_height = (rect_bottom - rect_top).max(0.0);

    let (bg_color, rect_color, rect_hover_color, rect_drag_color) = colors();
    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, bg_color);

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

    let thumb_color = if top_resp.dragged() || bottom_resp.dragged() || middle_resp.dragged() {
        rect_drag_color
    } else if middle_hovered || top_hovered || bottom_hovered {
        rect_hover_color
    } else {
        rect_color
    };
    ui.painter().rect_filled(rect_rect, 2.0, thumb_color);

    if top_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNorth);
    } else if bottom_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouth);
    } else if middle_hovered || middle_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──

    // Drag middle → pan value_scroll（仅当 max_scroll > 0 时有效）
    if middle_resp.dragged() && max_scroll > 0.0 {
        let delta = middle_resp.drag_delta().y;
        // y 增加 = 向下滚 = value_scroll 减小
        *value_scroll = (*value_scroll - delta / scale).clamp(0.0, max_scroll);
        *dirty = true;
        ui.ctx().request_repaint();
        return;
    }

    // Drag top edge → zoom, anchoring at bottom edge (固定 bottom_value)
    if top_resp.dragged() {
        let new_top_pixel =
            (rect_top + top_resp.drag_delta().y).clamp(0.0, rect_bottom - 2.0 * EDGE_WIDTH);
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
        let new_bottom_pixel =
            (rect_bottom + bottom_resp.drag_delta().y).clamp(rect_top + 2.0 * EDGE_WIDTH, sb_h);
        let new_bottom_value = total_value - new_bottom_pixel / scale;
        let new_visible = (top_value - new_bottom_value).max(0.01);
        let new_z = (total_value / new_visible).clamp(zoom_min, zoom_max);
        let new_visible_clamped = total_value / new_z;
        // 固定顶边，scroll = top_value - new_visible
        let new_scroll = (top_value - new_visible_clamped)
            .clamp(0.0, (total_value - new_visible_clamped).max(0.0));
        *value_zoom = new_z;
        *value_scroll = new_scroll;
        *dirty = true;
        ui.ctx().request_repaint();
    }
}
