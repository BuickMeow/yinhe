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
    yinhe_pianoroll::prepare(pianoroll, w, h, midi, view, selected, track_visible, *cursor_tick);
    view.dirty = false;

    // Render to offscreen texture and display in egui
    render_ctx.render_and_display(pianoroll, w, h, "pianoroll_frame", &painter, rect, texture_id);

    // Handle input (zoom/pan/cursor/drag/reset)
    crate::view_interaction::handle_input(ui, &resp, rect, view, cursor_tick, view.keyboard_width);
}
