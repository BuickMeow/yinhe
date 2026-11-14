//! 像素空间 / 值空间垂直滚动条。

use eframe::egui;

use super::drag_warp::{WarpAxis, drag_delta_with_edge_warp, finish_edge_warp};
use super::{EDGE_WIDTH, colors};

// ── Vertical scrollbar (pixel-space) ──

/// 把连续 cell 尺寸吸附到离散档位（`Some(step)` 时）或仅做范围 clamp（`None`）。
/// 档位 = min + k·step，再 clamp 到 [min, max]。
pub(super) fn snap_cell_size(cs: f32, min: f32, max: f32, step: Option<f32>) -> f32 {
    match step {
        Some(step) => {
            let k = ((cs - min) / step).round();
            (min + k * step).clamp(min, max)
        }
        None => cs.clamp(min, max),
    }
}

/// 垂直滚动条（像素空间）：用于 AR（lane_height + scroll_y）和 PR（key_height + scroll_y）。
/// Paint a scrollbar for a discrete-cell axis（音高/轨道）沿其主轴方向。
///
/// 总范围 = `num_cells * cell_size`（如 `num_tracks * lane_height` 或 `128 * key_height`）。
/// 视口 = `view_height` 像素。`cell_size` = 每个单元的像素长度（lane_height / key_height）。
///
/// 三区交互（与现状纵向条对称）：
/// - 中间拖动 → 平移 scroll（主轴方向）
/// - 起点边拖动 → 缩放 cell_size，锚定 thumb 终点 sb 位置
/// - 终点边拖动 → 缩放 cell_size，锚定 thumb 起点 sb 位置
///
/// `orientation` 决定条形走向：横向视图 = 右侧竖条（主轴 Y）；纵向瀑布流 = 底部横条（主轴 X）。
/// `cell_min` / `cell_max` = cell_size 的最小/最大值。
/// `cell_step` = `Some(step)` 时 cell_size 只在 min + k·step 的离散档位上取值（AR 行高），
/// `None` 为连续缩放（PR 音高）。
/// `scroll` / `cell_size` 会被原地修改；`dirty` 标记视图为脏。
///
/// 即使 `total_pixels <= view_height`（内容一屏装下），也会绘制占满滚动条的 thumb，
/// 用户仍可拖动边缘缩放。只有 `max_scroll == 0` 时 pan 无效。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn show_vertical(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view_height: f32,
    scroll_y: &mut f32,
    cell_size: &mut f32,
    content_units: f32, // 内容总长度（单位 × cell_size = 总像素；折叠后行高不等，用浮点）

    cell_min: f32,
    cell_max: f32,
    cell_step: Option<f32>,
    dirty: &mut bool,
    orientation: yinhe_types::Orientation,
) -> f32 {
    // 相对「现状=主轴Y」的转置：纵向瀑布流时 key 条横着（主轴=X）。
    let transpose = orientation == yinhe_types::Orientation::Vertical;
    let along_len = |r: egui::Rect| if transpose { r.width() } else { r.height() };
    let along_rect = |a0: f32, a1: f32| {
        if transpose {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x + a0, rect.min.y),
                egui::pos2(rect.min.x + a1, rect.max.y),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y + a0),
                egui::pos2(rect.max.x, rect.min.y + a1),
            )
        }
    };
    let drag_main = |v: egui::Vec2| if transpose { v.x } else { v.y };
    let drag_cross = |v: egui::Vec2| if transpose { v.y } else { v.x };

    let sb_h = along_len(rect);
    if sb_h <= 0.0 || view_height <= 0.0 || content_units <= 0.0 {
        return 0.0;
    }

    let total_pixels = content_units * *cell_size;

    // max_scroll：当 total_pixels <= view_height 时为 0（无滚动空间，但仍然绘制 thumb）
    let max_scroll = (total_pixels - view_height).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Scale: scrollbar pixels per content pixel.
    let scale = sb_h / total_pixels.max(view_height);

    // ── Rectangle position and size (derived from current view state) ──
    let rect_top = (*scroll_y * scale).clamp(0.0, sb_h);
    let rect_height = (view_height * scale).min(sb_h - rect_top);
    let rect_bottom = rect_top + rect_height;

    let (bg_color, rect_color, rect_hover_color, rect_drag_color) = colors();
    // Paint background bar
    ui.painter().rect_filled(rect, 0.0, bg_color);

    // ── Rectangle visual ──
    let rect_rect = along_rect(rect_top, rect_bottom.min(sb_h));

    // Three interaction zones (start edge / middle / end edge)
    let start_edge_rect = along_rect(rect_top, (rect_top + EDGE_WIDTH).min(rect_bottom));
    let end_edge_rect = along_rect((rect_bottom - EDGE_WIDTH).max(rect_top), rect_bottom);
    let middle_rect = along_rect(
        (rect_top + EDGE_WIDTH).min(rect_bottom),
        (rect_bottom - EDGE_WIDTH).max(rect_top),
    );

    let edge_id_start = ui.scope_id().with("__vsb_start__");
    let edge_id_end = ui.scope_id().with("__vsb_end__");
    let middle_id = ui.scope_id().with("__vsb_mid__");
    let warp_id = ui.scope_id().with("__vsb_warp__");

    let start_resp = ui.interact(
        start_edge_rect,
        edge_id_start,
        egui::Sense::click_and_drag(),
    );
    let end_resp = ui.interact(end_edge_rect, edge_id_end, egui::Sense::click_and_drag());
    let middle_resp = ui.interact(middle_rect, middle_id, egui::Sense::click_and_drag());

    // 只有鼠标指针真的在滚动条 band 上时才允许交互（同水平滚动条），
    // 避免 interact_radius 导致 band 附近的指针（如拖动自动化锚点时）误触发
    // 平移/缩放，绝不允许和自动化一起拖动。
    let on_sb = ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| rect.contains(p));
    let press_on_sb = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| rect.contains(p));

    let start_hovered = on_sb && (start_resp.hovered() || start_resp.dragged());
    let end_hovered = on_sb && (end_resp.hovered() || end_resp.dragged());
    let middle_hovered = on_sb && (middle_resp.hovered() || middle_resp.dragged());

    // Paint rectangle with appropriate color
    let thumb_color =
        if on_sb && (start_resp.dragged() || end_resp.dragged() || middle_resp.dragged()) {
            rect_drag_color
        } else if middle_hovered || start_hovered || end_hovered {
            rect_hover_color
        } else {
            rect_color
        };
    ui.painter().rect_filled(rect_rect, 0.0, thumb_color);

    // ── Cursor ──
    // 拖动中隐藏系统指针（同水平滚动条）。
    let sb_dragging =
        press_on_sb && (start_resp.dragged() || end_resp.dragged() || middle_resp.dragged());
    if sb_dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    } else if start_hovered {
        ui.ctx().set_cursor_icon(if transpose {
            egui::CursorIcon::ResizeWest
        } else {
            egui::CursorIcon::ResizeNorth
        });
    } else if end_hovered {
        ui.ctx().set_cursor_icon(if transpose {
            egui::CursorIcon::ResizeEast
        } else {
            egui::CursorIcon::ResizeSouth
        });
    } else if middle_hovered || (on_sb && middle_resp.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction ──
    // 像素空间核心公式：thumb_len × cell_size = view_height × sb_h / num_cells = K
    let k_constant = view_height * sb_h / content_units;

    // 缩放/平移一律从 thumb（或边缘）按下开始；背景区域拖拽不触发任何操作。

    // Drag middle → pan（主轴方向）；副轴位移 → 返回供调用处缩放对面轴。
    if press_on_sb && middle_resp.dragged() {
        if max_scroll > 0.0 {
            let drag = drag_delta_with_edge_warp(
                ui,
                warp_id,
                &middle_resp,
                if transpose { WarpAxis::X } else { WarpAxis::Y },
            );
            let delta = drag_main(drag);
            *scroll_y = (*scroll_y + delta / scale).clamp(0.0, max_scroll);
            *dirty = true;
            ui.ctx().request_repaint();
            return drag_cross(drag);
        }
        return 0.0;
    }

    // Drag start edge → zoom，锚定 thumb 终点 sb 位置（rect_bottom 不动）
    if press_on_sb && start_resp.dragged() {
        let new_thumb_start_sb = (rect_top + drag_main(start_resp.drag_delta()))
            .clamp(0.0, rect_bottom - 2.0 * EDGE_WIDTH);
        let new_thumb_len_sb = (rect_bottom - new_thumb_start_sb).max(2.0 * EDGE_WIDTH);
        let new_cs = snap_cell_size(k_constant / new_thumb_len_sb, cell_min, cell_max, cell_step);
        let new_scale = sb_h / (content_units * new_cs);
        let new_scroll = rect_bottom / new_scale - view_height;
        let new_total_pixels = content_units * new_cs;
        let max_sy = (new_total_pixels - view_height).max(0.0);
        *cell_size = new_cs;
        *scroll_y = new_scroll.clamp(0.0, max_sy);
        *dirty = true;
        ui.ctx().request_repaint();
        return 0.0;
    }

    // Drag end edge → zoom，锚定 thumb 起点 sb 位置（rect_top 不动）
    if press_on_sb && end_resp.dragged() {
        let new_thumb_end_sb = (rect_bottom + drag_main(end_resp.drag_delta()))
            .clamp(rect_top + 2.0 * EDGE_WIDTH, sb_h);
        let new_thumb_len_sb = (new_thumb_end_sb - rect_top).max(2.0 * EDGE_WIDTH);
        let new_cs = snap_cell_size(k_constant / new_thumb_len_sb, cell_min, cell_max, cell_step);
        let new_scale = sb_h / (content_units * new_cs);
        let new_scroll = rect_top / new_scale;
        let new_total_pixels = content_units * new_cs;
        let max_sy = (new_total_pixels - view_height).max(0.0);
        *cell_size = new_cs;
        *scroll_y = new_scroll.clamp(0.0, max_sy);
        *dirty = true;
        ui.ctx().request_repaint();
    }

    // 背景区域 / 未按下：返回 0，不触发任何缩放/平移。
    finish_edge_warp(ui, warp_id);
    0.0
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

    let edge_id_top = ui.scope_id().with("__vsb_v_top__");
    let edge_id_bottom = ui.scope_id().with("__vsb_v_bottom__");
    let middle_id = ui.scope_id().with("__vsb_v_mid__");
    let warp_id = ui.scope_id().with("__vsb_v_warp__");

    let top_resp = ui.interact(top_edge_rect, edge_id_top, egui::Sense::click_and_drag());
    let bottom_resp = ui.interact(
        bottom_edge_rect,
        edge_id_bottom,
        egui::Sense::click_and_drag(),
    );
    let middle_resp = ui.interact(middle_rect, middle_id, egui::Sense::click_and_drag());

    // 只有鼠标指针在滚动条 band 上时才允许交互，避免 egui 的
    // interact_radius（5px）导致 band 附近的指针误触发平移/缩放。
    // hover 用当前指针位置；drag 用按下位置（press_origin），否则拖动锚点时
    // 鼠标划过 band 仍会误触发滚动条操作。
    // 参见 value_scrollbar_ignores_drag_outside_band 测试。
    let on_sb = ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| rect.contains(p));
    let press_on_sb = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| rect.contains(p));

    let top_hovered = on_sb && (top_resp.hovered() || top_resp.dragged());
    let bottom_hovered = on_sb && (bottom_resp.hovered() || bottom_resp.dragged());
    let middle_hovered = on_sb && (middle_resp.hovered() || middle_resp.dragged());

    let thumb_color =
        if on_sb && (top_resp.dragged() || bottom_resp.dragged() || middle_resp.dragged()) {
            rect_drag_color
        } else if middle_hovered || top_hovered || bottom_hovered {
            rect_hover_color
        } else {
            rect_color
        };
    ui.painter().rect_filled(rect_rect, 0.0, thumb_color);

    // 拖动中隐藏系统指针（同水平滚动条）。
    let sb_dragging =
        press_on_sb && (top_resp.dragged() || bottom_resp.dragged() || middle_resp.dragged());
    if sb_dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    } else if top_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNorth);
    } else if bottom_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouth);
    } else if middle_hovered || (on_sb && middle_resp.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction（仅当鼠标按下时在滚动条 band 上才有效）──

    // Drag middle → pan value_scroll（仅当 max_scroll > 0 时有效）
    if press_on_sb && middle_resp.dragged() && max_scroll > 0.0 {
        let delta = drag_delta_with_edge_warp(ui, warp_id, &middle_resp, WarpAxis::Y).y;
        // y 增加 = 向下滚 = value_scroll 减小
        *value_scroll = (*value_scroll - delta / scale).clamp(0.0, max_scroll);
        *dirty = true;
        ui.ctx().request_repaint();
        return;
    }

    // Drag top edge → zoom, anchoring at bottom edge (固定 bottom_value)
    if press_on_sb && top_resp.dragged() {
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
    if press_on_sb && bottom_resp.dragged() {
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
    finish_edge_warp(ui, warp_id);
}
