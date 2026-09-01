use eframe::egui;

use crate::theme;

// ── Constants ──

/// Height of the horizontal scrollbar band.
pub(crate) const SCROLLBAR_H: f32 = theme::SCROLLBAR_H;
/// Width of the vertical scrollbar band.
pub(crate) const SCROLLBAR_W: f32 = theme::SCROLLBAR_W;

const EDGE_WIDTH: f32 = 4.0;

/// 滚动条四色（运行时读取当前主题，不能是 const——getter 非 const fn）。
/// hover/drag 用统一悬浮/按下增益，不再单独定义绝对色。
fn colors() -> (egui::Color32, egui::Color32, egui::Color32, egui::Color32) {
    let rect = theme::line_fg();
    (
        theme::track_bg(),
        rect,
        theme::hover_color(rect),
        theme::pressed_color(rect),
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
        ctx.run_ui(raw, |ui| {
            out = show(
                ui,
                rect,
                300.0,
                scroll_x,
                ppt,
                1000.0,
                None,
                dirty,
                yinhe_types::Orientation::Horizontal,
            );
        })
        .textures_delta
        .clear();
        out
    }

    /// 跑一帧值空间滚动条（show_vertical_value），无返回值。
    fn run_frame_value(
        ctx: &egui::Context,
        raw: egui::RawInput,
        rect: egui::Rect,
        value_scroll: &mut f32,
        value_zoom: &mut f32,
        dirty: &mut bool,
    ) {
        ctx.run_ui(raw, |ui| {
            show_vertical_value(
                ui,
                rect,
                200.0,
                value_scroll,
                value_zoom,
                127.0,
                1.0,
                8.0,
                dirty,
            );
        })
        .textures_delta
        .clear();
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

    /// 背景拖拽（thumb 之外的 band 区域）→ 不返回 dy、不平移。
    /// 回归：拖动自动化锚点时鼠标靠近滚动条 band，绝不能和滚动条一起拖动、
    /// 绝不能在没按到移动组件（thumb）时触发缩放。
    /// 配置：total=1000 tick，view_width=300，ppt=1 → thumb 占 [0, 90]，
    /// x=200 是背景区。
    /// egui 的 hit test 基于上一帧注册的 widgets：先 hover 注册一帧再 press。
    #[test]
    fn background_drag_does_not_zoom() {
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
        // 帧3：drag（本帧垂直移动 20px）→ 背景拖拽不返回 dy
        let dy = run_frame(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        assert_eq!(dy, 0.0, "背景拖拽不应返回 dy（不得触发缩放），实际 {dy}");
        assert_eq!(scroll_x, 0.0, "背景拖拽不应平移");
    }

    /// 水平滚动条：鼠标在 band 外按下拖动（interact_radius 范围内）→ 不应平移/缩放。
    /// 回归：拖动自动化锚点时鼠标靠近水平滚动条 band，egui 的 interact_radius
    /// 会让 band 附近的指针命中滚动条 widget，导致误触发滚动条操作。
    #[test]
    fn horizontal_scrollbar_ignores_drag_outside_band() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
        let mut scroll_x = 0.0f32;
        let mut ppt = 1.0f32;
        let mut dirty = false;

        // 鼠标在 band 外（y=-3，距 band 上边缘 3px < interact_radius=5），x 在 thumb 中间
        let start = egui::pos2(45.0, -3.0);
        let end = egui::pos2(95.0, -3.0);
        // 帧1：hover 注册 widget
        let _ = run_frame(
            &ctx,
            drag_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧2：press（band 外）
        let _ = run_frame(
            &ctx,
            press_event(start),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        // 帧3：drag（水平移动 50px）→ 不应平移、不应缩放
        let dy = run_frame(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_x,
            &mut ppt,
            &mut dirty,
        );
        assert_eq!(
            scroll_x, 0.0,
            "band 外拖动不应平移 scroll_x，实际 {scroll_x}"
        );
        assert_eq!(ppt, 1.0, "band 外拖动不应缩放 ppt，实际 {ppt}");
        assert_eq!(dy, 0.0, "band 外拖动不应返回 dy，实际 {dy}");
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

    /// 鼠标在滚动条 band 外按下拖动（interact_radius 范围内）→ 不应触发平移/缩放。
    /// 回归：拖动自动化锚点时鼠标靠近滚动条 band，egui 的 interact_radius 会让
    /// band 附近的指针命中滚动条 widget，导致误触发滚动条操作。
    #[test]
    fn value_scrollbar_ignores_drag_outside_band() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 200.0));
        let mut value_scroll = 0.0f32;
        let mut value_zoom = 2.0f32; // visible_range=63.5，max_scroll>0，middle 拖动可平移
        let mut dirty = false;

        // 鼠标在 band 外（x=-3，距 band 左边缘 3px < interact_radius=5），y 在 thumb 中间
        let start = egui::pos2(-3.0, 100.0);
        let end = egui::pos2(-3.0, 130.0);
        // 帧1：hover 注册 widget
        run_frame_value(
            &ctx,
            drag_event(start),
            rect,
            &mut value_scroll,
            &mut value_zoom,
            &mut dirty,
        );
        // 帧2：press
        run_frame_value(
            &ctx,
            press_event(start),
            rect,
            &mut value_scroll,
            &mut value_zoom,
            &mut dirty,
        );
        // 帧3：drag（垂直移动 30px）→ 不应平移 value_scroll
        run_frame_value(
            &ctx,
            drag_event(end),
            rect,
            &mut value_scroll,
            &mut value_zoom,
            &mut dirty,
        );
        assert_eq!(
            value_scroll, 0.0,
            "band 外拖动不应平移 value_scroll，实际 {value_scroll}"
        );
        assert_eq!(
            value_zoom, 2.0,
            "band 外拖动不应缩放 value_zoom，实际 {value_zoom}"
        );
    }

    /// 鼠标在 band 外按下、拖动过程中划过 band → 仍不应触发滚动条操作。
    /// 回归：拖动自动化锚点时鼠标按下在 band 外，但拖动路径经过 band。
    #[test]
    fn value_scrollbar_ignores_drag_crossing_band() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 200.0));
        let mut value_scroll = 0.0f32;
        let mut value_zoom = 2.0f32;
        let mut dirty = false;

        // 按下在 band 外（x=-3），拖动到 band 内（x=12）
        let start = egui::pos2(-3.0, 100.0);
        let end = egui::pos2(12.0, 130.0);
        // 帧1：hover 注册 widget
        run_frame_value(
            &ctx,
            drag_event(start),
            rect,
            &mut value_scroll,
            &mut value_zoom,
            &mut dirty,
        );
        // 帧2：press（band 外）
        run_frame_value(
            &ctx,
            press_event(start),
            rect,
            &mut value_scroll,
            &mut value_zoom,
            &mut dirty,
        );
        // 帧3：drag（进入 band 内）→ 仍不应触发
        run_frame_value(
            &ctx,
            drag_event(end),
            rect,
            &mut value_scroll,
            &mut value_zoom,
            &mut dirty,
        );
        assert_eq!(
            value_scroll, 0.0,
            "band 外按下后划过 band 不应平移，实际 {value_scroll}"
        );
        assert_eq!(
            value_zoom, 2.0,
            "band 外按下后划过 band 不应缩放，实际 {value_zoom}"
        );
    }

    /// 跑一帧像素空间垂直滚动条（show_vertical），返回背景/边缘拖拽返回值。
    fn run_frame_vertical(
        ctx: &egui::Context,
        raw: egui::RawInput,
        rect: egui::Rect,
        scroll_y: &mut f32,
        cell_size: &mut f32,
        dirty: &mut bool,
    ) -> f32 {
        let mut out = 0.0f32;
        ctx.run_ui(raw, |ui| {
            out = show_vertical(
                ui,
                rect,
                100.0,
                scroll_y,
                cell_size,
                200,
                0.5,
                8.0,
                dirty,
                yinhe_types::Orientation::Horizontal,
            );
        })
        .textures_delta
        .clear();
        out
    }

    /// 背景拖拽（thumb 之外的 band 区域）→ 不返回 dx、不平移（与水平滚动条一致）。
    /// 配置：total=200×2=400px，view=100px，thumb 占 [0, 25]，y=60 是背景区。
    #[test]
    fn vertical_background_drag_does_not_zoom() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 100.0));
        let mut scroll_y = 0.0f32;
        let mut cell_size = 2.0f32;
        let mut dirty = false;

        let start = egui::pos2(8.0, 60.0);
        let end = egui::pos2(28.0, 60.0);
        // 帧1：hover 注册 widget
        let _ = run_frame_vertical(
            &ctx,
            drag_event(start),
            rect,
            &mut scroll_y,
            &mut cell_size,
            &mut dirty,
        );
        // 帧2：press
        let _ = run_frame_vertical(
            &ctx,
            press_event(start),
            rect,
            &mut scroll_y,
            &mut cell_size,
            &mut dirty,
        );
        // 帧3：drag（水平移动 20px）→ 背景拖拽不返回 dx
        let dx = run_frame_vertical(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_y,
            &mut cell_size,
            &mut dirty,
        );
        assert_eq!(dx, 0.0, "背景拖拽不应返回 dx（不得触发缩放），实际 {dx}");
        assert_eq!(scroll_y, 0.0, "背景拖拽不应平移 scroll_y");
        assert_eq!(cell_size, 2.0, "背景拖拽不应缩放 cell_size");
    }

    /// 垂直滚动条：鼠标在 band 外按下拖动（interact_radius 范围内）→ 不应平移/缩放。
    /// 回归：拖动自动化锚点时鼠标靠近垂直滚动条 band。
    #[test]
    fn vertical_scrollbar_ignores_drag_outside_band() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 100.0));
        let mut scroll_y = 0.0f32;
        let mut cell_size = 2.0f32;
        let mut dirty = false;

        // 鼠标在 band 外（x=-3，距 band 左边缘 3px < interact_radius=5），y 在 thumb 中间
        let start = egui::pos2(-3.0, 12.0);
        let end = egui::pos2(-3.0, 42.0);
        // 帧1：hover 注册 widget
        let _ = run_frame_vertical(
            &ctx,
            drag_event(start),
            rect,
            &mut scroll_y,
            &mut cell_size,
            &mut dirty,
        );
        // 帧2：press（band 外）
        let _ = run_frame_vertical(
            &ctx,
            press_event(start),
            rect,
            &mut scroll_y,
            &mut cell_size,
            &mut dirty,
        );
        // 帧3：drag（垂直移动 30px）→ 不应平移、不应缩放
        let dx = run_frame_vertical(
            &ctx,
            drag_event(end),
            rect,
            &mut scroll_y,
            &mut cell_size,
            &mut dirty,
        );
        assert_eq!(
            scroll_y, 0.0,
            "band 外拖动不应平移 scroll_y，实际 {scroll_y}"
        );
        assert_eq!(
            cell_size, 2.0,
            "band 外拖动不应缩放 cell_size，实际 {cell_size}"
        );
        assert_eq!(dx, 0.0, "band 外拖动不应返回 dx，实际 {dx}");
    }
}

