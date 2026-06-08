use eframe::egui;

use yinhe_arrangement::instances as arrangement_instances;
use yinhe_arrangement::{ArrangementView, NoteSource, PianorollRenderer, Uniforms};
use yinhe_types::TimeSigEvent;

use crate::quantize::QuantizePreset;
use crate::render_context::RenderContext;

/// Hash viewport properties that affect static arrangement instances.
fn viewport_hash(width: u32, height: u32, view: &ArrangementView) -> u64 {
    let mut h: u64 = 0;
    h ^= width as u64;
    h = h.wrapping_mul(31).wrapping_add(height as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.scroll_x.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.scroll_y.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.pixels_per_tick.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.lane_height.to_bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(view.base.left_panel_width.to_bits() as u64);
    h
}

/// Display the arrangement view texture with zoom/pan interaction.
///
/// Uses `PianorollRenderer::prepare_with_static_cache` so that the expensive
/// note-instance build only runs when the viewport actually changes (scroll,
/// zoom, resize).  During playback, only the cheap playhead-cursor update
/// runs every frame, leaving the audio thread enough CPU time.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut PianorollRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: &mut Option<f64>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    is_playing: bool,
    _track_names: &[String],
    follow_mode: &mut crate::view_interaction::FollowMode,
) {
    // Sense::click_and_drag() so that the response passed to handle_input
    // provides hover/drag/click/double-click state.  Unlike the piano roll,
    // the arrangement view's painter rect *is* the interaction rect (there
    // is no ruler/kb sub-division inside this child UI), so decoupling them
    // would be artificial.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = resp.rect;
    let w = rect.width() as u32;
    let h = rect.height() as u32;

    if w == 0 || h == 0 {
        return;
    }

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(w, h);

    // Clamp scroll
    let total_ticks = crate::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
    );
    let num_tracks = track_visible.len();
    view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);

    // Auto-follow: scroll based on follow mode (playback only).
    // Never auto-follow when paused, so the user can freely scroll around.
    if let Some(ct) = *cursor_tick
        && is_playing
        && *follow_mode != crate::view_interaction::FollowMode::None
    {
        if let Some(new_scroll_x) = crate::view_interaction::compute_follow_scroll(
            ct,
            view.base.pixels_per_tick,
            w as f32,
            0.0,
            *follow_mode,
            0.01,
        ) {
            view.base.scroll_x = new_scroll_x;
            view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);
        }
    }

    // ── Compute uniforms ──
    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: view.base.scroll_x,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: view.lane_height,
        keyboard_width: 0.0,
        _pad: 0.0,
    };

    // ── Prepare GPU data with static caching ──
    // The viewport hash captures all view properties that affect static
    // instances.  When the hash matches the cached value, the expensive
    // note-instance build is skipped entirely — only the cheap cursor
    // update runs every frame.
    let mut vhash = viewport_hash(w, h, view);
    if view.base.dirty {
        vhash = !vhash; // force rebuild for non-viewport changes (e.g. track visibility)
    }
    view.base.dirty = false;

    let gpu_updated = crate::widgets::qos::guarded(|| {
        renderer.prepare_with_static_cache(
            uniforms,
            vhash,
            |static_instances| {
                arrangement_instances::build_arrangement_static(
                    static_instances,
                    w,
                    h,
                    midi,
                    view,
                    track_visible,
                    track_colors,
                );
            },
            |cursor_instances| {
                arrangement_instances::build_arrangement_cursor(
                    cursor_instances,
                    *cursor_tick,
                    view,
                    w,
                    h,
                );
            },
        )
    });

    // Paint — skip GPU submit if nothing changed and no cursor to animate
    let content_changed = gpu_updated || is_playing;
    crate::widgets::qos::guarded(|| {
        render_ctx.paint(
            renderer,
            w,
            h,
            "arrangement_frame",
            &painter,
            rect,
            content_changed,
        );
    });

    // Handle input (zoom/pan/cursor/drag/reset).
    // Pass the painter response directly — the painter rect and interaction
    // rect are the same here, so there is no need for a dedicated interact.
    crate::view_interaction::handle_input(
        ui,
        rect,
        view,
        cursor_tick,
        0.0,
        Some((quantize, ppq)),
        bar_line_data,
        Some(&resp),
        is_playing,
        follow_mode,
    );
}
