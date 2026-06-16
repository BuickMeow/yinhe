use eframe::egui;

use yinhe_arrangement::instances as arrangement_instances;
use yinhe_arrangement::{ArrangementView, NoteSource, PianorollRenderer, Uniforms};
use yinhe_types::TimeSigEvent;
use yinhe_wgpu::layer_cache_key;

use std::collections::HashSet;

use yinhe_editor_core::quantize::QuantizePreset;
use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

/// Display the arrangement view texture with zoom/pan interaction.
///
/// Uses the layered cache API: decor (layer 0), grid (layer 1), notes (layer 2).
/// The playhead cursor is drawn by egui on top of the wgpu texture.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut PianorollRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    selected: &mut HashSet<(u16, u32, u8)>,
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
    scroll_mode: u32,
    min_border_width: f32,
) {
    let _arrange_total_start = if crate::perf_probe::enabled() {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = resp.rect;
    let w = rect.width() as u32;
    let h = rect.height() as u32;

    if w == 0 || h == 0 {
        return;
    }

    render_ctx.ensure_size(w, h);

    let total_ticks = crate::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
    );
    let num_tracks = track_visible.len();
    view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);

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

    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = match scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        },
    };

    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: scroll_x_pos,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: view.lane_height,
        keyboard_width: view.base.left_panel_width,
        mode: 2, // AR notes: tick→pixel only
        scroll_frac,
        scroll_mode,
        min_border_width,
    };

    view.base.dirty = false;

    crate::util::qos::guarded(|| {
        renderer.upload_uniforms(uniforms);
        renderer.ensure_layers(3);

        // Layer 0: decor (background + track lanes)
        let tv_hash = {
            let mut h = 0u64;
            for &v in track_visible {
                h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
            }
            h
        };
        let decor_key = layer_cache_key(&[
            view.base.scroll_y.to_bits() as u64,
            view.lane_height.to_bits() as u64,
            h as u64,
            w as u64,
            view.base.left_panel_width.to_bits() as u64,
            tv_hash,
        ]);
        renderer.upload_layer(0, decor_key, |out| {
            arrangement_instances::build_decor(
                out,
                w as f32,
                h as f32,
                view.base.left_panel_width,
                view.lane_height,
                view.base.scroll_y,
                track_visible,
            );
        });

        // Layer 1: grid lines
        let mut grid_key = layer_cache_key(&[
            scroll_x_pos.to_bits() as u64,
            view.base.pixels_per_tick.to_bits() as u64,
            w as u64,
            h as u64,
            view.base.left_panel_width.to_bits() as u64,
        ]);
        if let Some(midi) = midi {
            let sig_events = midi.time_sig_events();
            let mut sig_hash = 0u64;
            for ev in sig_events {
                sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.tick as u64);
                sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.numerator as u64);
                sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.denominator as u64);
            }
            grid_key = layer_cache_key(&[grid_key, sig_hash]);
        }
        renderer.upload_layer(1, grid_key, |out| {
            if let Some(midi) = midi
                && let Some(tpb) = midi.ticks_per_beat()
            {
                let (def_num, def_den) = midi.time_sig_default();
                let sig_events = midi.time_sig_events();
                arrangement_instances::build_grid(
                    out,
                    w as f32,
                    h as f32,
                    view,
                    tpb,
                    def_num,
                    def_den,
                    sig_events,
                    scroll_x_pos,
                );
            }
        });

        // Layer 2: notes
        // Quantized scroll_x: during playback scroll_x changes smoothly, so the
        // cache stays valid for many frames.  tick_pad ensures cached notes
        // cover the full bucket range.  GPU clips off-screen notes.
        const SCROLL_BUCKET: f32 = 500.0;
        let scroll_bucket = (view.base.scroll_x / SCROLL_BUCKET) as i64 as u64;
        let tick_pad = (SCROLL_BUCKET / view.base.pixels_per_tick) as f64;
        let notes_key = layer_cache_key(&[
            scroll_bucket,
            view.base.scroll_y.to_bits() as u64,
            view.base.pixels_per_tick.to_bits() as u64,
            view.lane_height.to_bits() as u64,
            w as u64,
            h as u64,
            view.base.left_panel_width.to_bits() as u64,
            tv_hash,
        ]);
        renderer.upload_layer(2, notes_key, |out| {
            if let Some(midi) = midi {
                arrangement_instances::build_notes(
                    out,
                    w as f32,
                    h as f32,
                    midi,
                    view,
                    track_visible,
                    track_colors,
                    tick_pad,
                );
            }
        });
    });

    let content_changed = true;
    crate::util::qos::guarded(|| {
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

    // ── Playback cursor (drawn by egui on top of the wgpu texture) ──
    if let Some(ct) = *cursor_tick {
        let lb_w = view.base.left_panel_width;
        let cx_local = view.tick_to_x(ct);
        if cx_local >= lb_w && cx_local <= w as f32 {
            let cx = rect.min.x + cx_local;
            painter.line_segment(
                [
                    egui::pos2(cx, rect.min.y),
                    egui::pos2(cx, rect.max.y),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 204)),
            );
        }
    }

    // Selection drag runs BEFORE handle_input to avoid pointer-capture conflicts
    if *active_tool == Tool::Select && !is_playing {
        sel_drag_frame_arrange(
            ui,
            rect,
            view,
            midi,
            selected,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            num_tracks,
            cursor_tick,
        );
    }

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

    if let Some(t0) = _arrange_total_start {
        crate::perf_probe::record_arrange_total(t0.elapsed());
    }
}

