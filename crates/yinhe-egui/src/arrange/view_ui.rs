use eframe::egui;

use yinhe_arrangement::instances as arrangement_instances;
use yinhe_arrangement::{ArrangementView, NoteSource, PianorollRenderer, Uniforms};
use yinhe_types::TimeSigEvent;

use std::collections::HashSet;

use crate::quantize::QuantizePreset;
use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

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
    selected: &mut HashSet<(u16, u32)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: &mut Option<f64>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    is_playing: bool,
    _track_names: &[String],
    follow_mode: &mut crate::view_interaction::FollowMode,
    active_tool: &Tool,
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
        active_tool,
    );

    // ── Selection drag (Select tool only) ──
    if *active_tool == Tool::Select && !is_playing {
        sel_drag_frame_arrange(ui, rect, view, midi, selected, quantize, ppq, bar_line_data);
    }
}

// ── Arrangement selection drag ──

fn sel_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    selected: &mut HashSet<(u16, u32)>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) {
    let sel_id = ui.id().with("sel_drag_arr");
    let mut drag: Option<(egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && content_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        drag = Some((local, local));
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        if !cmd {
            selected.clear();
        }
    }

    if let Some((start, _)) = drag {
        if pointer.primary_down() {
            if let Some(pos) = pointer.hover_pos() {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                drag = Some((start, local));
            }
        }

        if pointer.primary_released() {
            if let (Some(midi_ref), Some((start, end))) = (midi, drag) {
                let sx = start.x.min(end.x);
                let ex = start.x.max(end.x);
                let sy = start.y.min(end.y);
                let ey = start.y.max(end.y);

                let tick_s = view.x_to_tick(sx);
                let tick_e = view.x_to_tick(ex);

                // Y: pixel → track index
                let lh = view.lane_height;
                let scroll_y = view.base.scroll_y;
                let track_lo = ((scroll_y + sy) / lh).floor().max(0.0) as usize;
                let track_hi = ((scroll_y + ey) / lh).ceil().max(0.0) as usize;

                // Snap ticks
                let snapped_s = snap_tick(tick_s, quantize, ppq, bar_line_data);
                let snapped_e = snap_tick(tick_e, quantize, ppq, bar_line_data);
                let t_start = snapped_s.min(snapped_e);
                let t_end = snapped_s.max(snapped_e);

                let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                if !cmd {
                    selected.clear();
                }
                for track in track_lo..=track_hi {
                    // Arrangement: iterate all keys for this track
                    // Since notes don't have a track-indexed lookup, we
                    // iterate key_notes and filter by track.
                    // With max 128 keys this is fast (<10 µs per key scan).
                    for key in 0..128u8 {
                        for note in midi_ref.key_notes(key) {
                            if note.track as usize != track {
                                continue;
                            }
                            if note.start_tick as f64 <= t_end && note.end_tick as f64 >= t_start {
                                selected.insert((note.track, note.start_tick));
                            }
                        }
                    }
                }

                view.base.dirty = true;
            }
            drag = None;
        }
    }

    // Draw selection rect
    if let Some((start, end)) = drag {
        let sx = content_rect.min.x + start.x.min(end.x);
        let sy = content_rect.min.y + start.y.min(end.y);
        let ex = content_rect.min.x + start.x.max(end.x);
        let ey = content_rect.min.y + start.y.max(end.y);
        let sel_rect = egui::Rect::from_min_max(egui::pos2(sx, sy), egui::pos2(ex, ey));
        ui.painter().rect_filled(
            sel_rect,
            0.0,
            egui::Color32::from_rgba_premultiplied(100, 180, 255, 50),
        );
        ui.painter().rect_stroke(
            sel_rect,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 180, 255)),
            egui::StrokeKind::Middle,
        );
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
}

fn snap_tick(
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
