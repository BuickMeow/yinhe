use eframe::egui;

use yinhe_types::TimeSigEvent;

use yinhe_editor_core::quantize::QuantizePreset;
use crate::widgets::tools_panel::Tool;
use yinhe_types::view_base::TimelineViewBase;

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
    fn reset_to_default(&mut self);
}

impl ViewInteraction for yinhe_pianoroll::PianoRollView {
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
    fn reset_to_default(&mut self) {
        *self = Self::default();
    }
}

impl ViewInteraction for yinhe_arrangement::ArrangementView {
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
    fn reset_to_default(&mut self) {
        *self = Self::default();
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
/// If `quantize` is provided, cursor placement snaps to the grid.
/// `bar_line_data: (ticks_per_beat, default_num, default_den, &[TimeSigEvent])`
pub(crate) fn handle_input(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view: &mut impl ViewInteraction,
    cursor_tick: &mut Option<f64>,
    left_zone_width: f32,
    quantize: Option<(QuantizePreset, u32)>,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    existing_resp: Option<&egui::Response>,
    is_playing: bool,
    follow_mode: &mut FollowMode,
    active_tool: &Tool,
) {
    // Use caller-supplied response when painter and interact rect are the
    // same; otherwise create a dedicated click_and_drag interact.
    let owned_resp;
    let content_resp: &egui::Response = if let Some(resp) = existing_resp {
        resp
    } else {
        owned_resp = ui.interact(
            rect,
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
    let pointer_in_rect = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p)));

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

        // Cmd+scroll: horizontal zoom; plain scroll: pan
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let scroll = ui.input(|i| i.smooth_scroll_delta);

        if scroll != egui::Vec2::ZERO {
            if cmd {
                if scroll.y.abs() > 0.5 {
                    let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    view.zoom_around_x(pointer_x, factor);
                }
                if left_zone_width > 0.0 && scroll.x.abs() > 0.5 {
                    let factor = if scroll.x > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    view.zoom_around_y(pointer_y, factor, rect.height());
                }
            } else {
                *view.scroll_x() -= scroll.x;
                *view.scroll_y() -= scroll.y;
                *view.dirty() = true;
                // Manual scroll during playback escapes follow mode.
                if is_playing && *follow_mode != FollowMode::None {
                    *follow_mode = FollowMode::None;
                }
            }
            ui.ctx().request_repaint();
        }
    }

