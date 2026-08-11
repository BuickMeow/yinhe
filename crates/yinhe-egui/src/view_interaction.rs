use eframe::egui;

use yinhe_types::TimeSigEvent;

use yinhe_editor_core::quantize::QuantizePreset;

use crate::widgets::tools_panel::Tool;

pub use yinhe_editor_core::follow::{FollowMode, compute_follow_scroll, total_ticks_padded};

/// Extension trait adding egui-specific methods to [`FollowMode`].
pub(crate) trait FollowModeExt {
    fn icon(self) -> egui_material_icons::MaterialIcon;
}

impl FollowModeExt for FollowMode {
    fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::*;
        match self {
            FollowMode::None => ICON_LOCK,
            FollowMode::Page => ICON_AUTO_STORIES,
            FollowMode::Continuous => ICON_CENTER_FOCUS_STRONG,
        }
    }
}

/// Trait unifying the zoom/pan/cursor interface of PianoRollView and ArrangementView.
pub(crate) trait ViewInteraction {
    fn scroll_x(&mut self) -> &mut f32;
    fn scroll_y(&mut self) -> &mut f32;
    fn dirty(&mut self) -> &mut bool;
    fn x_to_tick(&self, x: f32) -> f64;
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32);
    fn zoom_around_y(&mut self, pointer_y: f32, factor: f32, height: f32);
}

impl ViewInteraction for yinhe_types::PianoRollView {
    fn scroll_x(&mut self) -> &mut f32 {
        &mut self.base.scroll_x
    }
    fn scroll_y(&mut self) -> &mut f32 {
        &mut self.base.scroll_y
    }
    fn dirty(&mut self) -> &mut bool {
        &mut self.base.dirty
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.x_to_tick(x)
    }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) {
        self.zoom_around_x(pointer_x, factor);
    }
    fn zoom_around_y(&mut self, pointer_y: f32, factor: f32, height: f32) {
        self.zoom_around_y(pointer_y, factor, height);
    }
}

impl ViewInteraction for yinhe_types::ArrangementView {
    fn scroll_x(&mut self) -> &mut f32 {
        &mut self.base.scroll_x
    }
    fn scroll_y(&mut self) -> &mut f32 {
        &mut self.base.scroll_y
    }
    fn dirty(&mut self) -> &mut bool {
        &mut self.base.dirty
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.x_to_tick(x)
    }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) {
        self.zoom_around_x(pointer_x, factor);
    }
    fn zoom_around_y(&mut self, pointer_y: f32, factor: f32, _height: f32) {
        self.zoom_lane_height(pointer_y, factor);
    }
}

// ── Input handling ──

