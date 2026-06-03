use eframe::egui;

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
    fn scroll_x(&mut self) -> &mut f32 { &mut self.scroll_x }
    fn scroll_y(&mut self) -> &mut f32 { &mut self.scroll_y }
    fn dirty(&mut self) -> &mut bool { &mut self.dirty }
    fn x_to_tick(&self, x: f32) -> f64 { self.x_to_tick(x) }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) { self.zoom_around_x(pointer_x, factor); }
    fn zoom_around_y(&mut self, pointer_y: f32, factor: f32, height: f32) { self.zoom_around_y(pointer_y, factor, height); }
    fn reset_to_default(&mut self) { *self = Self::default(); }
}

impl ViewInteraction for yinhe_pianoroll::ArrangementView {
    fn scroll_x(&mut self) -> &mut f32 { &mut self.scroll_x }
    fn scroll_y(&mut self) -> &mut f32 { &mut self.scroll_y }
    fn dirty(&mut self) -> &mut bool { &mut self.dirty }
    fn x_to_tick(&self, x: f32) -> f64 { self.x_to_tick(x) }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) { self.zoom_around_x(pointer_x, factor); }
    fn zoom_around_y(&mut self, _pointer_y: f32, _factor: f32, _height: f32) { /* no-op for arrangement */ }
    fn reset_to_default(&mut self) { *self = Self::default(); }
}

/// Handle zoom/pan/cursor input for a view that implements ViewInteraction.
///
/// `left_zone_width`: pixels from the left edge where vertical zoom is allowed
///   (piano_view uses `keyboard_width`, arrangement uses `0.0` to disable).
/// Returns `true` if the view state changed and needs a repaint.
pub(crate) fn handle_input(
    ui: &mut egui::Ui,
    resp: &egui::Response,
    rect: egui::Rect,
    view: &mut impl ViewInteraction,
    cursor_tick: &mut Option<f64>,
    left_zone_width: f32,
) -> bool {
    let mut changed = false;

    if resp.hovered() {
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
            changed = true;
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
            }
            changed = true;
        }
    }

    // Click to set cursor — pointer release + small drag distance instead of
    // resp.clicked() which fails on trackpads due to micro-movement.
    let released = ui.input(|i| i.pointer.primary_released());
    let drag_dist = resp.drag_delta().length();
    if released && resp.hovered() && drag_dist < 3.0 {
        if let Some(pos) = resp.interact_pointer_pos() {
            let pointer_x = pos.x - rect.min.x;
            if pointer_x >= left_zone_width {
                let tick = view.x_to_tick(pointer_x);
                *cursor_tick = Some(tick.max(0.0));
                *view.dirty() = true;
                changed = true;
            }
        }
    }

    // Drag to pan
    if resp.dragged() {
        let delta = resp.drag_delta();
        *view.scroll_x() -= delta.x;
        *view.scroll_y() -= delta.y;
        *view.dirty() = true;
        changed = true;
    }

    // Double-click to reset view
    if resp.double_clicked() {
        view.reset_to_default();
        changed = true;
    }

    if changed {
        ui.ctx().request_repaint();
    }

    changed
}