// ── Arrangement selection drag ──

fn sel_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    selected: &mut HashSet<(u16, u32, u8)>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    num_tracks: usize,
    cursor_tick: &mut Option<f64>,
) {
    let sel_id = ui.id().with("sel_drag_arr");
    let mut drag: Option<(egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

    // Clear stale drag state (e.g. lost window focus mid-drag)
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }

    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && content_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        drag = Some((local, local));
        if !cmd {
            selected.clear();
        }
    }

    if let Some((start, _)) = drag {
        // Only update end on frames after the initial press
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(content_rect.min, content_rect.max);
                let local = egui::pos2(
                    clamped.x - content_rect.min.x,
                    clamped.y - content_rect.min.y,
                );
                drag = Some((start, local));

                // ── Auto-scroll when dragging near the edge ──
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
                    let old_x = view.base.scroll_x;
                    let old_y = view.base.scroll_y;
                    view.base.scroll_x += dx;
                    view.base.scroll_y += dy;
                    view.clamp_scroll(
                        content_rect.width(),
                        content_rect.height(),
                        total_ticks,
                        num_tracks,
                    );
                    let actual_dx = view.base.scroll_x - old_x;
                    let actual_dy = view.base.scroll_y - old_y;
                    if actual_dx != 0.0 || actual_dy != 0.0 {
                        view.base.dirty = true;
                        ui.ctx().request_repaint();
                        // Compensate start so it stays fixed in content space
                        drag = drag.map(|(s, e)| (egui::pos2(s.x - actual_dx, s.y - actual_dy), e));
                    }
                }
            }
        }

        if pointer.primary_released() {
            if let (Some(midi_ref), Some((start, end))) = (midi, drag) {
                let drag_dist = (end - start).length();

                if drag_dist < 3.0 {
                    // Click (no meaningful drag) — set cursor, clear selection
                    let tick = view.x_to_tick(start.x);
                    let snapped = snap_tick(tick, quantize, ppq, bar_line_data);
                    selected.clear();
                    *cursor_tick = Some(snapped.max(0.0));
                } else {
                    // Drag — existing marquee behavior
                    let (
                        _screen_sx,
                        _screen_ex,
                        _screen_sy,
                        _screen_ey,
                        t_start,
                        t_end,
                        track_lo,
                        track_hi,
                    ) = arrange_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);

                    if !cmd {
                        selected.clear();
                    }
                    for track in track_lo..=track_hi {
                        for key in 0..128u8 {
                            for note in midi_ref.key_notes(key) {
                                if note.track as usize != track {
                                    continue;
                                }
                                if (note.start_tick as f64) < t_end && (note.end_tick as f64) > t_start {
                                    selected.insert((note.track, note.start_tick, key));
                                }
                            }
                        }
                    }
                }
                view.base.dirty = true;
            }
            drag = None;
        }
    }

    // Draw snapped selection rect
    if let Some((start, end)) = drag {
        if (end - start).length() >= 3.0 {
            let (vx, vy, vw, vh, _, _, _, _) =
                arrange_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);
            let snapped = egui::Rect::from_min_max(
                egui::pos2(vx.min(vy), vw.min(vh)),
                egui::pos2(vx.max(vy), vw.max(vh)),
            );
            crate::widgets::selection_box::draw(&ui.painter(), content_rect, snapped);
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
}

/// Compute snapped selection bounds for arrangement.
/// Returns (view_local_sx, view_local_ex, view_local_sy, view_local_ey,
///          t_start, t_end, track_lo, track_hi).
fn arrange_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &ArrangementView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f32, f32, f32, f32, f64, f64, usize, usize) {
    let sx = start.x.min(end.x);
    let ex = start.x.max(end.x);
    let sy = start.y.min(end.y);
    let ey = start.y.max(end.y);

    let tick_s = view.x_to_tick(sx);
    let tick_e = view.x_to_tick(ex);
    let snapped_s = snap_tick(tick_s, quantize, ppq, bar_line_data);
    let snapped_e = snap_tick(tick_e, quantize, ppq, bar_line_data);
    let t_start = snapped_s.min(snapped_e);
    let mut t_end = snapped_s.max(snapped_e);

    // Ensure minimum width of one quantise grid interval
    let interval = quantize.tick_interval(ppq) as f64;
    if t_end <= t_start {
        t_end = t_start + interval.max(1.0);
    }

    let lh = view.lane_height;
    let scroll_y = view.base.scroll_y;
    let track_lo = ((scroll_y + sy) / lh).floor().max(0.0) as usize;
    let track_hi = ((scroll_y + ey) / lh).floor().max(0.0) as usize;

    let view_sy = track_lo as f32 * lh - scroll_y;
    let view_ey = (track_hi as f32 + 1.0) * lh - scroll_y;

    let view_sx = view.tick_to_x(t_start);
    let view_ex = view.tick_to_x(t_end);

    (
        view_sx, view_ex, view_sy, view_ey, t_start, t_end, track_lo, track_hi,
    )
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
