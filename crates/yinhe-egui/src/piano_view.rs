use std::sync::Arc;

use eframe::egui;

use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::quantize::QuantizePreset;

/// Height of the time ruler band at the top of the pianoroll view.
use crate::theme;
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
    selected: &std::collections::HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    last_cursor_tick: &mut Option<f64>,
    follow_mode: &mut super::view_interaction::FollowMode,
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
        (rect.height() - RULER_H - panels_total_h - super::scrollbar::SCROLLBAR_H).max(0.0);
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
    );

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
                crate::theme::MIN_KEYBOARD_WIDTH,
                rect.width() * crate::theme::MAX_KEYBOARD_RATIO,
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
    let gpu_dirty = crate::qos::guarded(|| {
        yinhe_pianoroll::prepare(
            pianoroll,
            w,
            h,
            midi,
            view,
            selected,
            track_visible,
            *cursor_tick,
            force_rebuild,
        )
    });

    let content_changed = view.base.dirty || gpu_dirty;
    view.base.dirty = false;

    // Paint wgpu content into the content_rect (below the ruler)
    crate::qos::guarded(|| {
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
        crate::time_ruler::paint(
            &painter, ruler_rect, view, tpb, def_num, def_den, sig_events,
        );
    }

    // ── Automation panels + scrollbar + AUTO buttons ──
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

        // Automation panels
        let _panels_h = crate::automation_panel::show_panels(
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

        // ── Horizontal scrollbar + AUTO buttons in left blank ──
        if midi.is_some() {
            // sb_y is always at the bottom of the available rect regardless
            // of panel heights; compute directly instead of depending on
            // pre-computed panels_total_h which may be stale mid-drag.
            let sb_y = rect.min.y + rect.height() - super::scrollbar::SCROLLBAR_H;

            // Left blank area (same width as piano keyboard) — houses AUTO +/- buttons
            let sb_left_blank = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, sb_y),
                egui::pos2(rect.min.x + kb_w, sb_y + super::scrollbar::SCROLLBAR_H),
            );
            // Actual scrollbar track (right of keyboard)
            let sb_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + kb_w, sb_y),
                egui::pos2(rect.max.x, sb_y + super::scrollbar::SCROLLBAR_H),
            );

            // Paint left blank background
            ui.painter()
                .rect_filled(sb_left_blank, 0.0, theme::SCROLLBAR_BG);

            // AUTO +/- buttons inside left blank
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sb_left_blank), |ui| {
                ui.horizontal_centered(|ui| {
                    let mut count = panels.len();
                    crate::automation_panel::show_toggle_buttons(ui, show, &mut count);
                    // Add/remove panels to match count
                    while panels.len() < count {
                        panels.push(yinhe_automation::AutomationPanelView::default());
                    }
                    while panels.len() > count {
                        panels.pop();
                    }
                });
            });

            // Scrollbar
            ui.push_id("piano_scrollbar", |ui| {
                super::scrollbar::show(
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
    }
}
