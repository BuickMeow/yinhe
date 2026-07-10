use eframe::egui;

use yinhe_arrangement::{build_decor, build_grid, build_notes};
use yinhe_types::{ArrangementView, NoteSource};
use yinhe_wgpu::{InstanceRenderer, Uniforms, TrackColorsUniform, MAX_TRACKS};
use yinhe_types::TimeSigEvent;
use yinhe_wgpu::layer_cache_key;

use yinhe_editor_core::quantize::QuantizePreset;
use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

/// Display the arrangement view texture with zoom/pan interaction.
///
/// Uses the layered cache API: decor (layer 0), grid (layer 1), notes (layer 2).
/// The playhead cursor is drawn by egui on top of the wgpu texture.
///
/// `arr_drag_delta` is set on mouse release after dragging an existing selection
/// (moving notes + automation events in the selected track/time range).
/// `(delta_ticks, delta_tracks)` — ticks are horizontal, tracks are vertical.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut InstanceRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    selected: &mut yinhe_core::Selection,
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
    haptic_engine: Option<&yinhe_haptic::HapticEngine>,
    midi_version: u64,
    arr_sel_rect: &mut Option<(f64, f64, usize, usize)>,
    arr_drag_delta: &mut Option<(i64, i32)>,
) {
    let _arrange_total_start = if yinhe_memtrace::perf_probe::enabled() {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = resp.rect;
    let ppp = ui.ctx().pixels_per_point();
    let w = rect.width() as u32;
    let h = rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;

    if w == 0 || h == 0 {
        return;
    }

    render_ctx.ensure_size(pw, ph);

    let total_ticks = crate::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
        ppq,
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

    // Build track colors uniform — allocate on heap to avoid 1MB stack overflow
    let track_count = track_colors.len().min(MAX_TRACKS) as u32;
    let mut tc_buf: Vec<u8> = vec![0u8; std::mem::size_of::<TrackColorsUniform>()];
    let tc_uniform: &mut TrackColorsUniform = bytemuck::from_bytes_mut(&mut tc_buf);
    for (i, color) in track_colors.iter().enumerate().take(MAX_TRACKS) {
        tc_uniform.colors[i] = [color[0], color[1], color[2], 1.0];
    }

    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: scroll_x_pos,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0, // AR unused (shader uses lane_height)
        keyboard_width: view.base.left_panel_width,
        mode: 2, // AR notes: shader computes pixel_y from lane_height + scroll_y
        scroll_frac,
        scroll_mode,
        min_border_width,
        track_count, // AR notes now use track_colors uniform for coloring
        sel_rect_count: 0, // unused in AR mode
        note_selection_highlight: 0, // AR mode: no note selection highlight
        lane_height: view.lane_height, // AR: per-track lane height
        note_alpha: 0.85, // AR notes semi-transparent
    };

    view.base.dirty = false;

    let theme = renderer.theme().clone();
    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(tc_uniform);
    renderer.ensure_layers(3);

    let vh = view.render_hash();
    let wh = {
        let mut hash: u64 = 0;
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(w as u64);
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(h as u64);
        hash
    };

    // Layer 0: decor (background + track lanes)
    let tv_hash = {
        let mut h = 0u64;
        for &v in track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };
    let decor_key = layer_cache_key(&[vh, wh, tv_hash]);
    renderer.upload_layer(0, decor_key, |out| {
        build_decor(
            out,
            w as f32,
            h as f32,
            view.base.left_panel_width,
            view.lane_height,
            view.base.scroll_y,
            track_visible,
            &theme,
        );
    });

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[vh, wh]);
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
            build_grid(
                out, w as f32, h as f32, view, tpb, def_num, def_den, sig_events, scroll_x_pos, &theme,
            );
        }
    });

    // Layer 2: notes (16B NoteInstance — shader computes pixel positions from uniforms)
    let notes_key = layer_cache_key(&[vh, wh, tv_hash, midi_version]);
    renderer.upload_note_layer(2, notes_key, |out| {
        if let Some(midi) = midi {
            build_notes(
                out,
                w as f32,
                h as f32,
                midi,
                view,
                track_visible,
            );
        }
    });

    let content_changed = true;
    render_ctx.paint(
        renderer,
        pw,
        ph,
        "arrangement_frame",
        &painter,
        rect,
        content_changed,
    );

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
                egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::CURSOR_COLOR),
            );
        }
    }

    // Selection drag runs BEFORE handle_input to avoid pointer-capture conflicts
    if *active_tool == Tool::Select {
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
            arr_sel_rect,
            arr_drag_delta,
        );
    }

    // Draw persisted selection rect (remains after mouse release).
    if let Some((t_start, t_end, track_lo, track_hi)) = *arr_sel_rect {
        let lh = view.lane_height;
        let scroll_y = view.base.scroll_y;
        let view_sy = track_lo as f32 * lh - scroll_y;
        let view_ey = (track_hi as f32 + 1.0) * lh - scroll_y;
        let view_sx = view.tick_to_x(t_start);
        let view_ex = view.tick_to_x(t_end);
        let snapped = egui::Rect::from_min_max(
            egui::pos2(view_sx.min(view_ex), view_sy.min(view_ey)),
            egui::pos2(view_sx.max(view_ex), view_sy.max(view_ey)),
        );
        crate::selection::draw::draw(&ui.painter(), rect, snapped);
    }

    // Save scroll state before input for haptic boundary detection
    let pre_scroll_x = view.base.scroll_x;
    let pre_scroll_y = view.base.scroll_y;
    let raw_scroll = ui.input(|i| i.smooth_scroll_delta);
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

    // Clamp scroll after input and check for haptic boundary
    view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);
    let max_sx = (total_ticks as f32 * view.base.pixels_per_tick - (w as f32 - view.base.left_panel_width)).max(0.0);
    let max_sy = (num_tracks as f32 * view.lane_height - h as f32).max(0.0);
    crate::view_interaction::notify_haptic_boundary(
        yinhe_haptic::HapticSlot::Arrangement,
        pre_scroll_x,
        pre_scroll_y,
        view.base.scroll_x,
        view.base.scroll_y,
        max_sx,
        max_sy,
        raw_scroll,
        haptic_engine,
    );
    crate::view_interaction::notify_haptic_zoom(
        yinhe_haptic::HapticSlot::Arrangement,
        view.base.pixels_per_tick,
        view.lane_height,
        0.001,
        10.0,
        16.0,
        120.0,
        haptic_engine,
    );

    if let Some(t0) = _arrange_total_start {
        yinhe_memtrace::perf_probe::record_arrange_total(t0.elapsed());
    }
}