/// Handle zoom/pan/cursor input for a view that implements ViewInteraction.
///
/// When `existing_resp` is `Some`, it is used directly for click/drag
/// detection instead of creating a new `click_and_drag` interact.  This avoids
/// egui interaction conflicts when the caller's `allocate_painter` already owns
/// the same rect (e.g. arrangement view inside a child UI).
///
/// When `existing_resp` is `None` (e.g. piano roll, where the interaction rect
/// differs from the painter rect), a dedicated `click_and_drag` interact is
/// created internally.
///
/// `left_zone_width`: pixels from the left edge where vertical zoom is
///   allowed (piano_view uses `keyboard_width`, arrangement uses `0.0`).
///   左区内：pinch/滚轮/cmd+scroll 均做垂直缩放；右区：pinch 水平缩放，
///   cmd+scroll.y 水平缩放，纯滚轮平移。
/// `rect` 必须是包含左区的完整 content rect（piano_view 传 content_rect，
///   arrangement 传 rect），因为 `x_to_tick` 内部会减 `left_panel_width`。
/// If `quantize` is provided, cursor placement snaps to the grid.
/// `bar_line_data: (ticks_per_beat, default_num, default_den, &[TimeSigEvent])`
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn handle_input(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view: &mut impl ViewInteraction,
    cursor_tick: &mut Option<f64>,
    left_zone_width: f32,
    quantize: Option<(QuantizePreset, u32)>,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    existing_resp: Option<&egui::Response>,
    // 命中区域（鼠标 containment / 自建 interact 的范围），None = rect。
    // AR 全宽纹理下 rect 含左列（轨道面板），交互必须限制在音乐区。
    hit_rect: Option<egui::Rect>,
    is_playing: bool,
    follow_mode: &mut FollowMode,
    active_tool: &Tool,
) {
    let hit_rect = hit_rect.unwrap_or(rect);
    // Use caller-supplied response when painter and interact rect are the
    // same; otherwise create a dedicated click_and_drag interact.
    let owned_resp;
    let content_resp: &egui::Response = if let Some(resp) = existing_resp {
        resp
    } else {
        owned_resp = ui.interact(
            hit_rect,
            ui.id().with("__content_drag__"),
            egui::Sense::click_and_drag(),
        );
        &owned_resp
    };

    // Hover, zoom, and scroll use a raw pointer-in-rect check instead of
    // content_resp.hovered().  The enclosing allocate_painter(Sense::hover())
    // blocks egui-level hover for child interacts, so we test containment
    // directly.  Drag/click/double-click go through content_resp and are
    // unaffected.
    let pointer_in_rect = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| hit_rect.contains(p)));

    if pointer_in_rect {
        let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
        let pointer_x = pointer_pos.x - rect.min.x;
        let pointer_y = pointer_pos.y - rect.min.y;

        // Trackpad pinch gesture → horizontal zoom, or vertical zoom when in left zone
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            if pointer_x < left_zone_width {
                view.zoom_around_y(pointer_y, zoom_delta, rect.height());
            } else {
                view.zoom_around_x(pointer_x, zoom_delta);
            }
            ui.ctx().request_repaint();
        }

        // Scroll handling:
        // - 左区（left_zone_width > 0，如钢琴键盘）：滚轮做垂直缩放（合并原 kb_zoom 语义）
        // - 右区 cmd+scroll.y: 水平缩放
        // - 右区纯滚轮: 平移
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let scroll = ui.input(|i| i.smooth_scroll_delta);

        if scroll != egui::Vec2::ZERO {
            let in_left_zone = pointer_x < left_zone_width;
            if in_left_zone {
                // 左区：滚轮垂直缩放（与原 kb_zoom 一致，cmd 不改变语义）
                // 方向：上滚 = 放大，下滚 = 缩小（与滚动条缩放一致）
                if scroll.y.abs() > 0.5 {
                    let factor = if scroll.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                    view.zoom_around_y(pointer_y, factor, rect.height());
                }
            } else if cmd {
                // 右区 cmd+scroll.y: 水平缩放（上滚 = 放大，下滚 = 缩小）
                if scroll.y.abs() > 0.5 {
                    let factor = if scroll.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                    view.zoom_around_x(pointer_x, factor);
                }
            } else {
                // 右区纯滚轮: 平移
                *view.scroll_x() -= scroll.x;
                *view.scroll_y() -= scroll.y;
                *view.dirty() = true;
                // Manual horizontal scroll during playback escapes follow mode.
                // Pure vertical scroll does not break follow.
                if is_playing && *follow_mode != FollowMode::None && scroll.x.abs() > 0.5 {
                    *follow_mode = FollowMode::None;
                }
            }
            ui.ctx().request_repaint();
        }
    }

    // Click to set cursor — pointer release + small drag distance.
    // Hover check here also uses raw rect containment for the same reason.
    // Skip in Select/SelectVertical/Pencil/Curve/Pan modes — those tools manage
    // their own clicks in drag.rs / pencil.rs.
    // 注意：Select 工具的 cursor 设置由 drag.rs 的 sel_drag_frame 处理，
    // 此处若也处理会导致坐标计算不一致（handle_input 收到 music_rect，
    // 而 x_to_tick 预期 widget-relative x，多减一次 kb_w 造成偏移）。
    let released = ui.input(|i| i.pointer.primary_released());
    let drag_dist = content_resp.drag_delta().length();
    if released
        && pointer_in_rect
        && drag_dist < 3.0
        && *active_tool != Tool::Select
        && *active_tool != Tool::SelectVertical
        && *active_tool != Tool::Pencil
        && *active_tool != Tool::Curve
        && *active_tool != Tool::Pan
        && let Some(pos) = content_resp.interact_pointer_pos()
    {
        let pointer_x = pos.x - rect.min.x;
        if pointer_x >= left_zone_width {
            let tick = view.x_to_tick(pointer_x);
            let snapped = if let Some((q, ppq)) = &quantize {
                let bar_ref = bar_line_data.as_ref().map(|(t, n, d, e)| (*t, *n, *d, *e));
                snap_tick(tick, *q, *ppq, bar_ref)
            } else {
                tick
            };
            *cursor_tick = Some(snapped.max(0.0));
            *view.dirty() = true;
            ui.ctx().request_repaint();
        }
    }

    // Middle-button drag → pan (always, regardless of tool)
    if pointer_in_rect && ui.input(|i| i.pointer.middle_down()) {
        let delta = ui.input(|i| i.pointer.delta());
        *view.scroll_x() -= delta.x;
        *view.scroll_y() -= delta.y;
        *view.dirty() = true;
        if is_playing && *follow_mode != FollowMode::None {
            *follow_mode = FollowMode::None;
        }
        ui.ctx().request_repaint();
    }

    // Left-button drag → pan (Pan tool only)
    if *active_tool == Tool::Pan && content_resp.dragged() {
        let delta = content_resp.drag_delta();
        *view.scroll_x() -= delta.x;
        *view.scroll_y() -= delta.y;
        *view.dirty() = true;
        // Manual drag during playback escapes follow mode.
        if is_playing && *follow_mode != FollowMode::None {
            *follow_mode = FollowMode::None;
        }
        ui.ctx().request_repaint();
    }
}

