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
) {
    let sel_id = ui.id().with("sel_drag_arr");
    // 拖框起始点存音乐坐标 (start_tick, start_track_y)，免疫任何滚动
    // （触摸板滚动、自动滚动、中键拖拽都不会改变音乐坐标）
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

    // Clear stale drag state (e.g. lost window focus mid-drag)
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        // Persist drag state before returning
        ui.data_mut(|d| d.insert_persisted(sel_id, drag));
        return;
    }

    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && content_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let start_tick = view.x_to_tick(local.x);
        let start_track_y = (local.y + view.base.scroll_y) / view.lane_height;
        drag = Some(((start_tick, start_track_y), local));
        // 立即清除上一个选框，避免拖动期间同时出现两个选框
        *arr_sel_rect = None;
        if !cmd {
            selected.clear();
        }
    }

    if let Some((start_music, _)) = drag {
        // Only update end on frames after the initial press
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(content_rect.min, content_rect.max);
                let local = egui::pos2(
                    clamped.x - content_rect.min.x,
                    clamped.y - content_rect.min.y,
                );
                drag = Some((start_music, local));

                // Auto-scroll when dragging near the edge.
                // 起始点用音乐坐标存储，滚动后转回像素自动正确，无需补偿。
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

        // Convert music start to pixel (after any auto-scroll modified view)
        let start_pixel = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 * view.lane_height - view.base.scroll_y,
        );

        if pointer.primary_released() {
            if let (Some(_midi_ref), Some((_, end))) = (midi, drag) {
                let drag_dist = (end - start_pixel).length();

                if drag_dist < 3.0 {
                    // Click (no meaningful drag) — set cursor, clear selection
                    let tick = view.x_to_tick(start_pixel.x);
                    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
                    selected.clear();
                    *arr_sel_rect = None;
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

        // Draw snapped selection rect
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