// ── Arrangement selection drag ──

fn sel_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    selected: &mut yinhe_core::Selection,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    num_tracks: usize,
    cursor_tick: &mut Option<f64>,
    arr_sel_rect: &mut Option<(f64, f64, usize, usize)>,
    arr_drag_delta: &mut Option<(i64, i32)>,
) {
    let sel_id = ui.id().with("sel_drag_arr");
    // 拖框起始点存音乐坐标 (start_tick, start_track_y)，免疫任何滚动
    // （触摸板滚动、自动滚动、中键拖拽都不会改变音乐坐标）
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    // Move-drag state: ((origin_tick, origin_track_f), (current_tick, current_track_f))
    // Stores both tick (horizontal) and track-float (vertical) music coordinates.
    let move_drag_id = ui.id().with("arr_move_drag");
    let mut move_drag: Option<((f64, f32), (f64, f32))> =
        ui.data_mut(|d| d.get_persisted(move_drag_id)).unwrap_or(None);
    // Saved original arr_sel_rect at drag start (so we can hide the original during drag)
    let move_orig_id = ui.id().with("arr_move_orig_sel");
    let mut move_orig_sel: Option<(f64, f64, usize, usize)> =
        ui.data_mut(|d| d.get_persisted(move_orig_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

    // Clear stale drag states (e.g. lost window focus mid-drag)
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }
    if move_drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        move_drag = None;
        move_orig_sel = None;
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(sel_id, drag));
        ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
        ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
        return;
    }

    // ── Check if mouse is inside the existing selection rect (for Move cursor + drag) ──
    let inside_sel_rect = if let Some((t_start, t_end, track_lo, track_hi)) = *arr_sel_rect {
        pointer.hover_pos().is_some_and(|pos| {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let lh = view.lane_height;
            let scroll_y = view.base.scroll_y;
            let sy = track_lo as f32 * lh - scroll_y;
            let ey = (track_hi as f32 + 1.0) * lh - scroll_y;
            let sx = view.tick_to_x(t_start);
            let ex = view.tick_to_x(t_end);
            let rect = egui::Rect::from_min_max(
                egui::pos2(sx.min(ex), sy.min(ey)),
                egui::pos2(sx.max(ex), sy.max(ey)),
            );
            rect.contains(local)
        })
    } else {
        false
    };

    // Show Move cursor when hovering over the selection rect (only when not currently dragging)
    if inside_sel_rect && move_drag.is_none() && drag.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }

    // ── Primary press handling ──
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && content_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let click_tick = view.x_to_tick(local.x);
        let click_track_f = (local.y + view.base.scroll_y) / view.lane_height;

        if inside_sel_rect && !cmd {
            // Start move-drag of existing selection
            // Save original rect and clear it (so show() doesn't draw the original during drag)
            move_orig_sel = *arr_sel_rect;
            *arr_sel_rect = None;
            let origin = (click_tick, click_track_f);
            move_drag = Some((origin, origin));
            drag = None;
        } else {
            // Start new selection marquee
            let start_track_y = (local.y + view.base.scroll_y) / view.lane_height;
            drag = Some(((click_tick, start_track_y), local));
            *arr_sel_rect = None;
            if !cmd {
                selected.clear();
            }
        }
    }

    // ── Move-drag handling ──
    if let Some((origin, _)) = move_drag {
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                let current_tick = view.x_to_tick(local.x);
                let current_track_f = (local.y + view.base.scroll_y) / view.lane_height;
                move_drag = Some((origin, (current_tick, current_track_f)));

                // Auto-scroll when dragging near the edge
                let lane_height = view.lane_height;
                crate::selection::drag::auto_scroll_on_drag(
                    ui,
                    &mut view.base,
                    content_rect,
                    pos,
                    |base, w, h| {
                        base.clamp_scroll_x(w, total_ticks);
                        let max_scroll_y = (num_tracks as f32 * lane_height - h).max(0.0);
                        base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
                    },
                );
            }
        }

        if pointer.primary_released() {
            if let Some(((origin_t, origin_tr), (current_t, current_tr))) = move_drag {
                // Snap ticks to grid for quantized horizontal delta
                let snapped_origin = crate::view_interaction::snap_tick(origin_t, quantize, ppq, bar_line_data);
                let snapped_current = crate::view_interaction::snap_tick(current_t, quantize, ppq, bar_line_data);
                let delta_ticks = (snapped_current - snapped_origin).round() as i64;
                // Vertical: round to nearest integer track
                let delta_tracks = (current_tr - origin_tr).round() as i32;

                let has_moved = delta_ticks != 0 || delta_tracks != 0;

                if has_moved {
                    *arr_drag_delta = Some((delta_ticks, delta_tracks));

                    // Restore arr_sel_rect from saved original, offset by delta
                    if let Some((t_start, t_end, track_lo, track_hi)) = move_orig_sel {
                        let new_lo = (track_lo as i32 + delta_tracks).max(0) as usize;
                        let new_hi = (track_hi as i32 + delta_tracks).max(0) as usize;
                        *arr_sel_rect = Some((
                            t_start + delta_ticks as f64,
                            t_end + delta_ticks as f64,
                            new_lo,
                            new_hi,
                        ));
                    }
                    view.base.dirty = true;
                } else {
                    // No move: restore original rect
                    *arr_sel_rect = move_orig_sel;
                }
            }
            move_drag = None;
            move_orig_sel = None;
        }

        // Draw offset selection rect during drag (using saved original)
        if let (Some((origin, current)), Some((t_start, t_end, track_lo, track_hi))) =
            (move_drag, move_orig_sel)
        {
            // Snap for display too
            let (origin_t, origin_tr) = origin;
            let (current_t, current_tr) = current;
            let snapped_origin = crate::view_interaction::snap_tick(origin_t, quantize, ppq, bar_line_data);
            let snapped_current = crate::view_interaction::snap_tick(current_t, quantize, ppq, bar_line_data);
            let display_dt = snapped_current - snapped_origin;
            let display_dtr = (current_tr - origin_tr).round() as i32;
            let lh = view.lane_height;
            let scroll_y = view.base.scroll_y;
            let new_track_lo = (track_lo as i32 + display_dtr).max(0) as usize;
            let new_track_hi = (track_hi as i32 + display_dtr).max(0) as usize;
            let sy = new_track_lo as f32 * lh - scroll_y;
            let ey = (new_track_hi as f32 + 1.0) * lh - scroll_y;
            let sx = view.tick_to_x(t_start + display_dt);
            let ex = view.tick_to_x(t_end + display_dt);
            let snapped = egui::Rect::from_min_max(
                egui::pos2(sx.min(ex), sy.min(ey)),
                egui::pos2(sx.max(ex), sy.max(ey)),
            );
            crate::selection::draw::draw(&ui.painter(), content_rect, snapped);
        }
    }

    // ── Selection marquee drag handling ──
    if let Some((start_music, _)) = drag {
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(content_rect.min, content_rect.max);
                let local = egui::pos2(
                    clamped.x - content_rect.min.x,
                    clamped.y - content_rect.min.y,
                );
                drag = Some((start_music, local));

                let lane_height = view.lane_height;
                crate::selection::drag::auto_scroll_on_drag(
                    ui,
                    &mut view.base,
                    content_rect,
                    pos,
                    |base, w, h| {
                        base.clamp_scroll_x(w, total_ticks);
                        let max_scroll_y = (num_tracks as f32 * lane_height - h).max(0.0);
                        base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
                    },
                );
            }
        }

        let start_pixel = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 * view.lane_height - view.base.scroll_y,
        );

        if pointer.primary_released() {
            if let (Some(_midi_ref), Some((_, end))) = (midi, drag) {
                let drag_dist = (end - start_pixel).length();

                if drag_dist < 3.0 {
                    let tick = view.x_to_tick(start_pixel.x);
                    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
                    selected.clear();
                    *arr_sel_rect = None;
                    *cursor_tick = Some(snapped.max(0.0));
                } else {
                    let (
                        _screen_sx,
                        _screen_ex,
                        _screen_sy,
                        _screen_ey,
                        t_start,
                        t_end,
                        track_lo,
                        track_hi,
                    ) = arrange_snapped_bounds(start_pixel, end, view, quantize, ppq, bar_line_data);

                    if !cmd {
                        selected.clear();
                    }
                    selected.add_rect_track(t_start as u32, t_end as u32, 0, 127, track_lo as u16, track_hi as u16);
                    *arr_sel_rect = Some((t_start, t_end, track_lo, track_hi));
                }
                view.base.dirty = true;
            }
            drag = None;
        }

        if let Some((_, end)) = drag {
            if (end - start_pixel).length() >= 3.0 {
                let (vx, vy, vw, vh, _, _, _, _) =
                    arrange_snapped_bounds(start_pixel, end, view, quantize, ppq, bar_line_data);
                let snapped = egui::Rect::from_min_max(
                    egui::pos2(vx.min(vy), vw.min(vh)),
                    egui::pos2(vx.max(vy), vw.max(vh)),
                );
                crate::selection::draw::draw(&ui.painter(), content_rect, snapped);
            }
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
    ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
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
    let snapped_s = crate::view_interaction::snap_tick(tick_s, quantize, ppq, bar_line_data);
    let snapped_e = crate::view_interaction::snap_tick(tick_e, quantize, ppq, bar_line_data);
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

