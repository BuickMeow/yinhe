use eframe::egui;

/// Display the pianoroll texture with zoom/pan interaction.
pub fn show(
    ui: &mut egui::Ui,
    texture_id: egui::TextureId,
    available: egui::Vec2,
    pianoroll: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &std::collections::HashSet<(u16, u32)>,
) {
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = resp.rect;
    let w = rect.width() as u32;
    let h = rect.height() as u32;

    if w == 0 || h == 0 {
        return;
    }

    // Resize render target if needed
    render_ctx.ensure_size(w, h);

    // Clamp scroll
    let total_ticks = midi.map(|m| m.duration() * 200.0).unwrap_or(10000.0);
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Prepare and render to offscreen texture
    pianoroll.prepare(w, h, midi, view, selected);

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
    if resp.hovered() {
        let pointer_x = ui.input(|i| i.pointer.hover_pos().map(|p| p.x).unwrap_or(0.0));

        // Cmd+scroll: horizontal zoom
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let scroll = ui.input(|i| i.smooth_scroll_delta);

        if scroll != egui::Vec2::ZERO {
            if cmd {
                // Zoom
                let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                view.zoom_around_x(pointer_x - rect.min.x, factor);
            } else {
                // Pan: horizontal with scroll.x, vertical with scroll.y
                view.scroll_x -= scroll.x;
                view.scroll_y -= scroll.y;
            }
        }
    }

    // Drag to pan
    if resp.dragged() {
        let delta = resp.drag_delta();
        view.scroll_x -= delta.x;
        view.scroll_y -= delta.y;
    }

    // Double-click to reset view
    if resp.double_clicked() {
        *view = yinhe_pianoroll::PianoRollView::default();
    }
}
