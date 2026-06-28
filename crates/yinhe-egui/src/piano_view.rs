use std::sync::Arc;

use eframe::egui;

use yinhe_types::{AutomationLane, TimeSigEvent};

use yinhe_editor_core::quantize::QuantizePreset;
use crate::widgets::tools_panel::Tool;
use crate::widgets::selection_actions::SelectionAction;

mod automation_panel;

/// Events emitted by the piano-roll view for the caller to act on.
pub enum PianoViewEvent {
    SelectionAction(SelectionAction),
    AddNote { track: u16, note: yinhe_core::NoteEvent },
}

/// Pencil-tool drag output for modifying existing notes.
/// `None` means the drag has ended; `Some(...)` means a drag is in progress.
pub enum PencilNoteDrag {
    /// Moving a single note by (delta_ticks, delta_keys) from its original position.
    Move { track: u16, start_tick: u32, key: u8, delta_ticks: i64, delta_keys: i32 },
    /// Resizing the right edge of a note.
    ResizeRight { track: u16, start_tick: u32, key: u8, new_end_tick: u32 },
    /// Resizing the left edge of a note.
    ResizeLeft { track: u16, start_tick: u32, key: u8, new_start_tick: u32 },
}

/// Internal pencil-tool drag mode persisted across frames.
#[derive(Clone)]
enum PencilDrag {
    /// Creating a new note: (start_tick, key)
    Create(f64, u8),
    /// Moving an existing note: (track, original_start_tick, original_key)
    Move(u16, u32, u8),
    /// Resizing right edge: (track, start_tick, end_tick, key)
    ResizeRight(u16, u32, u32, u8),
    /// Resizing left edge: (track, start_tick, end_tick, key)
    ResizeLeft(u16, u32, u32, u8),
}

/// Result of hit-testing the cursor against existing notes.
struct HitNote {
    track: u16,
    start_tick: u32,
    end_tick: u32,
    key: u8,
    mode: HitMode,
}