    // Click to set cursor — pointer release + small drag distance.
    // Hover check here also uses raw rect containment for the same reason.
    // Skip in Select mode — selection handler manages its own clicks.
    let released = ui.input(|i| i.pointer.primary_released());
    let drag_dist = content_resp.drag_delta().length();
    if released
        && pointer_in_rect
        && drag_dist < 3.0
        && *active_tool != Tool::Select
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
    if *active_tool != Tool::Select && content_resp.dragged() {
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

/// Snap a tick value to the current quantize grid, with optional bar-line awareness.
pub fn snap_tick(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> f64 {
    if let Some((tpb, num, den, events)) = bar_line_data {
        let (bar_start, next_bar) =
            yinhe_wgpu::grid::measure_bounds_at_tick(tick, tpb, num, den, events);
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

/// Auto-scroll the view when the pointer is near the edges of `content_rect`.
/// Returns the actual (dx, dy) scroll delta applied, so callers can compensate
/// drag anchors.
///
/// `clamp_fn` is called after modifying scroll to enforce bounds.
/// It receives `(content_width, content_height)` and should call
/// `view.base.clamp_scroll_x(...)` etc.
pub fn auto_scroll_on_drag(
    ui: &egui::Ui,
    base: &mut TimelineViewBase,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut TimelineViewBase, f32, f32),
) -> (f32, f32) {
    const MARGIN: f32 = 20.0;
    const BASE_SPEED: f32 = 15.0;
    let dt = ui.input(|i| i.unstable_dt);
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;

    if pos.x < content_rect.min.x + MARGIN {
        dx = -(content_rect.min.x + MARGIN - pos.x) * BASE_SPEED * dt;
    } else if pos.x > content_rect.max.x - MARGIN {
        dx = (pos.x - (content_rect.max.x - MARGIN)) * BASE_SPEED * dt;
    }

    if pos.y < content_rect.min.y + MARGIN {
        dy = -(content_rect.min.y + MARGIN - pos.y) * BASE_SPEED * dt;
    } else if pos.y > content_rect.max.y - MARGIN {
        dy = (pos.y - (content_rect.max.y - MARGIN)) * BASE_SPEED * dt;
    }

    if dx != 0.0 || dy != 0.0 {
        let old_x = base.scroll_x;
        let old_y = base.scroll_y;
        base.scroll_x += dx;
        base.scroll_y += dy;
        clamp_fn(base, content_rect.width(), content_rect.height());
        let actual_dx = base.scroll_x - old_x;
        let actual_dy = base.scroll_y - old_y;
        if actual_dx != 0.0 || actual_dy != 0.0 {
            base.dirty = true;
            ui.ctx().request_repaint();
            return (actual_dx, actual_dy);
        }
    }
    (0.0, 0.0)
}

/// Convert a persisted music selection `(t_start, t_end, key_lo, key_hi)` to
/// a pixel-space `Rect` in the pianoroll view.
pub fn music_sel_to_pixel_rect(
    base: &TimelineViewBase,
    key_height: f32,
    t_start: f64,
    t_end: f64,
    key_lo: u8,
    key_hi: u8,
) -> egui::Rect {
    let kh = key_height;
    let scroll_y = base.scroll_y;
    let sy = (127.0 - key_hi as f32) * kh - scroll_y;
    let ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;
    let sx = base.tick_to_x(t_start);
    let ex = base.tick_to_x(t_end);
    egui::Rect::from_min_max(
        egui::pos2(sx.min(ex), sy.min(ey)),
        egui::pos2(sx.max(ex), sy.max(ey)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_tick_no_bar_line() {
        let result = snap_tick(500.0, QuantizePreset::Quarter, 480, None);
        assert_eq!(result, 480.0);
    }

    #[test]
    fn snap_tick_at_bar_start() {
        let result = snap_tick(0.0, QuantizePreset::Quarter, 480, None);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn snap_tick_sixteenth_quantize() {
        let result = snap_tick(130.0, QuantizePreset::Sixteenth, 480, None);
        assert_eq!(result, 120.0);
    }

    #[test]
    fn snap_tick_with_bar_line_data() {
        let events = [];
        let result = snap_tick(500.0, QuantizePreset::Quarter, 480, Some((480, 4, 2, &events)));
        // In 4/4 at 480tpb, bar 1 spans 0..1920
        // tick 500 → offset from bar_start=0 is 500, snapped to 480
        assert_eq!(result, 480.0);
    }

    #[test]
    fn snap_tick_near_next_bar() {
        let events = [];
        // tick 1910 is very close to next bar at 1920
        let result = snap_tick(1910.0, QuantizePreset::Quarter, 480, Some((480, 4, 2, &events)));
        // 1910 - 0 = 1910 offset, snapped to 1920 (4*480), but distance to next_bar(1920) = 10
        // distance to grid_tick(1920) = 0, so grid_tick wins
        assert_eq!(result, 1920.0);
    }

    #[test]
    fn snap_tick_inside_bar_with_3_4() {
        let events = [];
        // 3/4 at 480tpb → ticks_per_bar = 1440
        let result = snap_tick(500.0, QuantizePreset::Quarter, 480, Some((480, 3, 2, &events)));
        // offset = 500, snapped to 480
        assert_eq!(result, 480.0);
    }

    #[test]
    fn snap_tick_zero_ppq() {
        // ppq=0 → interval=0 → returns tick unchanged
        let result = snap_tick(100.0, QuantizePreset::Quarter, 0, None);
        assert_eq!(result, 100.0);
    }

    #[test]
    fn snap_tick_large_tick() {
        // 100000 / 480 = 208.33 → round to 208 → 208*480 = 99840
        let result = snap_tick(100000.0, QuantizePreset::Quarter, 480, None);
        assert_eq!(result, 99840.0);
    }
}
