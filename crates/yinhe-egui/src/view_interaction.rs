use eframe::egui;

use yinhe_types::TimeSigEvent;

use crate::quantize::QuantizePreset;

/// Cursor-follow mode for auto-scrolling during playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FollowMode {
    /// Never auto-scroll — user has full manual control.
    None,
    /// Scroll when cursor reaches the viewport edge (current behavior).
    Page,
    /// Cursor stays glued to the leftmost edge of the content area.
    Continuous,
}

impl FollowMode {
    pub fn next(self) -> Self {
        match self {
            FollowMode::None => FollowMode::Page,
            FollowMode::Page => FollowMode::Continuous,
            FollowMode::Continuous => FollowMode::None,
        }
    }

    pub fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::*;
        match self {
            FollowMode::None => ICON_LOCK,
            FollowMode::Page => ICON_AUTO_STORIES,
            FollowMode::Continuous => ICON_CENTER_FOCUS_STRONG,
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            FollowMode::None => "不滚动",
            FollowMode::Page => "翻页跟随",
            FollowMode::Continuous => "实时跟随",
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

// ── Shared helpers ──

/// Total timeline length in ticks with 20% padding, or a sensible default
/// when the source has no notes.
pub(crate) fn total_ticks_padded(tick_length: u64) -> f64 {
    if tick_length > 0 {
        tick_length as f64 * 1.2
    } else {
        0.0
    }
}

/// Apply cursor-follow scrolling during playback.
///
/// Returns the new `scroll_x` if the view should scroll, or `None` if no
/// scroll is needed (mode is `None` or cursor is within the safe margin).
///
/// - `left_boundary`: left content edge in pixels (keyboard_width for piano, 0.0 for arrangement)
/// - `continuous_inset`: pixels to inset the cursor in Continuous mode (1.0 for piano, 0.01 for arrangement)
pub(crate) fn compute_follow_scroll(
    cursor_tick: f64,
    pixels_per_tick: f32,
    viewport_width: f32,
    left_boundary: f32,
    follow_mode: FollowMode,
    continuous_inset: f32,
) -> Option<f32> {
    match follow_mode {
        FollowMode::None => None,
        FollowMode::Page => {
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let content_width = viewport_width - left_boundary;
            let margin = content_width * 0.2;
            if cursor_x > viewport_width - margin || cursor_x < left_boundary + margin {
                Some((cursor_tick as f32 * pixels_per_tick) - content_width * 0.5)
            } else {
                None
            }
        }
        FollowMode::Continuous => {
            let target = cursor_tick as f32 * pixels_per_tick;
            Some(target - continuous_inset)
        }
    }
}

// ── Input handling ──

/// Handle zoom/pan/cursor input for a view that implements ViewInteraction.
///
/// When `existing_resp` is `Some`, it is used directly for click/drag/double-click
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
    let released = ui.input(|i| i.pointer.primary_released());
    let drag_dist = content_resp.drag_delta().length();
    if released
        && pointer_in_rect
        && drag_dist < 3.0
        && let Some(pos) = content_resp.interact_pointer_pos()
    {
        let pointer_x = pos.x - rect.min.x;
        if pointer_x >= left_zone_width {
            let tick = view.x_to_tick(pointer_x);
            let snapped = if let Some((q, ppq)) = &quantize {
                if let Some((tpb, num, den, events)) = &bar_line_data {
                    let (bar_start, next_bar) =
                        yinhe_wgpu::grid::measure_bounds_at_tick(tick, *tpb, *num, *den, events);
                    let offset = tick - bar_start;
                    let snapped_offset = q.snap_tick(offset, *ppq);
                    let grid_tick = bar_start + snapped_offset;
                    if (tick - next_bar).abs() < (tick - grid_tick).abs() {
                        next_bar
                    } else {
                        grid_tick
                    }
                } else {
                    q.snap_tick(tick, *ppq)
                }
            } else {
                tick
            };
            *cursor_tick = Some(snapped.max(0.0));
            *view.dirty() = true;
            ui.ctx().request_repaint();
        }
    }

    // Drag to pan
    if content_resp.dragged() {
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

    // Double-click to reset view
    if content_resp.double_clicked() {
        view.reset_to_default();
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_mode_returns_none() {
        assert_eq!(
            compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::None, 1.0),
            None
        );
    }

    #[test]
    fn continuous_mode_returns_cursor_minus_inset() {
        // cursor_tick=100, ppt=2.0 → cursor_x=200; inset=50 → 200-50=150
        assert_eq!(
            compute_follow_scroll(100.0, 2.0, 800.0, 0.0, FollowMode::Continuous, 50.0),
            Some(150.0)
        );
    }

    #[test]
    fn page_mode_scrolls_when_cursor_near_right_edge() {
        // cursor_tick=900, ppt=1.0 → cursor_x=900
        // content_width=800, margin=160
        // 900 > 800-160=640 → should scroll
        let result = compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0);
        assert!(result.is_some());
    }

    #[test]
    fn page_mode_scrolls_when_cursor_near_left_boundary() {
        // cursor_tick=20, ppt=1.0 → cursor_x=20
        // content_width=800, margin=160
        // 20 < 0+160=160 → should scroll
        let result = compute_follow_scroll(20.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0);
        assert!(result.is_some());
    }

    #[test]
    fn page_mode_stays_when_cursor_in_center() {
        // cursor_tick=400, ppt=1.0 → cursor_x=400
        // content_width=800, margin=160
        // 400 is between 160 and 640 → no scroll
        assert_eq!(
            compute_follow_scroll(400.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0),
            None
        );
    }

    #[test]
    fn page_mode_with_nonzero_left_boundary() {
        // left_boundary=100, viewport_width=800
        // content_width=700, margin=140
        // cursor_x=50, 50 < 100+140=240 → should scroll
        let result = compute_follow_scroll(50.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0);
        assert!(result.is_some());
    }
}
