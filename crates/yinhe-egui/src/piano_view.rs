use std::sync::Arc;

use eframe::egui;

use yinhe_types::{AutomationLane, TimeSigEvent};

use yinhe_editor_core::quantize::QuantizePreset;
use crate::widgets::tools_panel::Tool;

mod automation_panel;

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
/// Returns an optional `SelectionAction` if the user clicked a floating action button.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &mut std::collections::HashSet<(u16, u32, u8)>,
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
    auto_panels: Option<&mut Vec<yinhe_automation::AutomationPanelView>>,
    auto_renderers: Option<
        &mut Vec<(
            yinhe_automation::PianorollRenderer,
            super::render_context::RenderContext,
        )>,
    >,
    auto_lanes: Option<&[AutomationLane]>,
    auto_show: Option<&mut bool>,
    auto_wgpu_state: Option<&Arc<eframe::egui_wgpu::RenderState>>,
    scroll_mode: u32,
    min_border_width: f32,
    velocity_display_mode: &mut u32,
    automation_display_mode: &mut u32,
    automation_show_dots: &mut bool,
    note_drag_delta: &mut Option<(i64, i32)>,
    midi_version: u64,
) -> Option<crate::widgets::selection_actions::SelectionAction> {
    // Sense::hover() — no drag ownership. All drag is handled by dedicated
    // ui.interact calls below, each inside its own push_id scope.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;

    // Compute automation panel total height.
    // First panel has no leading handle; subsequent panels have SPLIT_H above them.
    let panels_total_h: f32 = match (&auto_panels, &auto_show) {
        (Some(panels), Some(show)) if **show && !panels.is_empty() => {
            panels.iter().map(|p| p.panel_height).sum::<f32>()
                + (panels.len() as f32 * automation_panel::SPLIT_H)
        }
        _ => 0.0,
    };

    // Layout: ruler | pianoroll content | automation panels | scrollbar
    let ruler_band_y = rect.min.y;
    let content_y = rect.min.y + RULER_H;
    let content_h =
        (rect.height() - RULER_H - panels_total_h - crate::widgets::scrollbar::SCROLLBAR_H)
            .max(0.0);
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, content_y),
        egui::pos2(rect.max.x, content_y + content_h),
    );
    let kb_w = view.keyboard_width();
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + kb_w, content_y),
        egui::pos2(rect.max.x, content_y + content_h),
    );
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;

    if w == 0 || h == 0 {
        return None;
    }

    // ── Perf probe (only when YIN_PERF=1) ──
    let perf_on = crate::perf_probe::enabled();
    let t_show_start = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(w, h);

    // Clamp scroll — add some extra space beyond the last note
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
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
    if *active_tool == Tool::Select && !is_playing {
        sel_drag_frame(
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
        );
    }

    // ── Hover cursor: show Move when over selection rect ──
    if *active_tool == Tool::Select && !is_playing {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if music_rect.contains(pos) {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                let persist_id = ui.id().with("sel_rect_persist");
                let sel_music: Option<Option<(f64, f64, u8, u8)>> =
                    ui.data_mut(|d| d.get_persisted(persist_id));
                let in_sel_rect = sel_music.flatten().is_some_and(|(t_start, t_end, key_lo, key_hi)| {
                    let pixel_rect = crate::view_interaction::music_sel_to_pixel_rect(
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
    let total_ticks = midi
        .map(|m| m.tick_length().unwrap_or(0) as f64)
        .unwrap_or(0.0);
    view.clamp_scroll(w as f32, h as f32, total_ticks);

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

    // Prepare GPU data
    let prep_timings = crate::util::qos::guarded(|| {
        yinhe_pianoroll::prepare(
            pianoroll,
            w,
            h,
            midi,
            view,
            &*selected,
            track_visible,
            track_colors,
            scroll_mode,
            min_border_width,
            midi_version,
        )
    });

    let t_prepare_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Static cache was removed — every frame rebuilds + uploads, so always paint.
    view.base.dirty = false;
    let content_changed = true;

    // Paint wgpu content into the content_rect
    crate::util::qos::guarded(|| {
        render_ctx.paint(
            pianoroll,
            w,
            h,
            "pianoroll_frame",
            &painter,
            content_rect,
            content_changed,
        );
    });

    let t_paint_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

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
    if *active_tool == Tool::Select && !is_playing {
        // Draw active drag box (if any)
        sel_draw_box(ui, content_rect, music_rect, view, quantize, ppq, bar_line_data);

        // Draw persisted selection rect (remains after mouse release).
        // Compute pixel rect from music coordinates each frame so it follows
        // scroll/zoom.
        let persist_id = ui.id().with("sel_rect_persist");
        let sel_music: Option<Option<(f64, f64, u8, u8)>> =
            ui.data_mut(|d| d.get_persisted(persist_id));
        let persisted_pixel_rect = sel_music.flatten().map(|(t_start, t_end, key_lo, key_hi)| {
            crate::view_interaction::music_sel_to_pixel_rect(
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
                crate::widgets::selection_box::draw(&ui.painter(), music_rect, shifted);
            }
        }

        // Show floating action bar next to the persisted selection rect
        if let Some(action) =
            crate::widgets::selection_actions::show(ui, music_rect, persisted_pixel_rect)
        {
            sel_action = Some(action);
        }
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
        crate::widgets::time_ruler::paint(
            &painter, ruler_rect, view, tpb, def_num, def_den, sig_events,
        );
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

        automation_panel::show_panels(
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
            automation_display_mode,
            automation_show_dots,
        );

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
                        panels.push(yinhe_automation::AutomationPanelView::default());
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
        // prep_static + prep_cursor + upload should ≈ prepare_total. The
        // residual (closure dispatch, hashing, etc.) goes into misc by
        // omitting it from this sample's prep_* fields.
        let known = prep_timings.build_static + prep_timings.build_cursor + prep_timings.upload;
        let prepare_overhead = prepare_total.saturating_sub(known);
        let follow_name = match follow_mode {
            super::view_interaction::FollowMode::None => "None",
            super::view_interaction::FollowMode::Page => "Page",
            super::view_interaction::FollowMode::Continuous => "Continuous",
        };
        crate::perf_probe::submit(crate::perf_probe::FrameSample {
            input,
            prep_static: prep_timings.build_static,
            prep_cursor: prep_timings.build_cursor,
            upload: prep_timings.upload,
            paint,
            misc: misc + prepare_overhead,
            static_rebuilt: prep_timings.static_rebuilt,
            instance_count: prep_timings.instance_count,
            follow_mode: follow_name,
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

    sel_action
}

// ── Selection drag logic ──

fn sel_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &mut std::collections::HashSet<(u16, u32, u8)>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    cursor_tick: &mut Option<f64>,
    note_drag_delta: &mut Option<(i64, i32)>,
) {
    let sel_id = ui.id().with("sel_drag");
    let mut drag: Option<(egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let note_drag_id = ui.id().with("note_drag_origin");
    let mut note_drag_origin: Option<(f64, f64)> =
        ui.data_mut(|d| d.get_persisted(note_drag_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

    // Clear stale drag state
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }
    if note_drag_origin.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        note_drag_origin = None;
    }

    // Start drag
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        // Check if click is on the floating action bar — if so, skip.
        let on_bar = {
            let persist_id = ui.id().with("sel_rect_persist");
            let sel_music: Option<Option<(f64, f64, u8, u8)>> =
                ui.data_mut(|d| d.get_persisted(persist_id));
            sel_music.flatten().is_some_and(|(t_start, t_end, key_lo, key_hi)| {
                let pixel_rect = crate::view_interaction::music_sel_to_pixel_rect(
                    &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
                );
                crate::widgets::selection_actions::compute_bar_rect(music_rect, pixel_rect)
                    .is_some_and(|bar| bar.contains(pos))
            })
        };

        if on_bar {
            // Don't start drag, don't clear anything — let the button handle it.
        } else {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            // Check if clicking inside the selection rect → note-drag mode
            let in_sel_rect = {
                let persist_id = ui.id().with("sel_rect_persist");
                let sel_music: Option<Option<(f64, f64, u8, u8)>> =
                    ui.data_mut(|d| d.get_persisted(persist_id));
                sel_music.flatten().is_some_and(|(t_start, t_end, key_lo, key_hi)| {
                    let pixel_rect = crate::view_interaction::music_sel_to_pixel_rect(
                        &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
                    );
                    pixel_rect.contains(local)
                })
            };
            if in_sel_rect {
                let raw_tick = view.x_to_tick(local.x);
                let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let key = view.y_to_key(local.y) as f64;
                note_drag_origin = Some((tick, key));
                // Don't clear sel_rect_persist — keep the selection box visible
            } else {
                // Not on selection rect → marquee selection
                drag = Some((local, local));
                if !cmd {
                    selected.clear();
                }
                let persist_id = ui.id().with("sel_rect_persist");
                ui.data_mut(|d| d.insert_persisted(persist_id, Option::<(f64, f64, u8, u8)>::None));
            }
        }
    }

    // Note drag: update delta each frame
    if let Some((origin_tick, origin_key)) = note_drag_origin {
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let local_x = pos.x - content_rect.min.x;
                let local_y = pos.y - content_rect.min.y;
                let raw_tick = view.x_to_tick(local_x);
                let snapped_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let current_key = view.y_to_key(local_y) as f64;
                let dt = (snapped_tick - origin_tick).round() as i64;
                let dk = (current_key - origin_key).round() as i32;
                *note_drag_delta = Some((dt, dk));
                ui.ctx().request_repaint();
            }
        }
        if pointer.primary_released() {
            // Set final delta before clearing origin
            if let Some(pos) = pointer.hover_pos() {
                let local_x = pos.x - content_rect.min.x;
                let local_y = pos.y - content_rect.min.y;
                let raw_tick = view.x_to_tick(local_x);
                let snapped_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let current_key = view.y_to_key(local_y) as f64;
                let dt = (snapped_tick - origin_tick).round() as i64;
                let dk = (current_key - origin_key).round() as i32;
                *note_drag_delta = Some((dt, dk));
            }
            note_drag_origin = None;
        }
    }

    // Marquee drag: update end on frames after the initial press
    if let Some((start, _)) = drag {
        if pointer.primary_down() && !pointer.primary_pressed() {
            if let Some(pos) = pointer.hover_pos() {
                let clamped = pos.clamp(music_rect.min, music_rect.max);
                let local = egui::pos2(
                    clamped.x - content_rect.min.x,
                    clamped.y - content_rect.min.y,
                );
                drag = Some((start, local));

                // ── Auto-scroll when dragging near the edge ──
                let (actual_dx, actual_dy) = crate::view_interaction::auto_scroll_on_drag(
                    ui,
                    &mut view.base,
                    music_rect,
                    pos,
                    |base, w, h| {
                        base.clamp_scroll_x(w, total_ticks);
                        base.scroll_y = base.scroll_y.max(0.0);
                    },
                );
                view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);
                if actual_dx != 0.0 || actual_dy != 0.0 {
                    drag = drag.map(|(s, e)| (egui::pos2(s.x - actual_dx, s.y - actual_dy), e));
                }
            }
        }

        // Release -> hit test
        if pointer.primary_released() {
            let persist_id = ui.id().with("sel_rect_persist");
            if let (Some(midi_ref), Some((start, end))) = (midi, drag) {
                let drag_dist = (end - start).length();

                if drag_dist < 3.0 {
                    // Click (no meaningful drag) — set cursor, clear selection
                    let tick = view.x_to_tick(start.x);
                    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
                    selected.clear();
                    *cursor_tick = Some(snapped.max(0.0));
                    // Clear persisted selection rect on click
                    ui.data_mut(|d| d.insert_persisted(persist_id, Option::<(f64, f64, u8, u8)>::None));
                } else {
                    // Drag — existing marquee behavior
                    let (
                        _snapped_sx,
                        _snapped_ex,
                        _snapped_sy,
                        _snapped_ey,
                        t_start,
                        t_end,
                        key_lo,
                        key_hi,
                    ) = piano_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);

                    if !cmd {
                        selected.clear();
                    }
                    for key in key_lo..=key_hi {
                        for note in midi_ref.key_notes(key) {
                            if (note.start_tick as f64) < t_end && (note.end_tick as f64) > t_start {
                                selected.insert((note.track, note.start_tick, key));
                            }
                        }
                    }

                    // Persist music coordinates for the floating action bar
                    ui.data_mut(|d| d.insert_persisted(persist_id, Some((t_start, t_end, key_lo, key_hi))));
                }
                view.base.dirty = true;
            }
            drag = None;
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    ui.data_mut(|d| d.insert_persisted(note_drag_id, note_drag_origin));
}

/// Check if a local coordinate hits any selected note.
/// Read persisted drag state and draw the selection box on top of GPU content.
/// Must be called AFTER `render_ctx.paint` so the box is not covered by the texture.
fn sel_draw_box(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &yinhe_pianoroll::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) {
    let sel_id = ui.id().with("sel_drag");
    let drag: Option<(egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    if let Some((start, end)) = drag {
        if (end - start).length() < 3.0 {
            return;
        }
        let (vx, vy, vw, vh, _, _, _, _) =
            piano_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);
        let kb_w = music_rect.min.x - content_rect.min.x;
        let snapped = egui::Rect::from_min_max(
            egui::pos2(vx.min(vy) - kb_w, vw.min(vh)),
            egui::pos2(vx.max(vy) - kb_w, vw.max(vh)),
        );
        crate::widgets::selection_box::draw(&ui.painter(), music_rect, snapped);
    }
}

/// Compute snapped selection bounds for piano roll.
#[allow(clippy::too_many_arguments)]
fn piano_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &yinhe_pianoroll::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f32, f32, f32, f32, f64, f64, u8, u8) {
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

    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;

    let key_lo = (127.0 - ((scroll_y + ey) / kh)).ceil().max(0.0).min(127.0) as u8;
    let key_hi = (127.0 - ((scroll_y + sy) / kh)).ceil().max(0.0).min(127.0) as u8;
    let screen_sy = (127.0 - key_hi as f32) * kh - scroll_y;
    let screen_ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;

    let screen_sx = view.tick_to_x(t_start);
    let screen_ex = view.tick_to_x(t_end);

    (
        screen_sx, screen_ex, screen_sy, screen_ey, t_start, t_end, key_lo, key_hi,
    )
}


