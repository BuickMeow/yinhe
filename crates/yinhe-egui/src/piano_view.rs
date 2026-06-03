use eframe::egui;

/// Display the pianoroll texture with zoom/pan interaction.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &std::collections::HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
) {
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = resp.rect;
    let w = rect.width() as u32;
    let h = rect.height() as u32;

    if w == 0 || h == 0 {
        return;
    }

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(w, h);
    let texture_id = render_ctx.preview_texture_id();

    // Clamp scroll — add some extra space beyond the last note
    let total_ticks = midi
        .and_then(|m| m.tick_length())
        .map(|tl| tl as f64 * 1.2)
        .unwrap_or(10000.0);
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Auto-follow: during playback, scroll so cursor stays visible
    if is_playing {
        if let Some(ct) = *cursor_tick {
            let cursor_x = view.tick_to_x(ct);
            let right_edge = w as f32;
            let margin = (right_edge - view.keyboard_width) * 0.2;
            if cursor_x > right_edge - margin || cursor_x < view.keyboard_width {
                view.scroll_x = (ct as f32 * view.pixels_per_tick)
                    - (right_edge - view.keyboard_width) * 0.5;
                view.clamp_scroll(w as f32, h as f32, total_ticks);
            }
        }
    }

    // Mark dirty during playback — cursor line needs to move each frame
    if is_playing && cursor_tick.is_some() {
        view.dirty = true;
    }

    // Prepare and render to offscreen texture
    pianoroll.prepare(w, h, midi, view, selected, track_visible, *cursor_tick);
    view.dirty = false;

    let mut encoder = render_ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pianoroll_frame"),
        });
    pianoroll.draw(&mut encoder, render_ctx.preview_view(), w, h);
    render_ctx.queue().submit(std::iter::once(encoder.finish()));

    // Display the texture in egui
    painter.image(
        texture_id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    // Handle input
    let mut changed = false;
    if resp.hovered() {
        let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
        let pointer_x = pointer_pos.x - rect.min.x;
        let pointer_y = pointer_pos.y - rect.min.y;

        // Trackpad pinch gesture → horizontal zoom, or vertical zoom when over keyboard
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            if pointer_x < view.keyboard_width {
                view.zoom_around_y(pointer_y, zoom_delta, rect.height());
            } else {
                view.zoom_around_x(pointer_x, zoom_delta);
            }
            changed = true;
        }

        // Cmd+scroll: scroll.y → horizontal zoom, scroll.x → vertical zoom
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let scroll = ui.input(|i| i.smooth_scroll_delta);

        if scroll != egui::Vec2::ZERO {
            if cmd {
                if scroll.y.abs() > 0.5 {
                    let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    view.zoom_around_x(pointer_x, factor);
                }
                if scroll.x.abs() > 0.5 {
                    let factor = if scroll.x > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    view.zoom_around_y(pointer_y, factor, rect.height());
                }
            } else {
                // Pan: horizontal with scroll.x, vertical with scroll.y
                view.scroll_x -= scroll.x;
                view.scroll_y -= scroll.y;
                view.dirty = true;
            }
            changed = true;
        }
    }

    // Set cursor on click — use pointer release + small drag distance instead of
    // resp.clicked() which fails on trackpads due to micro-movement.
    let released = ui.input(|i| i.pointer.primary_released());
    let drag_dist = resp.drag_delta().length();
    if released && resp.hovered() && drag_dist < 3.0 {
        if let Some(pos) = resp.interact_pointer_pos() {
            let pointer_x = pos.x - rect.min.x;
            if pointer_x >= view.keyboard_width {
                let tick = view.x_to_tick(pointer_x);
                *cursor_tick = Some(tick.max(0.0));
                view.dirty = true;
                changed = true;
            }
        }
    }

    // Drag to pan
    if resp.dragged() {
        let delta = resp.drag_delta();
        view.scroll_x -= delta.x;
        view.scroll_y -= delta.y;
        view.dirty = true;
        changed = true;
    }

    // Double-click to reset view
    if resp.double_clicked() {
        *view = yinhe_pianoroll::PianoRollView::default();
        changed = true;
    }

    if changed {
        ui.ctx().request_repaint();
    }
}
