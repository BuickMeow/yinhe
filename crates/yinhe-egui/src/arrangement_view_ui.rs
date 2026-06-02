use eframe::egui;

use yinhe_pianoroll::arrangement_instances;
use yinhe_pianoroll::{ArrangementView, NoteInstance, NoteSource, Uniforms};

use super::render_context::RenderContext;

/// States tracked for grid-line cache invalidation.
pub struct ArrangementGridCache {
    pub instances: Vec<NoteInstance>,
    pub ppu: f32,
    pub lb_w: f32,
    pub width: u32,
    pub scroll_x: f32,
}

/// Display the arrangement view texture with zoom/pan interaction.
///
/// `instances` is a reusable scratch buffer — caller should retain it across frames.
/// `grid_cache` holds cached grid-line instances — caller should retain across frames.
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    track_names: &[String],
    instances: &mut Vec<NoteInstance>,
    grid_cache: &mut ArrangementGridCache,
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

    // Clamp scroll
    let total_ticks = midi
        .and_then(|m| m.tick_length())
        .map(|tl| tl as f64 * 1.2)
        .unwrap_or(10000.0);
    let num_tracks = track_visible.len();
    view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);

    // Auto-follow: during playback, scroll so cursor stays visible
    if is_playing {
        if let Some(ct) = *cursor_tick {
            let cursor_x = view.tick_to_x(ct);
            let right_edge = w as f32;
            let margin = (right_edge - view.label_width) * 0.2;
            if cursor_x > right_edge - margin || cursor_x < view.label_width {
                view.scroll_x = (ct as f32 * view.pixels_per_tick)
                    - (right_edge - view.label_width) * 0.5;
                view.clamp_scroll(w as f32, h as f32, total_ticks, num_tracks);
            }
        }
    }

    // Mark dirty during playback — cursor line needs to move each frame
    if is_playing && cursor_tick.is_some() {
        view.dirty = true;
    }

    // ── Compute uniforms ──
    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: view.scroll_x,
        scroll_y: view.scroll_y,
        pixels_per_tick: view.pixels_per_tick,
        key_height: view.lane_height,
        keyboard_width: view.label_width,
        _pad: 0.0,
    };

    // Only rebuild instances if view state or uniforms changed
    let need_rebuild = view.dirty || renderer.uniforms_changed(&uniforms);

    if need_rebuild {
        // ── Grid cache management ──
        let grid_invalid = grid_cache.instances.is_empty()
            || grid_cache.ppu != view.pixels_per_tick
            || grid_cache.lb_w != view.label_width
            || grid_cache.width != w;

        if grid_invalid {
            // Full grid rebuild: ppu, width, or label_width changed
            grid_cache.instances.clear();
            if let Some(m) = midi {
                if let Some(tpb) = m.ticks_per_beat() {
                    // Encode tick in flags field for boundary handling during scroll
                    arrangement_instances::build_arrangement_grid(
                        &mut grid_cache.instances, w as f32, h as f32, view, tpb,
                    );
                }
            }
            grid_cache.ppu = view.pixels_per_tick;
            grid_cache.lb_w = view.label_width;
            grid_cache.width = w;
            grid_cache.scroll_x = view.scroll_x;
        } else {
            // Only scroll changed: update cached grid x-positions by delta
            let dx = view.scroll_x - grid_cache.scroll_x;
            if dx.abs() > 0.001 {
                for inst in &mut grid_cache.instances {
                    inst.x -= dx;
                }
                // Add boundary grid lines that scrolled into view
                extend_grid_boundary(
                    &mut grid_cache.instances, w as f32, h as f32, view, midi,
                );
                // Remove lines that scrolled completely out of view
                let lb_w = view.label_width;
                grid_cache.instances.retain(|inst| {
                    inst.x >= lb_w - 200.0 && inst.x <= w as f32 + 200.0
                });
                grid_cache.scroll_x = view.scroll_x;
            }
        }

        // ── Build instances using scratch buffer ──
        let mut scratch = std::mem::take(instances);
        scratch.clear();

        let grid = if grid_cache.instances.is_empty() {
            None
        } else {
            Some(&grid_cache.instances[..])
        };
        arrangement_instances::build_arrangement_instances(
            &mut scratch,
            w,
            h,
            midi,
            view,
            track_visible,
            track_colors,
            *cursor_tick,
            grid,
        );

        // ── Upload to GPU ──
        renderer.prepare_from_parts(uniforms, &scratch);

        // Return scratch buffer to caller
        scratch.clear();
        *instances = scratch;
    }
    view.dirty = false;

    // ── Render to offscreen texture ──
    let mut encoder = render_ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("arrangement_frame"),
        });
    renderer.draw(&mut encoder, render_ctx.preview_view(), w, h);
    render_ctx.queue().submit(std::iter::once(encoder.finish()));

    // ── Display the texture in egui ──
    painter.image(
        texture_id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    // ── Draw track label overlays ──
    let (trk_first, trk_last) = view.visible_track_range(h as f32, num_tracks);
    for idx in trk_first..trk_last {
        if !track_visible.get(idx).copied().unwrap_or(true) {
            continue;
        }

        let name = track_names.get(idx).map(|s| s.as_str()).unwrap_or("");
        let y = view.lane_y(idx) + rect.min.y;
        let lh = view.lane_height;
        let lw = view.label_width;

        // Background strip for label area
        let label_bg = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, y),
            egui::vec2(lw, lh),
        );
        let bg_alpha = if idx % 2 == 0 { 0.08 } else { 0.04 };
        painter.rect_filled(
            label_bg,
            0.0,
            egui::Color32::BLACK.gamma_multiply(bg_alpha),
        );

        // Color indicator
        let color = track_colors.get(idx).copied().unwrap_or([0.5, 0.5, 0.5]);
        let color32 = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );
        let indicator_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 4.0, y + 4.0),
            egui::vec2(6.0, lh - 8.0),
        );
        painter.rect_filled(indicator_rect, 2.0, color32);

        // Track name
        painter.text(
            egui::pos2(rect.min.x + 14.0, y + lh * 0.5),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE.gamma_multiply(0.85),
        );
    }

    // ── Handle input ──
    let mut changed = false;
    if resp.hovered() {
        let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
        let pointer_x = pointer_pos.x - rect.min.x;

        // Trackpad pinch gesture → horizontal zoom
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            view.zoom_around_x(pointer_x, zoom_delta);
            changed = true;
        }

        // Cmd+scroll: zoom; plain scroll: pan
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if scroll != egui::Vec2::ZERO {
            if cmd {
                if scroll.y.abs() > 0.5 {
                    let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    view.zoom_around_x(pointer_x, factor);
                }
            } else {
                view.scroll_x -= scroll.x;
                view.scroll_y -= scroll.y;
                view.dirty = true;
            }
            changed = true;
        }
    }

    // Click to seek cursor position
    let released = ui.input(|i| i.pointer.primary_released());
    let drag_dist = resp.drag_delta().length();
    if released && resp.hovered() && drag_dist < 3.0 {
        if let Some(pos) = resp.interact_pointer_pos() {
            let pointer_x = pos.x - rect.min.x;
            if pointer_x >= view.label_width {
                let tick = view.x_to_tick(pointer_x);
                *cursor_tick = Some(tick.max(0.0));
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
        *view = ArrangementView::default();
        changed = true;
    }

    if changed {
        ui.ctx().request_repaint();
    }
}

/// Add grid lines at the right (or left) boundary that scrolled into the
/// visible range but are not yet in the cache.
///
/// Reuses `flags` field to store the original tick value for boundary
/// detection.
fn extend_grid_boundary(
    cached: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    view: &ArrangementView,
    midi: Option<&dyn NoteSource>,
) {
    let tpb = match midi.and_then(|m| m.ticks_per_beat()) {
        Some(t) => t,
        None => return,
    };
    let ppu = view.pixels_per_tick;
    if ppu <= 0.01 {
        return;
    }

    let (tick_start, tick_end) = view.visible_tick_range(w);
    let ticks_per_sub = (tpb / 4).max(1);
    let lb_w = view.label_width;
    let x_origin = lb_w - view.scroll_x;

    // Find current min/max tick in cache (stored in flags field)
    let mut min_tick = u32::MAX;
    let mut max_tick = 0u32;
    for inst in cached.iter() {
        let t = inst.flags;
        if t < min_tick {
            min_tick = t;
        }
        if t > max_tick {
            max_tick = t;
        }
    }

    let ticks_per_measure = tpb * 4;

    // Re-compute aligned start for boundary tick generation
    let aligned_start = ((tick_start / ticks_per_sub as f64).floor() as u32)
        .saturating_mul(ticks_per_sub);

    // Right boundary: ticks from max_tick + step to tick_end
    let start_right = (max_tick / ticks_per_sub + 1) * ticks_per_sub;
    if start_right as f64 <= tick_end {
        let mut tick = start_right;
        while (tick as f64) <= tick_end {
            let x = x_origin + tick as f32 * ppu;
            if x >= lb_w && x <= w {
                let is_measure = tick % ticks_per_measure == 0;
                let is_beat = tick % tpb == 0;
                if is_measure {
                    cached.push(NoteInstance {
                        x,
                        y: 0.0,
                        w: 2.0,
                        h,
                        rgba_packed: yinhe_pianoroll::pack_rgba(
                            0.30, 0.30, 0.35, 1.0,
                        ),
                        props_packed: yinhe_pianoroll::pack_props(0.0, 0.0),
                        velocity: 0,
                        flags: tick,
                    });
                } else if is_beat {
                    cached.push(NoteInstance {
                        x,
                        y: 0.0,
                        w: 1.0,
                        h,
                        rgba_packed: yinhe_pianoroll::pack_rgba(
                            0.20, 0.20, 0.23, 1.0,
                        ),
                        props_packed: yinhe_pianoroll::pack_props(0.0, 0.0),
                        velocity: 0,
                        flags: tick,
                    });
                }
            }
            tick += ticks_per_sub;
        }
    }

    // Left boundary: ticks from aligned_start to min_tick - step
    let start_left = (aligned_start / ticks_per_sub) * ticks_per_sub;
    let end_left = ((min_tick.saturating_sub(ticks_per_sub)) / ticks_per_sub) * ticks_per_sub;
    if start_left < min_tick {
        let mut tick = start_left;
        while tick <= end_left && tick + ticks_per_sub <= min_tick {
            let x = x_origin + tick as f32 * ppu;
            if x >= lb_w && x <= w {
                let is_measure = tick % ticks_per_measure == 0;
                let is_beat = tick % tpb == 0;
                if is_measure {
                    cached.push(NoteInstance {
                        x,
                        y: 0.0,
                        w: 2.0,
                        h,
                        rgba_packed: yinhe_pianoroll::pack_rgba(
                            0.30, 0.30, 0.35, 1.0,
                        ),
                        props_packed: yinhe_pianoroll::pack_props(0.0, 0.0),
                        velocity: 0,
                        flags: tick,
                    });
                } else if is_beat {
                    cached.push(NoteInstance {
                        x,
                        y: 0.0,
                        w: 1.0,
                        h,
                        rgba_packed: yinhe_pianoroll::pack_rgba(
                            0.20, 0.20, 0.23, 1.0,
                        ),
                        props_packed: yinhe_pianoroll::pack_props(0.0, 0.0),
                        velocity: 0,
                        flags: tick,
                    });
                }
            }
            tick += ticks_per_sub;
        }
    }
}