#[derive(Clone)]
enum HitMode {
    Move,
    ResizeLeft,
    ResizeRight,
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
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    midi_version: u64,
    haptic_engine: Option<&yinhe_haptic::HapticEngine>,
    pencil_note_drag: &mut Option<PencilNoteDrag>,
) -> Option<PianoViewEvent> {
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
    let perf_on = yinhe_memtrace::perf_probe::enabled();
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
    let mut pencil_event: Option<PianoViewEvent> = None;
    let mut ghost_note = None;
    if *active_tool == Tool::Select {
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
            sel_rect,
        );
    } else if *active_tool == Tool::Pencil {
        let (note_event, ghost, pencil_drag) = pencil_frame(
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
        );
        ghost_note = ghost;
        *pencil_note_drag = pencil_drag;
        if let Some(note) = note_event {
            if let Some(track) = valid_pencil_track(track_selected, conductor_idx) {
                pencil_event = Some(PianoViewEvent::AddNote { track, note });
            }
        }
    }

    // ── Hover cursor: show Move when over selection rect ──
    if *active_tool == Tool::Select {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if music_rect.contains(pos) {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                let eff = sel_rect.effective();
                let in_sel_rect = eff.is_some_and(|(t_start, t_end, key_lo, key_hi)| {
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
    let total_ticks = midi
        .map(|m| m.tick_length().unwrap_or(0) as f64)
        .unwrap_or(0.0);
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

    // Prepare GPU data
    let prep_timings = yinhe_pianoroll::prepare(
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
        ghost_note,
    );

    let t_prepare_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Static cache was removed — every frame rebuilds + uploads, so always paint.
    view.base.dirty = false;
    let content_changed = true;

    // Paint wgpu content into the content_rect
    render_ctx.paint(
        pianoroll,
        w,
        h,
        "pianoroll_frame",
        &painter,
        content_rect,
        content_changed,
    );

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
    if *active_tool == Tool::Select {
        // Apply pending sel_rect delta from duplicate/transpose
        sel_rect.apply_pending();

        // Draw active drag box (if any)
        sel_draw_box(ui, content_rect, music_rect, view, quantize, ppq, bar_line_data);

        // Draw persisted selection rect (remains after mouse release).
        // Compute pixel rect from music coordinates each frame so it follows
        // scroll/zoom.
        let eff = sel_rect.effective();
        let persisted_pixel_rect = eff.map(|(t_start, t_end, key_lo, key_hi)| {
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
        crate::widgets::time_ruler::interactive_ruler(
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
        yinhe_memtrace::perf_probe::submit(yinhe_memtrace::perf_probe::FrameSample {
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

    sel_action.map(PianoViewEvent::SelectionAction).or(pencil_event)
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
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
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
        sel_rect.cancel_drag();
    }

    // Start drag
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let eff = sel_rect.effective();
        let on_bar = eff.is_some_and(|(t_start, t_end, key_lo, key_hi)| {
            let pixel_rect = crate::view_interaction::music_sel_to_pixel_rect(
                &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
            );
            crate::widgets::selection_actions::compute_bar_rect(music_rect, pixel_rect)
                .is_some_and(|bar| bar.contains(pos))
        });

        if on_bar {
            // Don't start drag, don't clear anything — let the button handle it.
        } else {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let in_sel_rect = eff.is_some_and(|(t_start, t_end, key_lo, key_hi)| {
                let pixel_rect = crate::view_interaction::music_sel_to_pixel_rect(
                    &view.base, view.key_height, t_start, t_end, key_lo, key_hi,
                );
                pixel_rect.contains(local)
            });
            if in_sel_rect {
                let raw_tick = view.x_to_tick(local.x);
                let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let key = view.y_to_key(local.y) as f64;
                note_drag_origin = Some((tick, key));
                sel_rect.start_drag();
            } else {
                // Not on selection rect → marquee selection
                drag = Some((local, local));
                if !cmd {
                    selected.clear();
                }
                sel_rect.rect = None;
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
                sel_rect.update_drag(dt, dk);
                ui.ctx().request_repaint();
            }
        }
        if pointer.primary_released() {
            if let Some(pos) = pointer.hover_pos() {
                let local_x = pos.x - content_rect.min.x;
                let local_y = pos.y - content_rect.min.y;
                let raw_tick = view.x_to_tick(local_x);
                let snapped_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
                let current_key = view.y_to_key(local_y) as f64;
                let dt = (snapped_tick - origin_tick).round() as i64;
                let dk = (current_key - origin_key).round() as i32;
                *note_drag_delta = Some((dt, dk));
                sel_rect.update_drag(dt, dk);
            }
            sel_rect.end_drag();
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
            if let (Some(midi_ref), Some((start, end))) = (midi, drag) {
                let drag_dist = (end - start).length();

                if drag_dist < 3.0 {
                    let tick = view.x_to_tick(start.x);
                    let snapped = crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data);
                    selected.clear();
                    *cursor_tick = Some(snapped.max(0.0));
                    sel_rect.rect = None;
                } else {
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

                    sel_rect.rect = Some((t_start, t_end, key_lo, key_hi));
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
/// Draws the active marquee selection box on top of GPU content.
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

/// Returns the single valid target track for the Pencil tool, if any.
fn valid_pencil_track(
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
) -> Option<u16> {
    if track_selected.len() != 1 {
        return None;
    }
    let &track = track_selected.iter().next()?;
    if Some(track) == conductor_idx {
        return None;
    }
    Some(track)
}

/// Pencil-tool input handling: hover preview, click to write a note, drag to lengthen,
/// or hover over / drag existing notes to move or resize them.
/// Returns `(note_event, ghost_note, pencil_note_drag)`.
fn pencil_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_pianoroll::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
) -> (Option<yinhe_core::NoteEvent>, Option<(f64, f64, u8)>, Option<PencilNoteDrag>) {
    let pencil_id = ui.id().with("pencil_drag");
    let drag_state: Option<PencilDrag> =
        ui.data_mut(|d| d.get_persisted(pencil_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state.
    if drag_state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
    }

    let hover_pos = pointer.hover_pos();
    let can_write = valid_pencil_track(track_selected, conductor_idx).is_some();
    let _track = valid_pencil_track(track_selected, conductor_idx);

    // Hover / drag preview.
    let preview = if let Some(pos) = hover_pos {
        if music_rect.contains(pos) {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let raw_tick = view.x_to_tick(local.x);
            let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
            let key = view.y_to_key(local.y);
            Some((tick.max(0.0), key))
        } else {
            None
        }
    } else {
        None
    };

    // ── Hit-test existing notes (only when not dragging) ──
    // Returns the closest note under cursor with its hit mode.
    const EDGE_THRESHOLD_PX: f32 = 6.0;
    let kb_w = music_rect.min.x - content_rect.min.x;

    let hit_note = if drag_state.is_none() && can_write {
        preview.and_then(|(_tick, key)| {
            let midi = midi?;
            let notes = midi.key_notes_in_range(key, 0, u32::MAX);
            let mouse_screen = hover_pos?;
            // Compute mouse position in local (music-area) coordinates
            let mouse_local_x = mouse_screen.x - music_rect.min.x;
            let mouse_local_y = mouse_screen.y - music_rect.min.y;

            // Find the note whose pixel rect contains the mouse
            for note in notes {
                let note_left = view.tick_to_x(note.start_tick as f64) - kb_w;
                let note_right = view.tick_to_x(note.end_tick as f64) - kb_w;
                let note_top = view.key_to_y(key);
                let note_bottom = note_top + view.key_height;

                if mouse_local_x >= note_left && mouse_local_x <= note_right
                    && mouse_local_y >= note_top && mouse_local_y <= note_bottom
                {
                    let dist_left = (mouse_local_x - note_left).abs();
                    let dist_right = (mouse_local_x - note_right).abs();
                    let mode = if dist_left < EDGE_THRESHOLD_PX {
                        HitMode::ResizeLeft
                    } else if dist_right < EDGE_THRESHOLD_PX {
                        HitMode::ResizeRight
                    } else {
                        HitMode::Move
                    };
                    return Some(HitNote {
                        track: note.track,
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        key,
                        mode,
                    });
                }
            }
            None
        })
    } else {
        None
    };

    // ── Set cursor based on hit test ──
    if let Some(ref hit) = hit_note {
        match hit.mode {
            HitMode::ResizeLeft => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest),
            HitMode::ResizeRight => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast),
            HitMode::Move => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
        }
    }

    // ── Ghost note: only when not over an existing note ──
    let ghost_note = if can_write && drag_state.is_none() && hit_note.is_none() {
        if let Some((tick, key)) = preview {
            let interval = quantize.tick_interval(ppq) as f64;
            // Not dragging (drag_state is None due to the outer condition),
            // show preview at hover position
            Some((tick, tick + interval, key))
        } else {
            None
        }
    } else {
        None
    };

    // ── Start drag ──
    if pointer.primary_pressed() {
        if let Some(hit) = hit_note {
            let new_drag = match hit.mode {
                HitMode::ResizeLeft => PencilDrag::ResizeLeft(hit.track, hit.start_tick, hit.end_tick, hit.key),
                HitMode::ResizeRight => PencilDrag::ResizeRight(hit.track, hit.start_tick, hit.end_tick, hit.key),
                HitMode::Move => PencilDrag::Move(hit.track, hit.start_tick, hit.key),
            };
            ui.data_mut(|d| d.insert_persisted(pencil_id, Some(new_drag)));
        } else if let Some((tick, key)) = preview {
            ui.data_mut(|d| d.insert_persisted(pencil_id, Some(PencilDrag::Create(tick, key))));
        }
    }

    // ── Compute drag output ──
    let mut result = None;
    let mut pencil_note_drag = None;

    match &drag_state {
        Some(PencilDrag::Create(s_tick, s_key)) => {
            // Release -> commit note.
            if pointer.primary_released() {
                if can_write {
                    let interval = quantize.tick_interval(ppq) as f64;
                    let end_tick = if let Some((tick, _)) = preview {
                        let current_end = tick.max(*s_tick + interval);
                        let snapped_end = crate::view_interaction::snap_tick_ceil(
                            current_end,
                            quantize,
                            ppq,
                            bar_line_data,
                        );
                        snapped_end.max(*s_tick + interval)
                    } else {
                        *s_tick + interval
                    };
                    result = Some(yinhe_core::NoteEvent {
                        start_tick: *s_tick as u32,
                        end_tick: end_tick as u32,
                        key: *s_key,
                        velocity: 100,
                        dup_index: 0,
                    });
                }
                ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
            }
        }
        Some(PencilDrag::Move(trk, orig_tick, orig_key)) => {
            if let Some((tick, key)) = preview {
                let dt = (tick as i64) - (*orig_tick as i64);
                let dk = (key as i32) - (*orig_key as i32);
                pencil_note_drag = Some(PencilNoteDrag::Move {
                    track: *trk,
                    start_tick: *orig_tick,
                    key: *orig_key,
                    delta_ticks: dt,
                    delta_keys: dk,
                });
            }
            if pointer.primary_released() {
                ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
            }
        }
        Some(PencilDrag::ResizeRight(trk, orig_tick, _orig_end, orig_key)) => {
            if let Some((tick, _)) = preview {
                let interval = quantize.tick_interval(ppq) as f64;
                let snapped = crate::view_interaction::snap_tick_ceil(
                    tick.max(*orig_tick as f64 + interval),
                    quantize,
                    ppq,
                    bar_line_data,
                );
                let new_end = snapped.max(*orig_tick as f64 + interval).min(u32::MAX as f64) as u32;
                pencil_note_drag = Some(PencilNoteDrag::ResizeRight {
                    track: *trk,
                    start_tick: *orig_tick,
                    key: *orig_key,
                    new_end_tick: new_end,
                });
            }
            if pointer.primary_released() {
                ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
            }
        }
        Some(PencilDrag::ResizeLeft(trk, orig_tick, orig_end, orig_key)) => {
            if let Some((tick, _)) = preview {
                let interval = quantize.tick_interval(ppq) as f64;
                let snapped = crate::view_interaction::snap_tick_floor(
                    tick,
                    quantize,
                    ppq,
                    bar_line_data,
                );
                let new_start = (snapped as u32).min(*orig_end - 1);
                // Clamp so the note is at least one quantize interval wide
                let min_start = (*orig_end as f64 - interval).max(0.0) as u32;
                let new_start = new_start.max(min_start);
                pencil_note_drag = Some(PencilNoteDrag::ResizeLeft {
                    track: *trk,
                    start_tick: *orig_tick,
                    key: *orig_key,
                    new_start_tick: new_start,
                });
            }
            if pointer.primary_released() {
                ui.data_mut(|d| d.insert_persisted(pencil_id, Option::<PencilDrag>::None));
            }
        }
        None => {}
    }

    // Persist drag state (updated for Create, cleared for others on release)
    // Note: PencilDrag variants other than Create are stored directly via
    // insert_persisted above on press, and cleared on release.
    // For Create, the state is managed via the original `start` variable.
    // We only need to persist if the drag_state was None and we didn't handle it above.
    // Actually, the drag_state is already managed via insert_persisted in press/release handlers.
    // The `start` variable from the old code is no longer needed since we use PencilDrag::Create.

    (result, ghost_note, pencil_note_drag)
}

