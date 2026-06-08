use std::sync::Arc;

use eframe::egui;

use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::quantize::QuantizePreset;
use crate::widgets::tools_panel::Tool;

/// Height of the time ruler band at the top of the pianoroll view.
use crate::widgets::theme;
const RULER_H: f32 = theme::RULER_H;

/// Display the pianoroll texture with zoom/pan interaction.
///
/// When `auto_*` parameters are `Some`, automation panels are rendered between
/// the pianoroll content and the horizontal scrollbar. The AUTO toggle and
/// +/- buttons live inside the scrollbar's left blank area (same width as the
/// piano keyboard).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_pianoroll::PianorollRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &mut std::collections::HashSet<(u16, u32)>,
    track_visible: &[bool],
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
) {
    // Sense::hover() — no drag ownership. All drag is handled by dedicated
    // ui.interact calls below, each inside its own push_id scope.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;

    // Compute automation panel total height.
    // First panel has no leading handle; subsequent panels have SPLIT_H above them.
    let panels_total_h: f32 = match (&auto_panels, &auto_show) {
        (Some(panels), Some(show)) if **show && !panels.is_empty() => {
            panels.iter().map(|p| p.panel_height).sum::<f32>()
                + (panels.len() as f32 * crate::automation_panel::SPLIT_H)
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
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;

    if w == 0 || h == 0 {
        return;
    }

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

    // ── Content interaction (zoom/pan/cursor/drag/reset) ──
    // Created FIRST so that the keyboard handle (below) wins in the 4px
    // overlap zone where they intersect.
    crate::view_interaction::handle_input(
        ui,
        content_rect,
        view,
        cursor_tick,
        view.keyboard_width(),
        Some((quantize, ppq)),
        bar_line_data,
        None,
        is_playing,
        follow_mode,
        active_tool,
    );

    // ── Selection drag (Select tool only) ──
    if *active_tool == Tool::Select && !is_playing {
        sel_drag_frame(
            ui,
            content_rect,
            view,
            midi,
            selected,
            quantize,
            ppq,
            bar_line_data,
        );
    }

    // ── Keyboard resize handle ──
    // Created AFTER content interact so it wins the 4px overlap at the edge.
    // Covers ruler + content area, not the scrollbar below.
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
                crate::widgets::theme::MIN_KEYBOARD_WIDTH,
                rect.width() * crate::widgets::theme::MAX_KEYBOARD_RATIO,
            );

            // Keep scrollbar thumb visually in sync with the content area by
            // adjusting scroll_x so that the thumb's pixel offset within the
            // scrollbar track stays constant as the track width changes.
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
    // handle_input() and keyboard drag may have set scroll_x/scroll_y out of bounds.
    // Clamp before rendering to prevent 1-frame out-of-bounds visual.
    let total_ticks = midi
        .map(|m| m.tick_length().unwrap_or(0) as f64)
        .unwrap_or(0.0);
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // ── Dirty detection ──
    // Run AFTER all interactions so handle_input/keyboard changes are caught.
    if *cursor_tick != *last_cursor_tick {
        view.base.dirty = true;
    }
    *last_cursor_tick = *cursor_tick;

    let force_rebuild = view.base.dirty;

    // Prepare GPU data — uses the latest view state (keyboard_width, scroll, etc.)
    let gpu_dirty = crate::widgets::qos::guarded(|| {
        yinhe_pianoroll::prepare(
            pianoroll,
            w,
            h,
            midi,
            view,
            &*selected,
            track_visible,
            *cursor_tick,
            force_rebuild,
        )
    });

    let content_changed = view.base.dirty || gpu_dirty;
    view.base.dirty = false;

    // Paint wgpu content into the content_rect (below the ruler)
    crate::widgets::qos::guarded(|| {
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

    // ── Time ruler (top band, right of keyboard) ──
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

        crate::automation_panel::show_panels(
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
        );

        // AUTO +/- buttons (in the scrollbar's left blank area)
        // We render them here while we still have access to `panels`/`show`.
        if midi.is_some() {
            let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
            let sb_left_blank = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, sb_y),
                egui::pos2(
                    rect.min.x + kb_w,
                    sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
                ),
            );
            // Paint background first, then buttons on top
            ui.painter()
                .rect_filled(sb_left_blank, 0.0, theme::SCROLLBAR_BG);
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sb_left_blank), |ui| {
                ui.horizontal_centered(|ui| {
                    let mut count = panels.len();
                    crate::automation_panel::show_toggle_buttons(ui, show, &mut count);
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

    // ── Horizontal scrollbar (always rendered) ──
    // scrollbar::show handles the total_ticks <= 0 case internally,
    // matching the arrangement view's behavior.
    // NOTE: The left blank area (same width as keyboard) is NOT painted here.
    // It is painted inside the automation block alongside the AUTO buttons
    // so the buttons are not covered by a later background fill.
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
}

// ── Selection drag logic ──

fn sel_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    view: &mut yinhe_pianoroll::PianoRollView,
    midi: Option<&dyn yinhe_pianoroll::NoteSource>,
    selected: &mut std::collections::HashSet<(u16, u32)>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) {
    let sel_id = ui.id().with("sel_drag");
    let mut drag: Option<(egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Start drag
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && content_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        drag = Some((local, local));
        // Non-Cmd mode: clear selection on drag start
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        if !cmd {
            selected.clear();
        }
    }

    // Update during drag
    if let Some((start, _)) = drag {
        if pointer.primary_down() {
            if let Some(pos) = pointer.hover_pos() {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                drag = Some((start, local));
            }
        }

        // Release → hit test
        if pointer.primary_released() {
            if let (Some(midi_ref), Some((start, end))) = (midi, drag) {
                let sx = start.x.min(end.x);
                let ex = start.x.max(end.x);
                let sy = start.y.min(end.y);
                let ey = start.y.max(end.y);

                // Pixel → tick (X axis)
                let tick_s = view.x_to_tick(sx);
                let tick_e = view.x_to_tick(ex);

                // Pixel → key (Y axis)
                let kh = view.key_height;
                let scroll_y = view.base.scroll_y;

                // content_rect Y: 0 = top = key 127, h = bottom = key 0
                let key_lo = (127.0 - ((scroll_y + ey) / kh)).floor().max(0.0).min(127.0) as u8;
                let key_hi = (127.0 - ((scroll_y + sy) / kh)).ceil().max(0.0).min(127.0) as u8;

                // Snap ticks to quantize grid
                let snapped_s = snap_tick(tick_s, quantize, ppq, bar_line_data);
                let snapped_e = snap_tick(tick_e, quantize, ppq, bar_line_data);
                let t_start = snapped_s.min(snapped_e);
                let t_end = snapped_s.max(snapped_e);

                // Hit test: notes overlapping the rect
                let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                if !cmd {
                    selected.clear();
                }
                for key in key_lo..=key_hi {
                    for note in midi_ref.key_notes(key) {
                        if note.start_tick as f64 <= t_end && note.end_tick as f64 >= t_start {
                            selected.insert((note.track, note.start_tick));
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

    // Persist across frames
    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
}

/// Snap a tick value to the current quantize grid.
fn snap_tick(
    tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[yinhe_types::TimeSigEvent])>,
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
