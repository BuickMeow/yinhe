use std::collections::HashSet;

use eframe::egui;

use yinhe_wgpu::{build_ghost_notes, build_arr_grid, build_arr_notes};
use yinhe_core::TrackInfo;
use yinhe_types::{ArrangementView, NoteSource, key_notes_in_range};
use yinhe_wgpu::{InstanceRenderer, Uniforms, TrackColorsUniform, MAX_TRACKS};
use yinhe_types::TimeSigEvent;
use yinhe_wgpu::layer_cache_key;

use yinhe_editor_core::quantize::QuantizePreset;
use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

/// Display the arrangement view texture with zoom/pan interaction.
///
/// Uses the layered cache API: decor (layer 0), grid (layer 1), notes (layer 2),
/// ghost notes (layer 3, no cache).  The playhead cursor is drawn by egui on top
/// of the wgpu texture.
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
    track_info: &[TrackInfo],
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
    revision: u64,
    arr_sel_rect: &mut Option<(f64, f64, usize, usize)>,
    arr_drag_delta: &mut Option<(i64, i32)>,
    arr_eraser_rect: &mut Option<(f64, f64, usize, usize)>,
    track_selected: &mut HashSet<u16>,
    selection_anchor: &mut Option<u16>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
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
        note_outline: 1, // AR mode: outline always on
        lane_height: view.lane_height, // AR: per-track lane height
        note_alpha: 1.0,
    };

    view.base.dirty = false;

    let theme = renderer.theme().clone();
    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(tc_uniform);
    renderer.ensure_layers(3);

    // ── Select tool dispatch (BEFORE layer building to get ghost notes) ──
    // Like PR's sel_drag_frame, this returns ghost_notes/hidden_notes generated
    // from the CURRENT frame's mouse position, enabling zero-delay ghost preview.
    let (ghost_notes, hidden_notes, drag_rect) = if *active_tool == Tool::Select {
        sel_drag_frame_arrange(
            ui,
            rect,
            view,
            midi,
            selected,
            track_visible,
            track_info,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            num_tracks,
            cursor_tick,
            arr_sel_rect,
            arr_drag_delta,
            track_selected,
            selection_anchor,
            info_content,
        )
    } else {
        (Vec::new(), HashSet::new(), None)
    };

    let vh = view.render_hash();
    let wh = {
        let mut hash: u64 = 0;
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(w as u64);
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(h as u64);
        hash
    };

    // tv_hash still needed for notes layer cache key
    let tv_hash = {
        let mut h = 0u64;
        for &v in track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };

    // Layer 0: grid lines (background + track lanes now drawn by egui)
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
    renderer.upload_layer(0, grid_key, |out| {
        if let Some(midi) = midi
            && let Some(tpb) = midi.ticks_per_beat()
        {
            let (def_num, def_den) = midi.time_sig_default();
            let sig_events = midi.time_sig_events();
            build_arr_grid(
                out, w as f32, h as f32, view, tpb, def_num, def_den, sig_events, scroll_x_pos, &theme,
            );
        }
    });

    // Layer 1: notes (16B NoteInstance — shader computes pixel positions from uniforms)
    let notes_key = layer_cache_key(&[vh, wh, tv_hash, revision, hidden_notes.len() as u64]);
    renderer.upload_note_layer(1, notes_key, |out| {
        if let Some(midi) = midi {
            build_arr_notes(
                out,
                w as f32,
                h as f32,
                midi,
                view,
                track_visible,
                &hidden_notes,
            );
        }
    });

    // Layer 2: ghost notes (no cache — rebuilt every frame during drag)
    renderer.upload_note_layer(2, 0, |out| {
        build_ghost_notes(out, &ghost_notes);
    });

    let content_changed = true;

    // ── Background + track lanes (drawn by egui before wgpu texture) ──
    let lb_w = view.base.left_panel_width;
    let (r, g, b) = theme.ar_bg;
    painter.rect_filled(
        rect,
        0.0,
        egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8),
    );
    let lh = view.lane_height;
    let scroll_y = view.base.scroll_y;
    let num_tracks = track_visible.len();
    if num_tracks > 0 {
        let (trk_first, trk_last) = ArrangementView::visible_track_range_static(scroll_y, h as f32, lh, num_tracks);
        for idx in trk_first..trk_last {
            if !track_visible.get(idx).copied().unwrap_or(true) {
                continue;
            }
            let y = rect.min.y + ArrangementView::lane_y_static(idx, scroll_y, lh);
            let col = if idx % 2 == 0 { theme.ar_lane_even } else { theme.ar_lane_odd };
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x + lb_w, y),
                    egui::vec2(w as f32 - lb_w, lh),
                ),
                0.0,
                egui::Color32::from_rgb((col.0 * 255.0) as u8, (col.1 * 255.0) as u8, (col.2 * 255.0) as u8),
            );
        }
    }

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

    // ── Draw drag selection rect (move-drag offset or marquee) on top of GPU texture ──
    if let Some(dr) = drag_rect {
        crate::selection::draw::draw(&ui.painter(), rect, dr, egui::Color32::WHITE, egui::Color32::WHITE);
    }

    // ── Eraser tool dispatch (after GPU texture, before eraser marquee drawing) ──
    if *active_tool == Tool::Eraser {
        eraser_drag_frame_arrange(
            ui,
            rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            num_tracks,
            arr_eraser_rect,
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
        crate::selection::draw::draw(&ui.painter(), rect, snapped, egui::Color32::WHITE, egui::Color32::WHITE);
    }

    // Draw eraser marquee box in red (active during drag)
    if *active_tool == Tool::Eraser {
        let drag_id = ui.id().with("eraser_drag_arr");
        let drag: Option<((f64, f32), egui::Pos2)> =
            ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);
        if let Some((start_music, end)) = drag {
            let start_pixel = egui::pos2(
                view.tick_to_x(start_music.0),
                start_music.1 * view.lane_height - view.base.scroll_y,
            );
            if (end - start_pixel).length() >= 3.0 {
                let (vx, vy, vw, vh, _, _, _, _) =
                    arrange_snapped_bounds(start_pixel, end, view, quantize, ppq, bar_line_data);
                let snapped = egui::Rect::from_min_max(
                    egui::pos2(vx.min(vy), vw.min(vh)),
                    egui::pos2(vx.max(vy), vw.max(vh)),
                );
                crate::selection::draw::draw(&ui.painter(), rect, snapped, egui::Color32::RED, egui::Color32::RED);
            }
        }
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

/// Returns `(ghost_notes, hidden_notes, drag_rect)` for move-drag preview.
///
/// `ghost_notes`: `(start_tick, end_tick, key, track)` — preview notes at new positions.
/// `hidden_notes`: `(track, start_tick, key)` — original notes to hide during drag.
/// `drag_rect`: the selection rect to draw on top of the GPU texture (move-drag offset
///   rect or marquee rect). `None` on release (arr_sel_rect takes over).
fn sel_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    selected: &mut yinhe_core::Selection,
    track_visible: &[bool],
    track_info: &[TrackInfo],
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    num_tracks: usize,
    cursor_tick: &mut Option<f64>,
    arr_sel_rect: &mut Option<(f64, f64, usize, usize)>,
    arr_drag_delta: &mut Option<(i64, i32)>,
    track_selected: &mut HashSet<u16>,
    selection_anchor: &mut Option<u16>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
) -> (Vec<(f64, f64, u8, u16)>, HashSet<(u16, u32, u8)>, Option<egui::Rect>) {
    let mut ghost_notes: Vec<(f64, f64, u8, u16)> = Vec::new();
    let mut hidden_notes: HashSet<(u16, u32, u8)> = HashSet::new();
    let mut drag_rect: Option<egui::Rect> = None;

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
        return (ghost_notes, hidden_notes, drag_rect);
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

    // ── Move-drag: update current position ──
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
    }

    // ── Generate ghost notes + drag_rect from current move_drag (BEFORE release) ──
    // Ghost notes must be generated before release clears move_drag, so the ghost
    // stays visible on the release frame (preventing flicker before model update).
    if let Some(((origin_t, origin_tr), (current_t, current_tr))) = move_drag {
        if let Some((t_start, t_end, track_lo, track_hi)) = move_orig_sel {
            let snapped_origin = crate::view_interaction::snap_tick(origin_t, quantize, ppq, bar_line_data);
            let snapped_current = crate::view_interaction::snap_tick(current_t, quantize, ppq, bar_line_data);
            let dt = (snapped_current - snapped_origin).round() as i64;
            let dtr = (current_tr - origin_tr).round() as i32;

            // Compute drag_rect (offset selection rect) for display
            let lh = view.lane_height;
            let scroll_y = view.base.scroll_y;
            let new_track_lo = (track_lo as i32 + dtr).max(0) as usize;
            let new_track_hi = (track_hi as i32 + dtr).max(0) as usize;
            let sy = new_track_lo as f32 * lh - scroll_y;
            let ey = (new_track_hi as f32 + 1.0) * lh - scroll_y;
            let sx = view.tick_to_x(t_start + dt as f64);
            let ex = view.tick_to_x(t_end + dt as f64);
            drag_rect = Some(egui::Rect::from_min_max(
                egui::pos2(sx.min(ex), sy.min(ey)),
                egui::pos2(sx.max(ex), sy.max(ey)),
            ));

            // Generate ghost notes at new positions + hide originals
            if dt != 0 || dtr != 0 {
                let ts = t_start as u32;
                let te = t_end as u32;
                let tl = track_lo as u16;
                let th = track_hi as u16;
                let max_track = (num_tracks as i32 - 1).max(0) as u16;

                if let Some(midi) = midi {
                    for key in 0u8..128u8 {
                        let notes = key_notes_in_range(midi.key_notes(key), ts, te);
                        for note in notes {
                            if note.track < tl || note.track > th {
                                continue;
                            }
                            if !selected.contains(note.track, note.start_tick, key) {
                                continue;
                            }
                            if !track_visible.get(note.track as usize).copied().unwrap_or(true) {
                                continue;
                            }
                            let new_tick = (note.start_tick as i64 + dt).max(0) as u32;
                            let length = note.end_tick - note.start_tick;
                            let new_track = (note.track as i32 + dtr).max(0).min(max_track as i32) as u16;
                            ghost_notes.push((
                                new_tick as f64,
                                (new_tick + length) as f64,
                                key,
                                new_track,
                            ));
                            hidden_notes.insert((note.track, note.start_tick, key));
                        }
                    }
                }
            }
        }
    }

    // ── Move-drag: release handling ──
    if move_drag.is_some() && pointer.primary_released() {
        if let Some(((origin_t, origin_tr), (current_t, current_tr))) = move_drag {
            let snapped_origin = crate::view_interaction::snap_tick(origin_t, quantize, ppq, bar_line_data);
            let snapped_current = crate::view_interaction::snap_tick(current_t, quantize, ppq, bar_line_data);
            let delta_ticks = (snapped_current - snapped_origin).round() as i64;
            let delta_tracks = (current_tr - origin_tr).round() as i32;

            let has_moved = delta_ticks != 0 || delta_tracks != 0;

            if has_moved {
                *arr_drag_delta = Some((delta_ticks, delta_tracks));

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
                *arr_sel_rect = move_orig_sel;
            }
        }
        move_drag = None;
        move_orig_sel = None;
        drag_rect = None; // arr_sel_rect takes over on release
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

        // Compute marquee drag_rect (BEFORE release, same pattern as move-drag)
        if let Some((_, end)) = drag {
            if (end - start_pixel).length() >= 3.0 {
                let (vx, vy, vw, vh, _, _, _, _) =
                    arrange_snapped_bounds(start_pixel, end, view, quantize, ppq, bar_line_data);
                drag_rect = Some(egui::Rect::from_min_max(
                    egui::pos2(vx.min(vy), vw.min(vh)),
                    egui::pos2(vx.max(vy), vw.max(vh)),
                ));
            }
        }

        if pointer.primary_released() {
            if let (Some(_midi_ref), Some((_, end))) = (midi, drag) {
                let drag_dist = (end - start_pixel).length();

                if drag_dist < 3.0 {
                    let tick = view.x_to_tick(start_pixel.x);
                    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
                    selected.clear();
                    *arr_sel_rect = None;
                    *cursor_tick = Some(snapped.max(0.0));

                    // 点击时同时选中对应音轨
                    let track_arr_idx = start_music.1.floor() as usize;
                    if track_arr_idx < num_tracks {
                        let track_idx = track_info[track_arr_idx].index;
                        track_selected.clear();
                        track_selected.insert(track_idx);
                        *selection_anchor = Some(track_idx);
                        *info_content = Some(crate::right_panel::InfoContent::Track);
                    }
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
            drag_rect = None; // arr_sel_rect takes over on release
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
    ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));

    (ghost_notes, hidden_notes, drag_rect)
}

// ── Arrangement eraser tool ──

/// Eraser-tool marquee drag for the arrangement view.
/// On release, sets `arr_eraser_rect` which triggers deletion in the caller.
fn eraser_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    view: &mut ArrangementView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    num_tracks: usize,
    arr_eraser_rect: &mut Option<(f64, f64, usize, usize)>,
) {
    let drag_id = ui.id().with("eraser_drag_arr");
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }

    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(drag_id, drag));
        return;
    }

    // Press → start drag (store music coordinates: (tick, track_f))
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && content_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let start_tick = view.x_to_tick(local.x);
        let start_track_f = (local.y + view.base.scroll_y) / view.lane_height;
        drag = Some(((start_tick, start_track_f), local));
        *arr_eraser_rect = None;
    }

    // Move → update with auto-scroll
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
                    ui, &mut view.base, content_rect, pos,
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

        // Release → compute snapped bounds, set eraser rect
        if pointer.primary_released() {
            if let Some((_, end)) = drag {
                if (end - start_pixel).length() >= 3.0 {
                    let (_, _, _, _, t_start, t_end, track_lo, track_hi) =
                        arrange_snapped_bounds(start_pixel, end, view, quantize, ppq, bar_line_data);
                    *arr_eraser_rect = Some((t_start, t_end, track_lo, track_hi));
                }
                view.base.dirty = true;
            }
            drag = None;
        }
    }

    ui.data_mut(|d| d.insert_persisted(drag_id, drag));
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

