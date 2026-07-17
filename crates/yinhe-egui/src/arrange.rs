mod track_panel;
mod view_ui;

use eframe::egui;

use yinhe_types::ArrangementView;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

/// Height of the time ruler band at the top of the arrangement view.
use crate::theme;
const RULER_H: f32 = theme::RULER_H;

/// Returns `Some(new_preset)` if the user picked a new quantize preset
/// from the corner AR button.
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_view: &mut ArrangementView,
    remaining: egui::Rect,
    arr_h: f32,
    transport_panel_width: &mut f32,
    arr_renderer: &mut yinhe_wgpu::InstanceRenderer,
    arr_render_ctx: &mut RenderContext,
    last_cursor_tick: &mut Option<f64>,
    is_playing: bool,
    follow_mode: &mut crate::view_interaction::FollowMode,
    active_tool: &Tool,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    request_pianoroll: &mut bool,
    selection_anchor: &mut Option<u16>,
    scroll_mode: u32,
    min_border_width: f32,
    haptic_engine: Option<&yinhe_haptic::HapticEngine>,
    arr_sel_rect: &mut Option<(f64, f64, usize, usize)>,
    arr_drag_delta: &mut Option<(i64, i32)>,
    arr_eraser_rect: &mut Option<(f64, f64, usize, usize)>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
) -> Option<QuantizePreset> {
    *last_cursor_tick = doc.edit.cursor_tick;

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
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );

    // ── GPU area: shifted down by RULER_H, shifted up by SCROLLBAR_H to leave room for the scrollbar ──
    let gpu_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y + RULER_H),
        egui::pos2(
            arr_rect.max.x,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );

    // Clamp scroll BEFORE drawing the ruler, so the ruler and GPU content
    // always see the same (clamped) scroll_x.  Otherwise when scroll_x is
    // pushed past a boundary by momentum/inertia scrolling, the ruler would
    // show unclamped positions while the GPU content (clamped inside
    // arrangement_view_ui::show) stays at the boundary — producing a visible
    // "bounce-back" effect on the ruler labels.
    let total_ticks = crate::view_interaction::total_ticks_padded(doc.data.model.tick_length, doc.data.model.meta.ppq);
    let num_tracks = doc.edit.track_visible.len();
    arr_view.clamp_scroll(gpu_rect.width(), gpu_rect.height(), total_ticks, num_tracks);

    // ── Ruler: top-right band, drawn with parent painter ──
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y),
            egui::pos2(arr_rect.max.x, arr_rect.min.y + RULER_H),
        );
        let model = &doc.data.model;
        let tpb = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let ruler_jumped = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            arr_view,
            tpb,
            def_num,
            def_den,
            sig_events,
            |tick| {
                crate::view_interaction::snap_tick(
                    tick,
                    doc.edit.quantize_arrange,
                    tpb,
                    Some((tpb, def_num, def_den, sig_events)),
                )
            },
            "arrange_ruler",
            &mut doc.edit.cursor_tick,
        );
        // 点击/拖动时间标尺跳转位置时，取消已选择的选框（含框选与全选）。
        if ruler_jumped {
            doc.edit.selected.clear();
            *arr_sel_rect = None;
        }
    }

    // ── Track panel content ──
    ui.scope_builder(egui::UiBuilder::new().max_rect(tp_rect), |ui| {
        ui.set_clip_rect(tp_rect);
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, crate::theme::APP_BG);

        arr_view.base.track_panel_scroll_y = arr_view.base.scroll_y;

        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001
            && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
            && tp_rect.contains(hover)
        {
            let pointer_y = hover.y - tp_rect.min.y;
            let old = arr_view.base.track_panel_row_height;
            arr_view.base.track_panel_row_height =
                (arr_view.base.track_panel_row_height * zoom_delta).clamp(16.0, 120.0);
            arr_view.lane_height = arr_view.base.track_panel_row_height;
            let track_frac = (pointer_y + arr_view.base.track_panel_scroll_y) / old;
            arr_view.base.track_panel_scroll_y =
                (track_frac * arr_view.base.track_panel_row_height - pointer_y).max(0.0);
            arr_view.base.dirty = true;
        }

        // Ensure parallel arrays are correctly sized (track count may have grown).
        let n = doc.edit.track_info_cache.len();
        if doc.edit.track_pianoroll_visible.len() < n {
            doc.edit.track_pianoroll_visible.resize(n, true);
        }
        if doc.edit.track_overrides.len() < n {
            doc.edit.track_overrides
                .resize(n, yinhe_editor_core::document::TrackOverride::default());
        }
        if doc.edit.track_colors_cache.len() < n {
            for i in doc.edit.track_colors_cache.len()..n {
                doc.edit.track_colors_cache
                    .push(yinhe_editor_core::document::track_color(i, doc.edit.conductor_track_idx));
            }
        }

        let (audio_dirty, track_actions) = track_panel::show(
            ui,
            &doc.edit.track_info_cache,
            &doc.edit.track_visible,
            &mut doc.edit.track_overrides,
            &mut doc.edit.track_selected,
            selection_anchor,
            doc.edit.conductor_track_idx,
            &doc.edit.track_colors_cache,
            &mut arr_view.base.track_panel_row_height,
            &mut arr_view.base.track_panel_scroll_y,
            request_pianoroll,
            info_content,
        );

        if audio_dirty {
            crate::right_panel::info_panel::send_skip_tracks(doc, audio);
        }

        // Handle track management actions (add/remove/move)
        for action in track_actions {
            let (undo_action, label) = match &action {
                track_panel::TrackAction::AddTrack { after_idx } => {
                    let idx = after_idx.unwrap_or(doc.data.model.tracks.len() - 1);
                    (doc.add_track(idx), "Add track")
                }
                track_panel::TrackAction::RemoveTrack { idx } => {
                    (doc.remove_track(*idx), "Remove track")
                }
                track_panel::TrackAction::MoveUp { idx } => {
                    if *idx > 0 {
                        (doc.move_track(*idx, *idx - 1), "Move track up")
                    } else {
                        (None, "")
                    }
                }
                track_panel::TrackAction::MoveDown { idx } => {
                    if *idx + 1 < doc.data.model.tracks.len() {
                        (doc.move_track(*idx, *idx + 1), "Move track down")
                    } else {
                        (None, "")
                    }
                }
            };
            if let Some(action) = undo_action {
                doc.history.push(yinhe_editor_core::history::UndoEntry {
                    action,
                    label,
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
                if let Some(ref audio) = audio {
                    audio.reload_notes(doc.data.model.clone());
                }
            }
        }

        arr_view.base.scroll_y = arr_view.base.track_panel_scroll_y;
    });

    // ── Arrangement GPU view (below ruler) ──
    let arr_midi: Option<&dyn yinhe_types::NoteSource> =
        Some(&*doc.data.model as &dyn yinhe_types::NoteSource);
    let gpu_size = gpu_rect.size();
    ui.scope_builder(egui::UiBuilder::new().max_rect(gpu_rect), |ui| {
        view_ui::show(
            ui,
            gpu_size,
            arr_renderer,
            arr_render_ctx,
            arr_view,
            arr_midi,
            &mut doc.edit.selected,
            &doc.edit.track_visible,
            &doc.edit.track_colors_cache,
            &doc.edit.track_info_cache,
            &mut doc.edit.cursor_tick,
            doc.edit.quantize_arrange,
            doc.data.model.meta.ppq,
            Some({
                let model = &doc.data.model;
                let (def_num, def_den) = model.tempo_map.time_sig_default;
                (
                    model.meta.ppq,
                    def_num,
                    def_den,
                    model.tempo_map.time_sig_events.as_slice(),
                )
            }),
            is_playing,
            &doc.data.track_names,
            follow_mode,
            active_tool,
            scroll_mode,
            min_border_width,
            haptic_engine,
            doc.data.revision,
            arr_sel_rect,
            arr_drag_delta,
            arr_eraser_rect,
            &mut doc.edit.track_selected,
            selection_anchor,
            info_content,
        );
    });

    // ── Horizontal scrollbar (right of track panel, below GPU content) ──
    {
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x + tp_w + 4.0, gpu_rect.max.y),
            egui::pos2(arr_rect.max.x, arr_rect.max.y),
        );
        crate::widgets::scrollbar::show(
            ui,
            sb_rect,
            gpu_rect.width(),
            &mut arr_view.base.scroll_x,
            &mut arr_view.base.pixels_per_tick,
            total_ticks,
            &mut arr_view.base.dirty,
        );
    }

    // ── AR quantize button in the top-left corner (left of ruler, above track panel) ──
    let mut pending_quantize = None;
    {
        let corner_rect = egui::Rect::from_min_size(
            egui::pos2(arr_rect.min.x, arr_rect.min.y),
            egui::vec2(tp_w, RULER_H),
        );
        let btn_size = 20.0;
        let btn_rect = egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
        let btn_resp = ui.interact(btn_rect, egui::Id::new("arr_quantize_btn"), egui::Sense::click());
        let hovered = btn_resp.hovered();

        let icon_color = if hovered {
            crate::theme::ACCENT_ACTIVE
        } else {
            egui::Color32::from_gray(160)
        };
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            doc.edit.quantize_arrange.label(),
            egui::FontId::proportional(11.0),
            icon_color,
        );

        egui::Popup::from_toggle_button_response(&btn_resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                let ppq = doc.data.model.meta.ppq;
                crate::widgets::quantize_popup::show(
                    ui,
                    ppq,
                    doc.edit.quantize_arrange,
                    &mut pending_quantize,
                );
            });
    }

    // ── "+" track add button in the corner (below track panel, left of scrollbar) ──
    {
        let corner_rect = egui::Rect::from_min_size(
            egui::pos2(arr_rect.min.x, arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H),
            egui::vec2(tp_w, crate::widgets::scrollbar::SCROLLBAR_H),
        );
        let btn_size = 20.0;
        let btn_rect = egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
        let btn_resp = ui.interact(btn_rect, egui::Id::new("arr_add_track_btn"), egui::Sense::click());
        let hovered = btn_resp.hovered();

        use egui_material_icons::icons::ICON_ADD;
        let icon_color = if hovered {
            crate::theme::ACCENT_ACTIVE
        } else {
            egui::Color32::from_gray(160)
        };
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_ADD.codepoint,
            egui::FontId::new(18.0, ICON_ADD.font_family()),
            icon_color,
        );

        if btn_resp.clicked() {
            let idx = doc.data.model.tracks.len() - 1;
            if let Some(action) = doc.add_track(idx) {
                doc.history.push(yinhe_editor_core::history::UndoEntry {
                    action,
                    label: "Add track",
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
                if let Some(ref audio) = audio {
                    audio.reload_notes(doc.data.model.clone());
                }
            }
        }
    }

    // ── Vertical splitter handle (drawn last so it sits on top) ──
    let v_handle = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w, arr_rect.min.y),
        egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.max.y),
    );
    let v_resp = crate::widgets::split_handle::vertical(ui, "__v_split__", v_handle);
    if v_resp.dragged() {
        *transport_panel_width =
            (*transport_panel_width + v_resp.drag_delta().x).clamp(60.0, arr_total_w - 60.0);
    }

    pending_quantize
}