// ── Shared helpers ──

/// Returns true if the pointer is currently over a foreground layer (popup/menu).
/// When true, lower layers should not process pointer events to avoid click-through.
pub(crate) fn pointer_over_popup(ctx: &egui::Context) -> bool {
    if let Some(pos) = ctx.input(|i| i.pointer.hover_pos())
        && let Some(layer) = ctx.layer_id_at(pos)
    {
        return layer.order == egui::Order::Foreground;
    }
    false
}

/// Snap a tick value to the current quantize grid, with optional bar-line awareness.
pub fn snap_tick(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) =
            yinhe_types::measure_bounds_at_tick(tick, tpb, num, den, events);
        let offset = tick - bar_start;
        let snapped_offset = quantize.snap_tick(offset, ppq);
        let grid_tick = bar_start + snapped_offset;
        if (tick - next_bar).abs() < (tick - grid_tick).abs() {
            next_bar
        } else {
            grid_tick
        }
    } else {
        quantize.snap_tick(tick, ppq)
    }
}

/// Snap a tick value to the next quantization grid boundary (ceil),
/// with optional bar-line awareness.
pub fn snap_tick_ceil(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) =
            yinhe_types::measure_bounds_at_tick(tick, tpb, num, den, events);
        let offset = tick - bar_start;
        let snapped_offset = quantize.snap_tick_ceil(offset, ppq);
        let grid_tick = bar_start + snapped_offset;
        if (tick - next_bar).abs() < (tick - grid_tick).abs() {
            next_bar
        } else {
            grid_tick
        }
    } else {
        quantize.snap_tick_ceil(tick, ppq)
    }
}

/// Snap a tick value to the previous quantization grid boundary (floor),
/// with optional bar-line awareness.
pub fn snap_tick_floor(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) =
            yinhe_types::measure_bounds_at_tick(tick, tpb, num, den, events);
        let offset = tick - bar_start;
        let snapped_offset = quantize.snap_tick_floor(offset, ppq);
        let grid_tick = bar_start + snapped_offset;
        if (tick - next_bar).abs() < (tick - grid_tick).abs() {
            next_bar
        } else {
            grid_tick
        }
    } else {
        quantize.snap_tick_floor(tick, ppq)
    }
}

// ── Hover tooltip ──

/// 格式化带符号的数值：正用 `+`，负用 `-`。
pub(crate) fn format_signed(label: &str, v: i64) -> String {
    let sign = if v >= 0 { "+" } else { "-" };
    format!("{} {} {}", sign, v.abs(), label)
}

