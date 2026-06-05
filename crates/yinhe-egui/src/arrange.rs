use eframe::egui;

use crate::arrangement_view_ui;
use crate::document::Document;
use crate::render_context::RenderContext;
use crate::track_panel;

/// Height of the time ruler band at the top of the arrangement view.
const RULER_H: f32 = 24.0;

pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    remaining: egui::Rect,
    arr_h: f32,
    transport_panel_width: &mut f32,
    arr_renderer: &mut yinhe_arrangement::PianorollRenderer,
    arr_render_ctx: &mut RenderContext,
    last_cursor_tick: &mut Option<f64>,
    is_playing: bool,
    follow_mode: &mut crate::view_interaction::FollowMode,
) {
    if doc.cursor_tick != *last_cursor_tick {
        doc.arr_view.dirty = true;
    }
    *last_cursor_tick = doc.cursor_tick;

    let arr_total_w = remaining.width();
    let tp_w = transport_panel_width.clamp(60.0, (arr_total_w - 60.0).max(60.0));
    *transport_panel_width = tp_w;

    let arr_rect = egui::Rect::from_min_max(
        remaining.min,
        egui::pos2(remaining.max.x, remaining.min.y + arr_h),
    );

    // ── Track panel: starts at RULER_H, ends at scrollbar top so rows align with GPU lanes ──
    let tp_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, arr_rect.min.y + RULER_H),
        egui::pos2(
            arr_rect.min.x + tp_w,
            arr_rect.max.y - crate::scrollbar::SCROLLBAR_H,
        ),
    );

    // ── GPU area: shifted down by RULER_H, shifted up by SCROLLBAR_H to leave room for the scrollbar ──
    let gpu_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y + RULER_H),
        egui::pos2(
            arr_rect.max.x,
            arr_rect.max.y - crate::scrollbar::SCROLLBAR_H,
        ),
    );

    // Clamp scroll BEFORE drawing the ruler, so the ruler and GPU content
    // always see the same (clamped) scroll_x.  Otherwise when scroll_x is
    // pushed past a boundary by momentum/inertia scrolling, the ruler would
    // show unclamped positions while the GPU content (clamped inside
    // arrangement_view_ui::show) stays at the boundary — producing a visible
    // "bounce-back" effect on the ruler labels.
    let total_ticks = crate::view_interaction::total_ticks_padded(doc.midi.tick_length);
    let num_tracks = doc.track_visible.len();
    doc.arr_view
        .clamp_scroll(gpu_rect.width(), gpu_rect.height(), total_ticks, num_tracks);

    // ── Ruler: top-right band, drawn with parent painter ──
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y),
            egui::pos2(arr_rect.max.x, arr_rect.min.y + RULER_H),
        );
        let tpb = doc.midi.ticks_per_beat;
        let def_num = doc.midi.time_sig_numerator;
        let def_den = doc.midi.time_sig_denominator;
        let sig_events = doc.midi.time_sig_events.as_slice();
        // Parent painter works in screen coordinates; paint_labels applies
        // offset_x = rect.min.x - view.content_left() internally.
        let ruler_painter = ui.painter();
        crate::time_ruler::paint(
            &ruler_painter,
            ruler_rect,
            &doc.arr_view,
            tpb,
            def_num,
            def_den,
            sig_events,
        );
    }

    // ── Track panel content ──
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(tp_rect), |ui| {
        ui.set_clip_rect(tp_rect);
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);

        doc.arr_view.track_panel_scroll_y = doc.arr_view.scroll_y;

        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                if tp_rect.contains(hover) {
                    let pointer_y = hover.y - tp_rect.min.y;
                    let old = doc.arr_view.track_panel_row_height;
                    doc.arr_view.track_panel_row_height =
                        (doc.arr_view.track_panel_row_height * zoom_delta).clamp(16.0, 120.0);
                    doc.arr_view.lane_height = doc.arr_view.track_panel_row_height;
                    let track_frac = (pointer_y + doc.arr_view.track_panel_scroll_y) / old;
                    doc.arr_view.track_panel_scroll_y =
                        (track_frac * doc.arr_view.track_panel_row_height - pointer_y).max(0.0);
                    doc.arr_view.dirty = true;
                }
            }
        }

        track_panel::show(
            ui,
            &doc.track_info_cache,
            &mut doc.track_visible,
            &mut doc.track_selected,
            &doc.pc_map_cache,
            &mut doc.arr_view.track_panel_row_height,
            &mut doc.arr_view.track_panel_scroll_y,
        );

        doc.arr_view.scroll_y = doc.arr_view.track_panel_scroll_y;
    });

    // ── Vertical splitter handle ──
    // Full height: covers ruler area above and scrollbar area below
    // so the handle is always reachable regardless of cursor position.
    let v_handle = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w, arr_rect.min.y),
        egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.max.y),
    );
    let v_resp = ui.interact(
        v_handle,
        ui.id().with("__v_split__"),
        egui::Sense::click_and_drag(),
    );
    let v_hovered = v_resp.hovered() || v_resp.dragged();
    ui.painter().rect_filled(
        v_handle,
        0.0,
        if v_hovered {
            egui::Color32::from_gray(160)
        } else {
            egui::Color32::from_gray(80)
        },
    );
    if v_resp.dragged() {
        *transport_panel_width =
            (*transport_panel_width + v_resp.drag_delta().x).clamp(60.0, arr_total_w - 60.0);
    }
    if v_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    // ── Arrangement GPU view (below ruler) ──
    let arr_midi: Option<&dyn yinhe_arrangement::NoteSource> =
        Some(&doc.midi as &dyn yinhe_arrangement::NoteSource);
    let track_colors = doc.track_colors();
    let track_names = doc.track_names();
    let gpu_size = gpu_rect.size();
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(gpu_rect), |ui| {
        arrangement_view_ui::show(
            ui,
            gpu_size,
            arr_renderer,
            arr_render_ctx,
            &mut doc.arr_view,
            arr_midi,
            &doc.track_visible,
            &track_colors,
            &mut doc.cursor_tick,
            doc.quantize,
            doc.midi.ticks_per_beat,
            Some((
                doc.midi.ticks_per_beat,
                doc.midi.time_sig_numerator,
                doc.midi.time_sig_denominator,
                doc.midi.time_sig_events.as_slice(),
            )),
            is_playing,
            &track_names,
            &mut doc.arr_instances,
            follow_mode,
        );
    });

    // ── Horizontal scrollbar (right of track panel, below GPU content) ──
    {
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x + tp_w + 4.0, gpu_rect.max.y),
            egui::pos2(arr_rect.max.x, arr_rect.max.y),
        );
        crate::scrollbar::show(
            ui,
            sb_rect,
            gpu_rect.width(),
            &mut doc.arr_view.scroll_x,
            &mut doc.arr_view.pixels_per_tick,
            total_ticks,
            &mut doc.arr_view.dirty,
        );
    }
}