// ── Horizontal scrollbar ──

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
    let rect_rect = along_rect(rect_left, rect_right.min(sb_w));

    // Three interaction zones
    let left_edge_rect = along_rect(rect_left, (rect_left + EDGE_WIDTH).min(rect_right));
    let right_edge_rect = along_rect((rect_right - EDGE_WIDTH).max(rect_left), rect_right);
    let middle_rect = along_rect(
        (rect_left + EDGE_WIDTH).min(rect_right),
        (rect_right - EDGE_WIDTH).max(rect_left),
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

    // Paint rectangle with appropriate color
    let thumb_color =
        if on_sb && (left_resp.dragged() || right_resp.dragged() || middle_resp.dragged()) {
            rect_drag_color
        } else if middle_hovered || left_hovered || right_hovered {
            rect_hover_color
        } else {
            rect_color
        };
    ui.painter().rect_filled(rect_rect, 0.0, thumb_color);

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
                    egui::pos2(rect.min.x, rect.min.y + px),
                    egui::pos2(rect.max.x, rect.min.y + px),
                )
            } else {
                (
                    egui::pos2(rect.min.x + px, rect.min.y),
                    egui::pos2(rect.min.x + px, rect.max.y),
                )
            };
            ui.painter().line_segment(
                [a, b],
                egui::Stroke::new(theme::CURSOR_WIDTH, theme::contrast_fg()),
            );
        }
    }

    // ── Cursor ──
    if left_hovered {
        ui.ctx().set_cursor_icon(if vertical {
            egui::CursorIcon::ResizeNorth
        } else {
            egui::CursorIcon::ResizeWest
        });
    } else if right_hovered {
        ui.ctx().set_cursor_icon(if vertical {
            egui::CursorIcon::ResizeSouth
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
        let delta = drag_main(middle_resp.drag_delta());
        let delta_ticks = delta as f64 / scale;
        *scroll_x = (*scroll_x as f64 + delta_ticks * *pixels_per_tick as f64) as f32;
        *scroll_x = scroll_x.clamp(0.0, max_scroll_x(*pixels_per_tick));
        *dirty = true;
        ui.ctx().request_repaint();
        return drag_cross(middle_resp.drag_delta());
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
        let new_left = (rect_left + drag_main(left_resp.drag_delta()))
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
        let new_right = (rect_right + drag_main(right_resp.drag_delta()))
            .clamp(rect_left + 2.0 * EDGE_WIDTH, sb_w);
        let new_right_tick = new_right as f64 / scale;
        let new_viewport_ticks = (new_right_tick - start_tick).max(1.0);
        apply_zoom(scroll_x, pixels_per_tick, start_tick, new_viewport_ticks);
        ui.ctx().request_repaint();
    }

    // 背景区域 / 未按下：返回 0，不触发任何缩放/平移。
    // （缩放对面轴的 dy 已由 thumb 中间拖拽的垂直位移提供。）
    0.0
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
    num_cells: usize,
    cell_min: f32,
    cell_max: f32,
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
    if sb_h <= 0.0 || view_height <= 0.0 || num_cells == 0 {
        return 0.0;
    }

    let num_cells_f = num_cells as f32;
    let total_pixels = num_cells_f * *cell_size;

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

    let edge_id_start = ui.id().with("__vsb_start__");
    let edge_id_end = ui.id().with("__vsb_end__");
    let middle_id = ui.id().with("__vsb_mid__");

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
    if start_hovered {
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
    let k_constant = view_height * sb_h / num_cells_f;

    // 缩放/平移一律从 thumb（或边缘）按下开始；背景区域拖拽不触发任何操作。

    // Drag middle → pan（主轴方向）；副轴位移 → 返回供调用处缩放对面轴。
    if press_on_sb && middle_resp.dragged() {
        if max_scroll > 0.0 {
            let delta = drag_main(middle_resp.drag_delta());
            *scroll_y = (*scroll_y + delta / scale).clamp(0.0, max_scroll);
            *dirty = true;
            ui.ctx().request_repaint();
        }
        return drag_cross(middle_resp.drag_delta());
    }

    // Drag start edge → zoom，锚定 thumb 终点 sb 位置（rect_bottom 不动）
    if press_on_sb && start_resp.dragged() {
        let new_thumb_start_sb = (rect_top + drag_main(start_resp.drag_delta()))
            .clamp(0.0, rect_bottom - 2.0 * EDGE_WIDTH);
        let new_thumb_len_sb = (rect_bottom - new_thumb_start_sb).max(2.0 * EDGE_WIDTH);
        let new_cs = (k_constant / new_thumb_len_sb).clamp(cell_min, cell_max);
        let new_scale = sb_h / (num_cells_f * new_cs);
        let new_scroll = rect_bottom / new_scale - view_height;
        let new_total_pixels = num_cells_f * new_cs;
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
        let new_cs = (k_constant / new_thumb_len_sb).clamp(cell_min, cell_max);
        let new_scale = sb_h / (num_cells_f * new_cs);
        let new_scroll = rect_top / new_scale;
        let new_total_pixels = num_cells_f * new_cs;
        let max_sy = (new_total_pixels - view_height).max(0.0);
        *cell_size = new_cs;
        *scroll_y = new_scroll.clamp(0.0, max_sy);
        *dirty = true;
        ui.ctx().request_repaint();
    }

    // 背景区域 / 未按下：返回 0，不触发任何缩放/平移。
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

    if top_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNorth);
    } else if bottom_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeSouth);
    } else if middle_hovered || (on_sb && middle_resp.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ── Interaction（仅当鼠标按下时在滚动条 band 上才有效）──

    // Drag middle → pan value_scroll（仅当 max_scroll > 0 时有效）
    if press_on_sb && middle_resp.dragged() && max_scroll > 0.0 {
        let delta = middle_resp.drag_delta().y;
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
}