/// 在屏幕坐标 `(x, y)` 右上方绘制多行悬浮提示。
///
/// 复用于 automation panel / 选框工具 / 铅笔工具。
/// 各工具自行计算要显示的行内容，这里只负责绘制。
/// 提示框位于鼠标右上方（水平偏移 `gap`），避免框体盖住鼠标指针。
pub(crate) fn draw_hover_tooltip(ctx: &egui::Context, lines: &[String], x: f32, y: f32) {
    let painter = ctx.debug_painter();
    let font_id = egui::FontId::monospace(crate::theme::TOOLTIP_FONT);
    // 向右偏移 24px 再上移，拖拽时提示框不遮挡鼠标（曾因偏移过小挡住指针）。
    let gap = 24.0;
    let tooltip_x = x + gap;
    let tooltip_y = y - 24.0;
    let mut max_w = 0.0_f32;
    let line_h = 16.0;
    for line in lines {
        let galley =
            painter.layout_no_wrap(line.clone(), font_id.clone(), crate::theme::tooltip_text());
        max_w = max_w.max(galley.rect.width());
    }
    let total_h = line_h * lines.len() as f32;
    let pad = 6.0;
    let bg_rect = egui::Rect::from_min_size(
        egui::pos2(tooltip_x - pad, tooltip_y - pad),
        egui::vec2(max_w + pad * 2.0, total_h + pad * 2.0),
    );
    painter.rect_filled(bg_rect, 4.0, egui::Color32::from_black_alpha(180));
    let mut ly = tooltip_y;
    for line in lines {
        painter.text(
            egui::pos2(tooltip_x, ly),
            egui::Align2::LEFT_TOP,
            line,
            font_id.clone(),
            crate::theme::tooltip_text(),
        );
        ly += line_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_tick_no_bar_line() {
        let result = snap_tick(500.0, QuantizePreset::Fraction(1, 4), 480, None);
        assert_eq!(result, 480.0);
    }

    #[test]
    fn snap_tick_at_bar_start() {
        let result = snap_tick(0.0, QuantizePreset::Fraction(1, 4), 480, None);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn snap_tick_sixteenth_quantize() {
        let result = snap_tick(130.0, QuantizePreset::Fraction(1, 16), 480, None);
        assert_eq!(result, 120.0);
    }

    #[test]
    fn snap_tick_with_bar_line_data() {
        let events = [];
        let result = snap_tick(
            500.0,
            QuantizePreset::Fraction(1, 4),
            480,
            Some((480, 4, 2, &events)),
        );
        // In 4/4 at 480tpb, bar 1 spans 0..1920
        // tick 500 → offset from bar_start=0 is 500, snapped to 480
        assert_eq!(result, 480.0);
    }

    #[test]
    fn snap_tick_near_next_bar() {
        let events = [];
        // tick 1910 is very close to next bar at 1920
        let result = snap_tick(
            1910.0,
            QuantizePreset::Fraction(1, 4),
            480,
            Some((480, 4, 2, &events)),
        );
        // 1910 - 0 = 1910 offset, snapped to 1920 (4*480), but distance to next_bar(1920) = 10
        // distance to grid_tick(1920) = 0, so grid_tick wins
        assert_eq!(result, 1920.0);
    }

    #[test]
    fn snap_tick_inside_bar_with_3_4() {
        let events = [];
        // 3/4 at 480tpb → ticks_per_bar = 1440
        let result = snap_tick(
            500.0,
            QuantizePreset::Fraction(1, 4),
            480,
            Some((480, 3, 2, &events)),
        );
        // offset = 500, snapped to 480
        assert_eq!(result, 480.0);
    }

    #[test]
    fn snap_tick_zero_ppq() {
        // ppq=0 → interval=0 → returns tick unchanged
        let result = snap_tick(100.0, QuantizePreset::Fraction(1, 4), 0, None);
        assert_eq!(result, 100.0);
    }

    #[test]
    fn snap_tick_large_tick() {
        // 100000 / 480 = 208.33 → round to 208 → 208*480 = 99840
        let result = snap_tick(100000.0, QuantizePreset::Fraction(1, 4), 480, None);
        assert_eq!(result, 99840.0);
    }
}
