//! 时间轴滚动条（横向 / 纵向瀑布流）。

use eframe::egui;
use yinhe_editor_core::density::NoteDensityView;

use super::density::draw_density_image;
use super::drag_warp::{WarpAxis, drag_delta_with_edge_warp, finish_edge_warp};
use super::{EDGE_WIDTH, colors};
use crate::theme;

/// Pixel-range allowed for `pixels_per_tick`.
const PPT_MIN: f32 = yinhe_types::PIXELS_PER_TICK_MIN;
const PPT_MAX: f32 = yinhe_types::PIXELS_PER_TICK_MAX;

/// 密度预览的绘制参数：计数数据 + 轨道配色（索引 = track id，`[r,g,b,a]`）。
pub(crate) struct DensityPreview<'a> {
    pub density: NoteDensityView<'a>,
    pub track_colors: &'a [[f32; 4]],
}

/// Paint a scrollbar into the given rect along the **主轴方向**（时间轴）。
///
/// The scrollbar represents the full timeline; a draggable rectangle
/// shows the current viewport.  Dragging the middle pans, dragging
/// either edge zooms (anchored on the opposite edge).
///
/// `orientation` 决定条形走向：横向 = 底部横条（X）；纵向瀑布流 = 右侧竖条（Y）。
/// `view_width` is the pixel-length of the content area along the main axis
/// (right of the keyboard / track-panel, or below the keyboard in vertical).
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view_width: f32,
    scroll_x: &mut f32,
    pixels_per_tick: &mut f32,
    total_ticks: f64,
    preview: Option<&DensityPreview<'_>>,
    play_tick: Option<f64>,
    dirty: &mut bool,
    orientation: yinhe_types::Orientation,
) -> f32 {
    let vertical = orientation == yinhe_types::Orientation::Vertical;
    // 沿主轴的 band 长度 / 厚度方向
    let along_len = |r: egui::Rect| if vertical { r.height() } else { r.width() };
    // 构造沿主轴的矩形（a0..a1 为相对 rect 起点的主轴区间）
    let along_rect = |a0: f32, a1: f32| {
        if vertical {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y + a0),
                egui::pos2(rect.max.x, rect.min.y + a1),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x + a0, rect.min.y),
                egui::pos2(rect.min.x + a1, rect.max.y),
            )
        }
    };
    // 主轴 / 副轴的 drag 分量
    let drag_main = |v: egui::Vec2| if vertical { v.y } else { v.x };
    let drag_cross = |v: egui::Vec2| if vertical { v.x } else { v.y };

    let sb_w = along_len(rect);
    if sb_w <= 0.0 || total_ticks <= 0.0 {
        return 0.0;
    }

    // 纵向瀑布流的时间轴屏幕坐标是镜像的（tick0 在底部）：把主轴内容坐标 a
    // 映射到屏幕上沿坐标。横向 = 恒等。
    let to_screen = |a: f32| if vertical { sb_w - a } else { a };
    // 内容区间 [a0,a1] → 屏幕矩形（纵向镜像后端点交换，故取 min/max）。
    let screen_rect = |a0: f32, a1: f32| {
        let (s0, s1) = (to_screen(a0), to_screen(a1));
        along_rect(s0.min(s1), s0.max(s1))
    };
    // 屏幕拖拽位移 → 主轴内容位移（纵向镜像取反）。
    let signed = |v: f32| if vertical { -v } else { v };

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

    // ── Density preview（背景之上、thumb 之下；不参与交互）──
    if let Some(p) = preview
        && p.density.max_count > 0
    {
        // 副轴像素行数 = 滚动条实际像素高度/宽度（加高自动更细），
        // 行 r 覆盖 key `[r*128/px_rows, (r+1)*128/px_rows)`。
        let px_rows = if vertical {
            rect.width().round().max(1.0) as usize
        } else {
            rect.height().round().max(1.0) as usize
        };
        draw_density_image(
            ui,
            rect,
            vertical,
            sb_w,
            px_rows,
            scale,
            &p.density,
            p.track_colors,
        );
    }

    // ── Rectangle visual ──
    let th_right = rect_right.min(sb_w);
    let rect_rect = screen_rect(rect_left, th_right);

    // Three interaction zones（镜像后 start=底部/左端均正确）
    let left_edge_rect = screen_rect(rect_left, (rect_left + EDGE_WIDTH).min(rect_right));
    let right_edge_rect = screen_rect((rect_right - EDGE_WIDTH).max(rect_left), rect_right);
    let middle_rect = screen_rect(
        (rect_left + EDGE_WIDTH).min(rect_right),
        (rect_right - EDGE_WIDTH).max(rect_left),
    );

    let edge_id_left = ui.scope_id().with("__sb_left__");
    let edge_id_right = ui.scope_id().with("__sb_right__");
    let middle_id = ui.scope_id().with("__sb_mid__");
    let warp_id = ui.scope_id().with("__sb_warp__");

    let left_resp = ui.interact(left_edge_rect, edge_id_left, egui::Sense::click_and_drag());
    let right_resp = ui.interact(
        right_edge_rect,
        edge_id_right,
        egui::Sense::click_and_drag(),
    );
    let middle_resp = ui.interact(middle_rect, middle_id, egui::Sense::click_and_drag());

    // 只有鼠标指针真的在滚动条 band 上时才允许交互，避免 egui 的
    // interact_radius（5px）导致 band 附近的指针误触发平移/缩放——
    // 例如拖动自动化锚点时鼠标靠近滚动条，绝不能和滚动条一起拖动。
    // hover 用当前指针位置；drag 用按下位置（press_origin）。
    let on_sb = ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| rect.contains(p));
    let press_on_sb = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| rect.contains(p));

    let left_hovered = on_sb && (left_resp.hovered() || left_resp.dragged());
    let right_hovered = on_sb && (right_resp.hovered() || right_resp.dragged());
    let middle_hovered = on_sb && (middle_resp.hovered() || middle_resp.dragged());

    // Paint rectangle with appropriate color。
    // thumb 半透明：视口铺满整条（fit 全曲）时也必须透出密度预览；
    // hover/drag 提升到 80% 保证交互可见性。边框（文本色系）让轮廓清晰。
    let thumb_color =
        if on_sb && (left_resp.dragged() || right_resp.dragged() || middle_resp.dragged()) {
            rect_drag_color.gamma_multiply(0.8)
        } else if middle_hovered || left_hovered || right_hovered {
            rect_hover_color.gamma_multiply(0.8)
        } else {
            rect_color.gamma_multiply(0.45)
        };
    ui.painter().rect_filled(rect_rect, 0.0, thumb_color);
    ui.painter().rect_stroke(
        rect_rect,
        0.0,
        egui::Stroke::new(1.0, theme::text_secondary()),
        egui::StrokeKind::Inside,
    );

    // ── Playhead on scrollbar（整曲进度，与 thumb 同 scale）──
    // `total_ticks` 已含 64 小节 padded（follow::total_ticks_padded），与 thumb 同一套映射，
    // 因此即使播放头在视口外，滚动条上仍可见全曲进度。细线盖在 thumb 之上。
    if let Some(ct) = play_tick
        && ct.is_finite()
        && ct >= 0.0
        && ct <= total_ticks
        && sb_w > 0.0
        && total_ticks > 0.0
    {
        let px = (ct * scale) as f32;
        if px >= 0.0 && px <= sb_w {
            let (a, b) = if vertical {
                (
                    egui::pos2(rect.min.x, rect.min.y + to_screen(px)),
                    egui::pos2(rect.max.x, rect.min.y + to_screen(px)),
                )
            } else {
                (
                    egui::pos2(rect.min.x + px, rect.min.y),
                    egui::pos2(rect.min.x + px, rect.max.y),
                )
            };
            ui.painter().line_segment(
                [a, b],
                egui::Stroke::new(theme::CURSOR_WIDTH, theme::accent_active()),
            );
        }
    }

    // ── Cursor ──
    // 拖动中隐藏系统指针：滚动条拖到窗口边缘时，指针不会在那里晃/触发系统 UI。
    let sb_dragging =
        press_on_sb && (left_resp.dragged() || right_resp.dragged() || middle_resp.dragged());
    if sb_dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    } else if left_hovered {
        ui.ctx().set_cursor_icon(if vertical {
            egui::CursorIcon::ResizeSouth
        } else {
            egui::CursorIcon::ResizeWest
        });
    } else if right_hovered {
        ui.ctx().set_cursor_icon(if vertical {
            egui::CursorIcon::ResizeNorth
        } else {
            egui::CursorIcon::ResizeEast
        });
    } else if middle_hovered || (on_sb && middle_resp.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction（仅当鼠标按下时真的在滚动条 band 上才有效）──
    // 缩放/平移一律从 thumb（或边缘）按下开始；背景区域（thumb 之外）
    // 拖拽不触发任何操作——确保"移动组件被按下才能开始缩放"。

    // Drag middle → pan（x 方向）；垂直位移 → 返回 dy 供调用处缩放对面轴。
    // 斜拖 = 平移 + 缩放同时进行。
    if press_on_sb && middle_resp.dragged() {
        let drag = drag_delta_with_edge_warp(
            ui,
            warp_id,
            &middle_resp,
            if vertical { WarpAxis::Y } else { WarpAxis::X },
        );
        let delta = signed(drag_main(drag));
        let delta_ticks = delta as f64 / scale;
        *scroll_x = (*scroll_x as f64 + delta_ticks * *pixels_per_tick as f64) as f32;
        *scroll_x = scroll_x.clamp(0.0, max_scroll_x(*pixels_per_tick));
        *dirty = true;
        ui.ctx().request_repaint();
        return drag_cross(drag);
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
    if press_on_sb && left_resp.dragged() {
        let new_left = (rect_left + signed(drag_main(left_resp.drag_delta())))
            .clamp(0.0, rect_right - 2.0 * EDGE_WIDTH);
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
    if press_on_sb && right_resp.dragged() {
        let new_right = (rect_right + signed(drag_main(right_resp.drag_delta())))
            .clamp(rect_left + 2.0 * EDGE_WIDTH, sb_w);
        let new_right_tick = new_right as f64 / scale;
        let new_viewport_ticks = (new_right_tick - start_tick).max(1.0);
        apply_zoom(scroll_x, pixels_per_tick, start_tick, new_viewport_ticks);
        ui.ctx().request_repaint();
    }

    // 背景区域 / 未按下：返回 0，不触发任何缩放/平移。
    // （缩放对面轴的 dy 已由 thumb 中间拖拽的垂直位移提供。）
    finish_edge_warp(ui, warp_id);
    0.0
}
