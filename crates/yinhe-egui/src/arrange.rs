mod track_panel;
mod view_ui;

use eframe::egui;
use rust_i18n::t;

use yinhe_types::ArrangementView;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

/// Height of the time ruler band at the top of the arrangement view.
use crate::theme;
const RULER_H: f32 = theme::RULER_H;

/// Arrange 拖拽偏移量：(tick delta, track 行 delta)。
pub(crate) type ArrDragDelta = (i64, i32);
/// Arrange 选框/橡皮擦矩形：(t_start, t_end, track_lo, track_hi)。
pub(crate) type ArrSelRect = (f64, f64, usize, usize);

/// Arrange 视图的只读模型数据（同一帧内不变）。
pub(crate) struct ArrangeData<'a> {
    pub midi: Option<&'a dyn yinhe_types::NoteSource>,
    pub track_visible: &'a [bool],
    pub track_colors: &'a [[f32; 4]],
    pub track_info: &'a [yinhe_core::TrackInfo],
    pub quantize: QuantizePreset,
    pub ppq: u32,
    pub bar_line_data: Option<(u32, u8, u8, &'a [yinhe_types::TimeSigEvent])>,
    pub total_ticks: f64,
    pub num_tracks: usize,
}

/// Arrange 交互产生的可变编辑状态（out-params 聚合）。
pub(crate) struct ArrangeEdit<'a> {
    pub selected: &'a mut yinhe_core::Selection,
    pub cursor_tick: &'a mut Option<f64>,
    pub arr_sel_rect: &'a mut Vec<ArrSelRect>,
    pub arr_drag_delta: &'a mut Option<ArrDragDelta>,
    pub arr_eraser_rect: &'a mut Option<ArrSelRect>,
    pub track_selected: &'a mut std::collections::HashSet<u16>,
    pub selection_anchor: &'a mut Option<u16>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
}

/// Arrange 视图/播放/渲染配置（layout.rs 每帧构造）。
pub(crate) struct ArrangeViewCfg<'a> {
    pub is_playing: bool,
    pub follow_mode: &'a mut crate::view_interaction::FollowMode,
    pub active_tool: &'a Tool,
    pub scroll_mode: u32,
    pub min_border_width: f32,
    pub revision: u64,
}

/// Arrange 布局几何。
pub(crate) struct ArrangeLayout<'a> {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub transport_panel_width: &'a mut f32,
}

