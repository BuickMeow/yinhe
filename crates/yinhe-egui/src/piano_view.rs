use std::sync::Arc;

use eframe::egui;

use yinhe_types::{AutomationLane, TimeSigEvent};

use yinhe_editor_core::quantize::QuantizePreset;
pub use yinhe_types::PencilNoteDrag;
use crate::widgets::tools_panel::Tool;
use crate::widgets::selection_actions::SelectionAction;

pub mod automation_panel;
mod drag;
mod pencil;

/// Events emitted by the piano-roll view for the caller to act on.
pub enum PianoViewEvent {
    SelectionAction(SelectionAction),
    AddNote { track: u16, note: yinhe_core::NoteEvent },
    EraserDelete { t_start: u32, t_end: u32, key_lo: u8, key_hi: u8, track_lo: u16, track_hi: u16 },
    QuantizePreset(QuantizePreset),
}

/// Height of the time ruler band at the top of the pianoroll view.
use crate::theme;
const RULER_H: f32 = theme::RULER_H;

/// Display the pianoroll texture with zoom/pan interaction.
///
/// When `auto_*` parameters are `Some`, automation panels are rendered between
/// the pianoroll content and the horizontal scrollbar. The AUTO toggle and
/// +/- buttons live inside the scrollbar's left blank area (same width as the
/// piano keyboard).
///
/// Returns an optional event for the caller to handle (selection action or
/// note-add request).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_wgpu::InstanceRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    render_thread: Option<&yinhe_wgpu::RenderThreadHandle>,
    view: &mut yinhe_types::PianoRollView,
    last_cull_revision: &mut u64, // revision ^ hidden_hash — triggers all_notes re-upload
    last_cull_revision_only: &mut u64, // last revision for incremental detection
    last_hidden_hash: &mut u64, // last hidden_hash for incremental detection
    midi: Option<&dyn yinhe_types::NoteSource>,
    selected: &mut yinhe_core::Selection,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    last_cursor_tick: &mut Option<f64>,
    follow_mode: &mut super::view_interaction::FollowMode,
    active_tool: &Tool,
    // Automation panel data (all-or-nothing)
    auto_panels: Option<&mut Vec<yinhe_types::AutomationPanelView>>,
    auto_renderers: Option<
        &mut Vec<(
            yinhe_wgpu::InstanceRenderer,
            super::render_context::RenderContext,
        )>,
    >,
    auto_lanes: Option<&[AutomationLane]>,
    auto_show: Option<&mut bool>,
    auto_wgpu_state: Option<&Arc<eframe::egui_wgpu::RenderState>>,
    scroll_mode: u32,
    min_border_width: f32,
    velocity_display_mode: &mut u32,
    note_outline: bool,
    tempo_events: &[(u32, f64)],
    note_drag_delta: &mut Option<(i64, i32)>,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    revision: u64,
    note_revisions: &[u64; 128],
    haptic_engine: Option<&yinhe_haptic::HapticEngine>,
    pencil_note_drag: &mut Option<PencilNoteDrag>,
    auto_edit_events: &mut Vec<crate::piano_view::automation_panel::AutomationEdit>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    right_tab: &mut Option<crate::right_panel::RightTab>,
    automation_drag_ghost: &mut Option<(u32, u16)>,
) -> Option<PianoViewEvent> {
    // Sense::hover() — no drag ownership. All drag is handled by dedicated
    // ui.interact calls below, each inside its own push_id scope.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;

    // Compute automation panel natural total height.
    // First panel has no leading handle; subsequent panels have SPLIT_H above them.
    let panels_natural_h: f32 = match (&auto_panels, &auto_show) {
        (Some(panels), Some(show)) if **show && !panels.is_empty() => {
            panels.iter().map(|p| p.panel_height).sum::<f32>()
                + (panels.len() as f32 * automation_panel::SPLIT_H)
        }
        _ => 0.0,
    };

    // Cap panels area to prevent overflow when too many panels.
    // Reserve at least 35% of available height for the pianoroll content;
    // excess panels become scrollable.
    let avail_h = rect.height() - RULER_H - crate::widgets::scrollbar::SCROLLBAR_H;
    let panels_max_h = (avail_h * 0.65).max(0.0);
    let panels_total_h = panels_natural_h.min(panels_max_h);

    // Layout: ruler | pianoroll content | automation panels | scrollbar
    let ruler_band_y = rect.min.y;
    let content_y = rect.min.y + RULER_H;
    let content_h = (avail_h - panels_total_h).max(0.0);
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, content_y),
        egui::pos2(rect.max.x, content_y + content_h),
    );
    let kb_w = view.keyboard_width();
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + kb_w, content_y),
        egui::pos2(rect.max.x, content_y + content_h),
    );
    let ppp = ui.ctx().pixels_per_point();
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;

    if w == 0 || h == 0 {
        return None;
    }

    // ── Perf probe (only when YIN_PERF=1) ──
    let perf_on = yinhe_memtrace::perf_probe::enabled();
    let t_show_start = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(pw, ph);

    // Update render thread's target view if texture was recreated
    if let Some(rt) = render_thread {
        rt.update_target(render_ctx.preview_view().clone(), pw, ph);
    }

    // Clamp scroll — add some extra space beyond the last note
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
        ppq,
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Auto-follow: scroll based on follow mode (playback only).
    // Never auto-follow when paused, so the user can freely scroll around.
    if let Some(ct) = *cursor_tick
        && is_playing
        && *follow_mode != super::view_interaction::FollowMode::None
    {
        if let Some(new_scroll_x) = super::view_interaction::compute_follow_scroll(
            ct,
            view.base.pixels_per_tick,
            w as f32,
            view.keyboard_width(),
            *follow_mode,
            1.0,
        ) {
            view.base.scroll_x = new_scroll_x;
            view.clamp_scroll(w as f32, h as f32, total_ticks);
        }
    }

    // ── Selection drag (Select tool only) ──
    // Update state BEFORE handle_input to avoid egui pointer-capture conflicts.
    let mut sel_action = None;
    let mut pencil_event: Option<PianoViewEvent> = None;
    let mut eraser_event: Option<PianoViewEvent> = None;
    let mut ghost_notes: Vec<(f64, f64, u8, u16)> = Vec::new();
    let mut hidden_notes: std::collections::HashSet<(u16, u32, u8)> = std::collections::HashSet::new();
    if *active_tool == Tool::Select {
        let (sel_ghosts, sel_hidden) = drag::sel_drag_frame(
            ui,
            content_rect,
            music_rect,
            view,
            midi,
            selected,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            cursor_tick,
            note_drag_delta,
            sel_rect,
            track_colors,
            track_visible,
            track_selected,
        );
        ghost_notes = sel_ghosts;
        hidden_notes = sel_hidden.into_iter().collect();
    } else if *active_tool == Tool::Pencil {
        let (note_event, ghost, hidden, pencil_drag) = pencil::pencil_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            track_selected,
            conductor_idx,
            midi,
            track_colors,
        );
        ghost_notes = ghost;
        hidden_notes.extend(hidden);
        *pencil_note_drag = pencil_drag;
        if let Some(note) = note_event {
            if let Some(track) = pencil::valid_pencil_track(track_selected, conductor_idx) {
                pencil_event = Some(PianoViewEvent::AddNote { track, note });
            }
        }
    } else if *active_tool == Tool::Eraser {
        eraser_event = drag::eraser_drag_frame(
            ui, content_rect, music_rect, view, quantize, ppq, bar_line_data, total_ticks,
            track_selected,
        );
    }

    // ── Hover cursor: show Move when over selection rect ──
    if *active_tool == Tool::Select
        && !crate::view_interaction::pointer_over_popup(ui.ctx())
    {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if music_rect.contains(pos) {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                let eff = sel_rect.effective();
                let in_sel_rect = eff.is_some_and(|(t_start, t_end, key_lo, key_hi)| {
                    let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                        &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
                    );
                    pixel_rect.contains(local)
                });
                if in_sel_rect {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                }
            }
        }
    }

    // ── Content interaction (zoom/pan/cursor/drag/reset) ──
    // Save scroll state before input for haptic boundary detection
    let pre_scroll_x = view.base.scroll_x;
    let pre_scroll_y = view.base.scroll_y;
    let raw_scroll = ui.input(|i| i.smooth_scroll_delta);
    crate::view_interaction::handle_input(
        ui,
        music_rect,
        view,
        cursor_tick,
        0.0,
        Some((quantize, ppq)),
        bar_line_data,
        None,
        is_playing,
        follow_mode,
        active_tool,
    );

    // ── Keyboard area: vertical zoom (pinch / cmd+scroll) ──
    ui.push_id("kb_zoom", |ui| {
        let kb_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, content_y),
            egui::pos2(rect.min.x + kb_w, content_y + content_h),
        );
        let pointer_in_kb = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| kb_rect.contains(p)));
        if pointer_in_kb {
            let zoom_delta = ui.input(|i| i.zoom_delta());
            if (zoom_delta - 1.0).abs() > 0.001 {
                let pointer_y = ui.input(|i| i.pointer.hover_pos().unwrap_or_default()).y - content_y;
                view.zoom_around_y(pointer_y, zoom_delta, content_h);
                view.base.dirty = true;
                ui.ctx().request_repaint();
            }
            let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            if cmd && scroll.y.abs() > 0.5 {
                let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                let pointer_y = ui.input(|i| i.pointer.hover_pos().unwrap_or_default()).y - content_y;
                view.zoom_around_y(pointer_y, factor, content_h);
                view.base.dirty = true;
                ui.ctx().request_repaint();
            }
        }
    });

    // ── Keyboard resize handle ──
    ui.push_id("kb_handle", |ui| {
        let handle_x = rect.min.x + view.keyboard_width();
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(handle_x - 2.0, rect.min.y),
            egui::pos2(handle_x + 2.0, content_rect.max.y),
        );
        let handle_resp = ui.interact(handle_rect, ui.id(), egui::Sense::click_and_drag());
        if handle_resp.hovered() || handle_resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if handle_resp.dragged() {
            let delta = handle_resp.drag_delta().x;
            let old_kb = view.keyboard_width();
            let new_kb = (old_kb + delta).clamp(
                crate::theme::MIN_KEYBOARD_WIDTH,
                rect.width() * crate::theme::MAX_KEYBOARD_RATIO,
            );

            let old_sb_w = w as f32 - old_kb;
            let new_sb_w = w as f32 - new_kb;
            if old_sb_w > 0.0 && new_sb_w > 0.0 {
                let start_tick = view.base.scroll_x / view.base.pixels_per_tick;
                let new_start_tick = start_tick * old_sb_w / new_sb_w;
                view.base.scroll_x = new_start_tick * view.base.pixels_per_tick;
            }

            view.base.left_panel_width = new_kb;
            view.base.dirty = true;
            ui.ctx().request_repaint();
        }
    });

    // ── Clamp scroll after all interactions ──
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.map(|m| m.tick_length().unwrap_or(0)).unwrap_or(0),
        ppq,
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // ── Haptic boundary feedback ──
    let max_sx = (total_ticks as f32 * view.base.pixels_per_tick - (w as f32 - view.base.left_panel_width)).max(0.0);
    let max_sy = (view.total_key_height() - h as f32).max(0.0);
    crate::view_interaction::notify_haptic_boundary(
        yinhe_haptic::HapticSlot::PianoRoll,
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
        yinhe_haptic::HapticSlot::PianoRoll,
        view.base.pixels_per_tick,
        view.key_height,
        0.001,
        10.0,
        h as f32 / 128.0,
        60.0,
        haptic_engine,
    );

    // ── Dirty detection ──
    // cursor_tick no longer affects rendering at all — the cursor is drawn
    // by egui directly on top of the wgpu texture, outside the cache.
    // app_eframe already calls request_repaint while audio is playing.
    *last_cursor_tick = *cursor_tick;

    // Perf probe: capture input phase duration (everything up to prepare).
    let t_input_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Upload all notes to GPU cull buffer ──
    // Strategy: try incremental per-key upload first, fallback to full upload.
    // - hidden_notes changed → must full upload (affects per-key content)
    // - revision changed, per-key revision matches → skip (already uploaded)
    // - revision changed, some keys differ → try incremental (count must match)
    // - revision changed, count mismatch → full upload
    let note_key = yinhe_wgpu::NoteBufferKey::new(revision, track_visible, &hidden_notes);
    if note_key.value() != *last_cull_revision {
        if let Some(midi_src) = midi {
            // Check if only hidden_notes changed (revision same, hidden_hash different)
            let revision_changed = revision != *last_cull_revision_only;
            let hidden_changed = yinhe_wgpu::hash_hidden(&hidden_notes) != *last_hidden_hash;

            if hidden_changed && !revision_changed {
                // Only hidden_notes changed → must full upload
                let (all_notes, offsets) = yinhe_wgpu::build_all_notes(
                    midi_src, &hidden_notes, track_visible,
                );
                pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
            } else if revision_changed {
                // Revision changed → try incremental per-key upload
                let uploaded = pianoroll.uploaded_key_revisions();
                let dirty_keys: Vec<u8> = (0u8..128)
                    .filter(|&k| note_revisions[k as usize] != uploaded[k as usize])
                    .collect();

                if dirty_keys.is_empty() {
                    // Revision bumped but no key revisions changed (e.g. conductor-only edit)
                    // Just update tracking, no re-upload needed.
                } else {
                    // Try incremental: build + upload each dirty key
                    let mut all_ok = true;
                    for &key in &dirty_keys {
                        let key_notes = yinhe_wgpu::build_key_notes(
                            midi_src, key, &hidden_notes, track_visible,
                        );
                        if !pianoroll.try_incremental_key_upload(
                            key, &key_notes, note_revisions[key as usize],
                        ) {
                            all_ok = false;
                            break;
                        }
                    }

                    if !all_ok {
                        // Fallback: full upload (some key's count changed)
                        let (all_notes, offsets) = yinhe_wgpu::build_all_notes(
                            midi_src, &hidden_notes, track_visible,
                        );
                        pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
                    }
                }
            }
        }
        *last_cull_revision = note_key.value();
        *last_cull_revision_only = revision;
        *last_hidden_hash = yinhe_wgpu::hash_hidden(&hidden_notes);
    }

    // Prepare GPU data (ghost notes are handled separately as a transient overlay)
    let theme = pianoroll.theme().clone();
    let cull_ready = pianoroll.cull_ready();
    if cull_ready {
        // GPU cull path: upload decor layers + ghost layer (GPU cull handles notes)
        let job = yinhe_wgpu::build_render_job(
            w, h, midi, view, &*selected,
            track_colors, scroll_mode, min_border_width,
            note_outline, &theme,
        );
        pianoroll.upload_uniforms(job.uniforms);
        pianoroll.upload_track_colors(&job.track_colors);
        pianoroll.upload_selection(&job.selection);
        pianoroll.ensure_layers(job.decor_layers.len());
        for (i, dl) in job.decor_layers.iter().enumerate() {
            pianoroll.upload_layer(i, dl.cache_key, |out| {
                out.extend_from_slice(&dl.instances);
            });
        }
        // Ghost note overlay: built and uploaded independently.
        // Always upload (even when empty) to clear stale ghost data from the previous frame.
        let ghost_idx = job.decor_layers.len();
        pianoroll.upload_note_layer(ghost_idx, 0, |out| {
            for &(start_tick, end_tick, key, track) in &ghost_notes {
                yinhe_wgpu::build_ghost_note(out, start_tick, end_tick, key, track, &theme);
            }
        });
    } else if let Some(rt) = render_thread {
        // Async path (no cull): build instances on this thread, send to render thread
        let job = yinhe_wgpu::build_render_job(
            w, h, midi, view, &*selected,
            track_colors, scroll_mode, min_border_width,
            note_outline, &theme,
        );
        // Build note instances + ghost overlay as note layers for the render thread.
        let mut notes_instances = Vec::new();
        if let Some(midi) = midi {
            yinhe_wgpu::build_notes(&mut notes_instances, w as f32, h as f32, midi, view, &hidden_notes, track_visible);
        }
        let mut ghost_instances = Vec::new();
        for &(start_tick, end_tick, key, track) in &ghost_notes {
            yinhe_wgpu::build_ghost_note(&mut ghost_instances, start_tick, end_tick, key, track, &theme);
        }
        let note_layers = vec![
            yinhe_wgpu::NoteLayerData { instances: notes_instances, cache_key: 0, force: false },
            yinhe_wgpu::NoteLayerData { instances: ghost_instances, cache_key: 0, force: true },
        ];
        rt.send_job(yinhe_wgpu::RenderJob {
            width: job.width,
            height: job.height,
            uniforms: job.uniforms,
            track_colors: job.track_colors,
            selection: job.selection,
            decor_layers: job.decor_layers,
            note_layers,
        });
    }

    let t_prepare_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Static cache was removed — every frame rebuilds + uploads, so always paint.
    view.base.dirty = false;

    // Paint wgpu content into the content_rect
    if cull_ready {
        // GPU cull path: draw directly (no render thread needed — cull makes GPU work fast)
        render_ctx.paint(pianoroll, pw, ph, "pianoroll_frame", &painter, content_rect, true);
    } else {
        // Render thread handles GPU work — just display the latest texture
        render_ctx.paint_texture_only(pw, ph, &painter, content_rect);
    }

    let t_paint_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Keyboard (drawn by egui on top of the wgpu texture) ──
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;
    let _ppu = view.base.pixels_per_tick;
    let h_f32 = h as f32;
    let bottom = 128.0 * kh - scroll_y;
    let theme = pianoroll.theme();

    // White keys
    for key in 0u8..128 {
        if yinhe_types::is_black_key(key) {
            continue;
        }
        let y = bottom - (key as f32 + 1.0) * kh;
        if y + kh < 0.0 || y > h_f32 {
            continue;
        }
        let screen_y = content_rect.min.y + y;
        let (r, g, b) = theme.pr_white_key;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(content_rect.min.x, screen_y),
                egui::vec2(kb_w, kh),
            ),
            2.0,
            egui::Color32::from_rgb(
                (r * 255.0) as u8,
                (g * 255.0) as u8,
                (b * 255.0) as u8,
            ),
        );
    }

    // Black keys on top
    for key in 0u8..128 {
        if !yinhe_types::is_black_key(key) {
            continue;
        }
        let y = bottom - (key as f32 + 1.0) * kh;
        if y + kh < 0.0 || y > h_f32 {
            continue;
        }
        let screen_y = content_rect.min.y + y;
        let (r, g, b) = theme.pr_black_key;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(content_rect.min.x, screen_y),
                egui::vec2(kb_w, kh),
            ),
            1.5,
            egui::Color32::from_rgb(
                (r * 255.0) as u8,
                (g * 255.0) as u8,
                (b * 255.0) as u8,
            ),
        );
    }


    // ── Playback cursor (drawn by egui on top of the wgpu texture) ──
    // Decoupled from the wgpu pipeline so cursor movement during playback
    // does NOT invalidate the static instance cache.
    if let Some(ct) = *cursor_tick {
        let kb_w = view.keyboard_width();
        let cx_local = view.tick_to_x(ct);
        if cx_local >= kb_w && cx_local <= w as f32 {
            let cx = content_rect.min.x + cx_local;
            painter.line_segment(
                [
                    egui::pos2(cx, content_rect.min.y),
                    egui::pos2(cx, content_rect.max.y),
                ],
                egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::CURSOR_COLOR),
            );
        }
    }

    // ── Draw selection box on TOP of GPU content ──
    // State was already updated by sel_drag_frame above; this just draws the box
    // after the GPU paint so it's not covered by the texture.
    if *active_tool == Tool::Select {
        // Apply pending sel_rect delta from duplicate/transpose
        sel_rect.apply_pending();

        // Draw active drag box (if any)
        drag::draw_marquee_box(ui, content_rect, music_rect, view, quantize, ppq, bar_line_data,
            "sel_drag", egui::Color32::WHITE, egui::Color32::WHITE);

        // Draw persisted selection rect (remains after mouse release).
        // Compute pixel rect from music coordinates each frame so it follows
        // scroll/zoom.
        let eff = sel_rect.effective();
        let persisted_pixel_rect = eff.map(|(t_start, t_end, key_lo, key_hi)| {
            crate::selection::drag::music_sel_to_pixel_rect(
                &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
            )
        });
        if let Some(rect) = persisted_pixel_rect {
            // Only draw if at least partially visible
            let kb_w = music_rect.min.x - content_rect.min.x;
            let music_rect_local = egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(music_rect.width(), music_rect.height()),
            );
            let shifted = egui::Rect::from_min_max(
                egui::pos2(rect.min.x - kb_w, rect.min.y),
                egui::pos2(rect.max.x - kb_w, rect.max.y),
            );
            if shifted.intersects(music_rect_local) {
                crate::selection::draw::draw(&ui.painter(), music_rect, shifted, egui::Color32::WHITE, egui::Color32::WHITE);
            }
        }

        // Show floating action bar next to the persisted selection rect
        if let Some(action) =
            crate::widgets::selection_actions::show(ui, music_rect, persisted_pixel_rect)
        {
            sel_action = Some(action);
        }
    } else if *active_tool == Tool::Eraser {
        // Draw eraser marquee box in red
        drag::draw_marquee_box(ui, content_rect, music_rect, view, quantize, ppq, bar_line_data,
            "eraser_drag", egui::Color32::RED, egui::Color32::RED);
    }

    // ── Time ruler ──
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + view.keyboard_width(), ruler_band_y),
            egui::pos2(rect.max.x, ruler_band_y + RULER_H),
        );
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let ruler_jumped = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            view,
            tpb,
            def_num,
            def_den,
            sig_events,
            |tick| crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data),
            "piano_ruler",
            cursor_tick,
        );
        // 点击/拖动时间标尺跳转位置时，取消已选择的选框（含框选与全选）。
        if ruler_jumped {
            selected.clear();
            sel_rect.rect = None;
        }
    }

    // ── Automation panels ──
    let panels_y = content_rect.max.y;
    if let (Some(panels), Some(renderers), Some(lanes), Some(show), Some(wgpu_state)) = (
        auto_panels,
        auto_renderers,
        auto_lanes,
        auto_show,
        auto_wgpu_state,
    ) {
        let kb_w = view.keyboard_width();
        let combo_w = kb_w * theme::AUTO_PANEL_COMBO_WIDTH_RATIO;

        // automation 编辑上下文：Pencil/Curve 工具时启用。
        // active_track 与 automation_lanes 用同样的逻辑：第一个选中 track（排除 conductor），
        // 没有选中时用 track 0。不要求"唯一选中"。
        let active_track = track_selected
            .iter()
            .next()
            .copied()
            .filter(|&t| Some(t) != conductor_idx)
            .or(Some(0));
        let edit_ctx = if *active_tool == Tool::Pencil || *active_tool == Tool::Curve {
            Some(automation_panel::AutomationEditCtx {
                active_tool: *active_tool,
                active_track,
                quantize,
                ppq,
                bar_line_data,
            })
        } else {
            None
        };

        let (_h, auto_edits, auto_feedback, auto_drag_info) = automation_panel::show_panels(
            ui,
            panels,
            renderers,
            lanes,
            show,
            wgpu_state,
            combo_w,
            view.base.scroll_x,
            view.base.pixels_per_tick,
            rect.max.x,
            panels_y,
            panels_total_h,
            midi.and_then(|m| m.ticks_per_beat()),
            bar_line_data.map(|b| b.1).unwrap_or(4),
            bar_line_data.map(|b| b.2).unwrap_or(4),
            bar_line_data.map(|b| b.3).unwrap_or(&[]),
            track_visible,
            track_colors,
            scroll_mode,
            min_border_width,
            midi,
            velocity_display_mode,
            edit_ctx.as_ref(),
            tempo_events,
            revision,
            info_content,
            right_tab,
        );
        for edit in auto_edits {
            auto_edit_events.push(edit);
        }

        // 应用 automation 面板的 pianoroll 联动反馈（水平滚动/缩放）
        if auto_feedback.scroll_x_delta != 0.0 {
            view.base.scroll_x -= auto_feedback.scroll_x_delta;
            view.base.dirty = true;
        }
        if (auto_feedback.zoom_factor - 1.0).abs() > 0.001 {
            view.zoom_around_x(auto_feedback.zoom_center_x, auto_feedback.zoom_factor);
        }

        // 存储 ghost drag info 供信息面板实时显示
        *automation_drag_ghost = auto_drag_info;

        if midi.is_some() {
            let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
            let sb_left_blank = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, sb_y),
                egui::pos2(
                    rect.min.x + kb_w,
                    sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
                ),
            );
            ui.painter()
                .rect_filled(sb_left_blank, 0.0, theme::SCROLLBAR_BG);
            ui.scope_builder(egui::UiBuilder::new().max_rect(sb_left_blank), |ui| {
                ui.horizontal_centered(|ui| {
                    let mut count = panels.len();
                    automation_panel::show_toggle_buttons(ui, show, &mut count);
                    while panels.len() < count {
                        panels.push(yinhe_types::AutomationPanelView::default());
                    }
                    while panels.len() > count {
                        panels.pop();
                    }
                });
            });
        }
    }

    // ── Horizontal scrollbar ──
    let kb_w = view.keyboard_width();
    let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
    let sb_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + kb_w, sb_y),
        egui::pos2(rect.max.x, sb_y + crate::widgets::scrollbar::SCROLLBAR_H),
    );

    ui.push_id("piano_scrollbar", |ui| {
        crate::widgets::scrollbar::show(
            ui,
            sb_rect,
            w as f32 - kb_w,
            &mut view.base.scroll_x,
            &mut view.base.pixels_per_tick,
            total_ticks,
            &mut view.base.dirty,
        );
    });

    // ── Perf probe: submit per-frame sample ──
    if let (Some(t0), Some(t1), Some(t2), Some(t3)) =
        (t_show_start, t_input_end, t_prepare_end, t_paint_end)
    {
        let t_end = std::time::Instant::now();
        let input = t1.saturating_duration_since(t0);
        let prepare_total = t2.saturating_duration_since(t1);
        let paint = t3.saturating_duration_since(t2);
        let misc = t_end.saturating_duration_since(t3);
        yinhe_memtrace::perf_probe::submit(yinhe_memtrace::perf_probe::FrameSample {
            input,
            prep_static: prepare_total,
            paint,
            misc,
            instance_count: 0,
            follow_mode: match follow_mode {
                super::view_interaction::FollowMode::None => "None",
                super::view_interaction::FollowMode::Page => "Page",
                super::view_interaction::FollowMode::Continuous => "Continuous",
            },
            total_notes: midi
                .map(|m| {
                    let mut sum = 0u64;
                    for k in 0..128u8 {
                        sum += m.key_notes(k).len() as u64;
                    }
                    sum
                })
                .unwrap_or(0),
            ppt: view.base.pixels_per_tick,
            visible_ticks: {
                let (s, e) = view.visible_tick_range(w as f32);
                e - s
            },
        });
    }

    // ── PR quantize button in the top-left corner (left of ruler, above keyboard) ──
    let mut pr_quantize_event: Option<PianoViewEvent> = None;
    {
        let corner_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, ruler_band_y),
            egui::vec2(kb_w, RULER_H),
        );
        let btn_size = 20.0;
        let btn_rect = egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
        let btn_resp = ui.interact(btn_rect, egui::Id::new("pr_quantize_btn"), egui::Sense::click());
        let hovered = btn_resp.hovered();

        let icon_color = if hovered {
            crate::theme::ACCENT_ACTIVE
        } else {
            egui::Color32::from_gray(160)
        };
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            quantize.label(),
            egui::FontId::proportional(11.0),
            icon_color,
        );

        let mut pending_q = None;
        egui::Popup::from_toggle_button_response(&btn_resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                crate::widgets::quantize_popup::show(
                    ui,
                    ppq,
                    quantize,
                    &mut pending_q,
                );
            });
        if let Some(new_preset) = pending_q {
            pr_quantize_event = Some(PianoViewEvent::QuantizePreset(new_preset));
        }
    }

    sel_action
        .map(PianoViewEvent::SelectionAction)
        .or(pencil_event)
        .or(eraser_event)
        .or(pr_quantize_event)
}