/// Returns `Some(new_preset)` if the user picked a new quantize preset
/// from the corner AR button.
///
/// 编排视图协调器：聚合 doc/布局/渲染/配置/编辑原料/信号六个职责面的输入，
/// 内部按只读数据、可变编辑状态、视图配置三轴分发（见 ArrangeData 等）。
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_view: &mut ArrangementView,
    layout: ArrangeLayout<'_>,
    arr_renderer: &mut yinhe_wgpu::InstanceRenderer,
    arr_render_ctx: &mut RenderContext,
    mut cfg: ArrangeViewCfg<'_>,
    last_cursor_tick: &mut Option<f64>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    request_pianoroll: &mut bool,
    selection_anchor: &mut Option<u16>,
    arr_drag_delta: &mut Option<ArrDragDelta>,
    arr_eraser_rect: &mut Option<ArrSelRect>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    // 音轨结构变化（add/remove track）需要 teardown + 重建音频引擎。
    // 由调用方 layout.rs 读取后调 `App::teardown_audio()`，下一帧
    // `rebuild_audio_if_needed` 会用新 model 重新 spawn 引擎和 ChannelLayout。
    needs_audio_rebuild: &mut bool,
    status_hint: &mut Option<String>,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
) -> Option<QuantizePreset> {
    *last_cursor_tick = doc.edit.cursor_tick;

    let arr_total_w = layout.remaining.width();
    let tp_w = layout
        .transport_panel_width
        .clamp(60.0, (arr_total_w - 60.0).max(60.0));
    *layout.transport_panel_width = tp_w;

    let arr_rect = egui::Rect::from_min_max(
        layout.remaining.min,
        egui::pos2(
            layout.remaining.max.x,
            layout.remaining.min.y + layout.arr_h,
        ),
    );

    // ── Track panel: starts at RULER_H, ends at scrollbar top so rows align with GPU lanes ──
    let tp_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, arr_rect.min.y + RULER_H),
        egui::pos2(
            arr_rect.min.x + tp_w,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );

    // ── GPU area: shifted down by RULER_H, shifted up by SCROLLBAR_H,
    //    shifted left by SCROLLBAR_W to leave room for the vertical scrollbar ──
    let gpu_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y + RULER_H),
        egui::pos2(
            arr_rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );

    // Clamp scroll BEFORE drawing the ruler, so the ruler and GPU content
    // always see the same (clamped) scroll_x.  Otherwise when scroll_x is
    // pushed past a boundary by momentum/inertia scrolling, the ruler would
    // show unclamped positions while the GPU content (clamped inside
    // arrangement_view_ui::show) stays at the boundary — producing a visible
    // "bounce-back" effect on the ruler labels.
    let total_ticks = crate::view_interaction::total_ticks_padded(
        doc.data.model.tick_length,
        doc.data.model.meta.ppq,
    );
    let num_tracks = doc.edit.track_visible.len();
    arr_view.clamp_scroll(gpu_rect.width(), gpu_rect.height(), total_ticks, num_tracks);

    // ── Ruler: top-right band, drawn with parent painter ──
    //    ruler 右边界对齐 gpu_rect.max.x，让出垂直滚动条空间
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y),
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + RULER_H),
        );
        let model = &*doc.data.model;
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
            doc.edit.arr_sel_rect.clear();
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
            doc.edit
                .track_overrides
                .resize(n, yinhe_editor_core::document::TrackOverride::default());
        }
        if doc.edit.track_colors_cache.len() < n {
            for i in doc.edit.track_colors_cache.len()..n {
                doc.edit
                    .track_colors_cache
                    .push(yinhe_editor_core::document::track_color(
                        &doc.data.model.tracks[i],
                        i,
                        doc.edit.conductor_track_idx,
                    ));
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
            &mut doc.edit.editing_track,
            info_content,
        );

        if audio_dirty {
            crate::right_panel::info_panel::send_skip_tracks(doc, audio);
        }

        // Handle track management actions (add/remove/move)
        for action in track_actions {
            let before = doc.capture_snapshot();
            let (undo_action, label) = match &action {
                track_panel::TrackAction::AddTrack { after_idx } => {
                    let idx = after_idx.unwrap_or(doc.data.model.tracks.len() - 1);
                    (doc.add_track(idx), t!("undo.add_track").to_string())
                }
                track_panel::TrackAction::RemoveTrack { idx } => {
                    (doc.remove_track(*idx), t!("undo.remove_track").to_string())
                }
                track_panel::TrackAction::MoveUp { idx } => {
                    if *idx > 0 {
                        (
                            doc.move_track(*idx, *idx - 1),
                            t!("undo.move_track_up").to_string(),
                        )
                    } else {
                        (None, String::new())
                    }
                }
                track_panel::TrackAction::MoveDown { idx } => {
                    if *idx + 1 < doc.data.model.tracks.len() {
                        (
                            doc.move_track(*idx, *idx + 1),
                            t!("undo.move_track_down").to_string(),
                        )
                    } else {
                        (None, String::new())
                    }
                }
            };
            if let Some(action) = undo_action {
                doc.push_undo(action, &label, before);
                // 方案 A：音轨结构变化（add/remove/move）→ teardown + 下帧重建。
                // 不再调 audio.reload_notes —— ChannelLayout 在引擎创建时冻结，
                // reload_notes 不会更新 active_mask/channel_map，旧引擎无法 dispatch 新通道。
                *needs_audio_rebuild = true;
            }
        }

        arr_view.base.scroll_y = arr_view.base.track_panel_scroll_y;
    });

    // ── Arrangement GPU view (below ruler) ──
    let gpu_size = gpu_rect.size();
    ui.scope_builder(egui::UiBuilder::new().max_rect(gpu_rect), |ui| {
        let model = &*doc.data.model;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let data = ArrangeData {
            midi: Some(model as &dyn yinhe_types::NoteSource),
            track_visible: &doc.edit.track_visible,
            track_colors: &doc.edit.track_colors_cache,
            track_info: &doc.edit.track_info_cache,
            quantize: doc.edit.quantize_arrange,
            ppq: model.meta.ppq,
            bar_line_data: Some((
                model.meta.ppq,
                def_num,
                def_den,
                model.tempo_map.time_sig_events.as_slice(),
            )),
            total_ticks,
            num_tracks,
        };
        let mut edit = ArrangeEdit {
            selected: &mut doc.edit.selected,
            cursor_tick: &mut doc.edit.cursor_tick,
            arr_sel_rect: &mut doc.edit.arr_sel_rect,
            arr_drag_delta,
            arr_eraser_rect,
            track_selected: &mut doc.edit.track_selected,
            selection_anchor,
            info_content,
        };
        view_ui::show(
            ui,
            gpu_size,
            arr_renderer,
            arr_render_ctx,
            arr_view,
            data,
            &mut edit,
            &mut cfg,
        );
    });

    // ── Horizontal scrollbar (right of track panel, below GPU content) ──
    //    让出右下角 SCROLLBAR_W × SCROLLBAR_H 给垂直滚动条+水平滚动条的交叠区
    {
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x + tp_w + 4.0, gpu_rect.max.y),
            egui::pos2(gpu_rect.max.x, arr_rect.max.y),
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

    // ── Vertical scrollbar (right of GPU content, full AR height minus ruler) ──
    //    像素空间：num_cells = num_tracks，cell_size = lane height (track_panel_row_height)
    {
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + RULER_H),
            egui::pos2(arr_rect.max.x, gpu_rect.max.y),
        );
        ui.push_id("arr_vscroll", |ui| {
            crate::widgets::scrollbar::show_vertical(
                ui,
                vsb_rect,
                gpu_rect.height(),
                &mut arr_view.base.scroll_y,
                &mut arr_view.base.track_panel_row_height,
                num_tracks,
                16.0,
                120.0,
                &mut arr_view.base.dirty,
            );
        });
    }

    // ── AR quantize button in the top-left corner (left of ruler, above track panel) ──
    let mut pending_quantize = None;
    {
        let corner_rect = egui::Rect::from_min_size(
            egui::pos2(arr_rect.min.x, arr_rect.min.y),
            egui::vec2(tp_w, RULER_H),
        );
        let btn_size = 20.0;
        let btn_rect =
            egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
        let btn_resp = ui.interact(
            btn_rect,
            egui::Id::new("arr_quantize_btn"),
            egui::Sense::click(),
        );
        let hovered = btn_resp.hovered();

        let icon_color = if hovered {
            crate::theme::ACCENT_ACTIVE
        } else {
            crate::theme::TEXT_MUTED
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
            egui::pos2(
                arr_rect.min.x,
                arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
            ),
            egui::vec2(tp_w, crate::widgets::scrollbar::SCROLLBAR_H),
        );
        let btn_size = 20.0;
        let btn_rect =
            egui::Rect::from_center_size(corner_rect.center(), egui::vec2(btn_size, btn_size));
        let btn_resp = ui.interact(
            btn_rect,
            egui::Id::new("arr_add_track_btn"),
            egui::Sense::click(),
        );
        let hovered = btn_resp.hovered();

        use egui_material_icons::icons::ICON_ADD;
        let icon_color = if hovered {
            crate::theme::ACCENT_ACTIVE
        } else {
            crate::theme::TEXT_MUTED
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
            let before = doc.capture_snapshot();
            if let Some(action) = doc.add_track(idx) {
                doc.push_undo(action, t!("undo.add_track").as_ref(), before);
                // 方案 A：add_track → teardown + 下帧重建（同 track_actions 分支）。
                *needs_audio_rebuild = true;
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
        *layout.transport_panel_width =
            (*layout.transport_panel_width + v_resp.drag_delta().x).clamp(60.0, arr_total_w - 60.0);
    }

    // ── 状态栏讲解行：走带悬停提示（位置 + 音轨号；有选框时优先显示选框统计）──
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
        let model = &*doc.data.model;
        let tpb = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let lh = arr_view.lane_height();
        let scroll_y = arr_view.base.scroll_y;
        let hover_track =
            |y: f32| (((y + scroll_y) / lh).floor() as usize).min(num_tracks.saturating_sub(1));
        let track_str = |track: usize| t!("hint.track", n = format!("{:03}", track)).to_string();
        // 本视图有选框 → 讲解行显示选框统计（参考 info panel）
        let sel_text = if !doc.edit.arr_sel_rect.is_empty()
            && let Some(sh) = sel_hint
        {
            Some(t!("hint.sel_notes", n = sh.count, span = &sh.span).to_string())
        } else {
            None
        };
        if gpu_rect.contains(pos) {
            let tick = arr_view.x_to_tick(pos.x - gpu_rect.min.x).max(0.0);
            let track = hover_track(pos.y);
            let pos_str =
                format_tick_bar_beat_with_time_sig(tick, tpb, sig_events, def_num, def_den);
            *status_hint = Some(if let Some(s) = sel_text {
                s
            } else {
                format!("{} {}", pos_str, track_str(track))
            });
        } else if tp_rect.contains(pos) {
            *status_hint = Some(if let Some(s) = sel_text {
                s
            } else {
                track_str(hover_track(pos.y))
            });
        } else if arr_rect.contains(pos) {
            // 走带视图内但不在内容区（标尺/滚动条）→ 清空
            *status_hint = None;
        }
    }

    pending_quantize
}
