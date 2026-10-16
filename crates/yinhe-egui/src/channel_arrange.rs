//! 通道 AR（MIX 主区域）视图。
//!
//! 与轨道 AR（`arrange`）的本质差异：以**通道为单位**（MIDI 源通道 A01..P16
//! 与音频通道 Audio-XX），而不是以轨道为单位。
//! - 折叠：通道内所有轨道的音符在 GPU 层合并画在通道行上（统一通道色）；
//! - 展开：通道行变窄作总览，下方平铺「轨道音符行 + 轨道自动化行」；
//! - Conductor 行显示 Tempo（直编，与原 AR 一致）。
//!
//! 编辑能力对齐原 AR：选框/移动、自动化锚点编辑、音频片段编辑；
//! 唯一的例外是橡皮擦（轨道 AR 因工具固定 Select 实际不可达，这里同样不提供）。

mod groups;
mod interaction;
mod panel;
mod render;
mod rows;
mod types;
mod view;

use eframe::egui;
use rust_i18n::t;

pub(crate) use groups::{CaChannel, CaGroups};
pub(crate) use rows::CaRowLayout;
pub(crate) use types::CaArRangeDelta;

use crate::render_context::RenderContext;
use yinhe_editor_core::audio_settings::LayoutSettings;
use yinhe_editor_core::document::Document;

/// 顶部时间标尺高度（与轨道 AR 相同）。
use crate::theme;

pub(crate) struct CaArrangeCfg<'a> {
    pub is_playing: bool,
    pub follow_mode: &'a mut crate::view_interaction::FollowMode,
    pub min_border_width: f32,
    pub revision: u64,
}

pub(crate) struct CaArrangeLayout<'a> {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub panel_width: &'a mut f32,
    /// 面板宽度拖拽刚结束（本帧释放），调用方据此持久化布局设置。
    pub drag_ended: &'a mut bool,
}

/// 通道 AR 单帧入口（MIX 模式主区域）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    layout: CaArrangeLayout<'_>,
    arr_renderer: &mut yinhe_wgpu::InstanceRenderer,
    arr_render_ctx: &mut RenderContext,
    cfg: CaArrangeCfg<'_>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    audio_library: &crate::app::audio_library::AudioLibrary,
    request_pianoroll: &mut bool,
    selection_anchor: &mut Option<u16>,
    arr_drag_delta: &mut Option<CaArRangeDelta>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    right_tab: &mut Option<crate::right_panel::RightTab>,
    needs_audio_notify: &mut bool,
    automation_picker_req: &mut Option<usize>,
    track_meters: &[(f32, f32)],
    // 通道电平显示值（L/R，独立衰减；见 App::ca_channel_levels）。
    midi_levels: &[crate::widgets::meter::StereoMeter],
    audio_levels: &[crate::widgets::meter::StereoMeter],
    // 通道行 M/S 变更输出（由调用方 note_mixer_edit + apply_*）。
    set_strips: &mut Vec<(u8, yinhe_mixer::StripParams)>,
    set_audio_strips: &mut Vec<(u16, yinhe_mixer::StripParams)>,
) {
    let arr_total_w = layout.remaining.width();
    crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.mix"));
    let tp_w = layout
        .panel_width
        .clamp(60.0, (arr_total_w - 60.0).max(60.0));
    *layout.panel_width = tp_w;

    let arr_rect = egui::Rect::from_min_max(
        layout.remaining.min,
        egui::pos2(
            layout.remaining.max.x,
            layout.remaining.min.y + layout.arr_h,
        ),
    );

    // 左侧面板：从标尺下方到横向滚动条上方（与 GPU 行对齐）。
    let tp_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, arr_rect.min.y + theme::RULER_H),
        egui::pos2(
            arr_rect.min.x + tp_w,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );
    // GPU 区：全宽纹理（含面板列；音符坐标以全宽左缘为原点，
    // 视口左边界由 shader clamp 排除）。
    let gpu_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, arr_rect.min.y + theme::RULER_H),
        egui::pos2(
            arr_rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(
            arr_rect.min.x + tp_w + theme::SPLIT_HANDLE_W,
            arr_rect.min.y + theme::RULER_H,
        ),
        gpu_rect.max,
    );

    doc.edit.chan_ar_view.base.left_panel_width = tp_w + theme::SPLIT_HANDLE_W;

    let total_ticks = crate::view_interaction::total_ticks_padded(
        doc.data.model.tick_length,
        doc.data.model.meta.ppq,
    );
    let conductor_idx = doc.edit.track_cache.conductor_idx;
    let conductor_color = crate::theme::conductor_color_f32();

    // ── 通道分组与行布局 ──
    let groups = CaGroups::from_model(&doc.data.model, conductor_idx);
    let build_layout = |edit: &yinhe_editor_core::edit_state::EditState,
                        model: &yinhe_core::YinModel| {
        let expanded_midi = &edit.chan_ar_expanded_midi;
        let expanded_audio = &edit.chan_ar_expanded_audio;
        CaRowLayout::build(
            conductor_idx.is_some(),
            &groups,
            |ch| match ch {
                CaChannel::Midi(c) => expanded_midi.get(c as usize).copied().unwrap_or(false),
                CaChannel::Audio(c) => expanded_audio.get(c as usize).copied().unwrap_or(false),
            },
            |t| {
                model
                    .tracks
                    .get(t as usize)
                    .map(|tr| tr.automation_lanes.len())
                    .unwrap_or(0)
            },
            edit.chan_ar_view.lane_height(),
        )
    };
    let mut row_layout = build_layout(&doc.edit, &doc.data.model);
    doc.edit.chan_ar_view.base.scroll_y = doc.edit.chan_ar_view.base.scroll_y.clamp(
        0.0,
        (row_layout.total_height() - gpu_rect.height()).max(0.0),
    );

    // ── 顶部标尺 ──
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x + tp_w + theme::SPLIT_HANDLE_W,
                arr_rect.min.y,
            ),
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + theme::RULER_H),
        );
        let corner_rect = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, arr_rect.min.y),
            egui::pos2(arr_rect.max.x, arr_rect.min.y + theme::RULER_H),
        );
        ui.painter()
            .rect_filled(corner_rect, 0.0, crate::theme::track_bg());

        let model = &*doc.data.model;
        let tpb = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let markers = model.conductor.markers.as_slice();
        // 量化档位解析：Auto（自适应）按当前通道 AR 缩放出具体网格。
        let ca_quantize = doc
            .edit
            .quantize_arrange
            .resolved(doc.edit.chan_ar_view.base.pixels_per_tick, tpb);
        let outcome = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            &mut doc.edit.chan_ar_view,
            tpb,
            def_num,
            def_den,
            sig_events,
            markers,
            |tick| {
                crate::view_interaction::snap_tick(
                    tick,
                    ca_quantize,
                    tpb,
                    Some((tpb, def_num, def_den, sig_events)),
                )
            },
            "ca_ruler",
            &mut doc.edit.cursor_tick,
        );
        if outcome.jumped {
            doc.edit.selected.clear();
            doc.edit.arr_sel_rect.clear();
            doc.edit.chan_ar_sel.clear();
        }
        if let Some(edit) = outcome.marker_edit {
            crate::marker_ops::apply_marker_edit(doc, edit);
        }
    }

    // ── 左侧通道面板 ──
    let mut picker_track: Option<u16> = None;
    let mut panel_audio_dirty = false;
    let mut panel_am_ms_dirty = false;
    // 面板色带统一为通道色（与 GPU 层同源）。
    let channel_colors = groups.track_colors_by_channel(
        &doc.edit.track_cache.colors,
        conductor_idx,
        conductor_color,
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(tp_rect), |ui| {
        ui.set_clip_rect(tp_rect);
        let io = &mut panel::CaPanelIo {
            track_info: &doc.edit.track_cache.info,
            track_colors: &channel_colors,
            track_visible: &doc.edit.track_visible,
            track_meters,
            track_overrides: &mut doc.edit.track_overrides,
            track_selected: &mut doc.edit.track_selected,
            selection_anchor,
            info_content,
            expanded_midi: &mut doc.edit.chan_ar_expanded_midi,
            expanded_audio: &mut doc.edit.chan_ar_expanded_audio,
            tracks: &doc.data.model.tracks,
            arr_am_ms: &mut doc.edit.arr_am_ms,
            request_pianoroll,
            midi_strips: &doc.mixer.channels,
            audio_strips: &doc.mixer.audio_channels,
            midi_levels,
            audio_levels,
            cursor_tick: doc.edit.cursor_tick,
            tempo_lane: &doc.data.model.conductor.tempo,
        };
        let out = panel::show(
            ui,
            &mut doc.edit.chan_ar_view,
            &row_layout,
            &groups,
            conductor_color,
            io,
        );
        panel_audio_dirty = out.audio_dirty;
        panel_am_ms_dirty = out.am_ms_dirty;
        picker_track = out.automation_picker;
        set_strips.extend(out.set_strips);
        set_audio_strips.extend(out.set_audio_strips);
    });
    if let Some(t) = picker_track {
        *automation_picker_req = Some(t as usize);
    }
    if panel_audio_dirty {
        crate::right_panel::info_panel::send_skip_tracks(doc, audio);
    }
    if panel_am_ms_dirty && let Some(audio) = audio {
        audio.set_am_ms(std::sync::Arc::new(doc.edit.arr_am_ms.clone()));
    }

    // 展开状态可能刚变：重建行布局并重新 clamp。
    row_layout = build_layout(&doc.edit, &doc.data.model);
    doc.edit.chan_ar_view.base.scroll_y = doc.edit.chan_ar_view.base.scroll_y.clamp(
        0.0,
        (row_layout.total_height() - gpu_rect.height()).max(0.0),
    );

    // ── GPU 视图 ──
    let mut am_edits: Vec<yinhe_types::AutomationEdit> = Vec::new();
    let mut audio_commands: Vec<crate::arrange::view_ui::AudioEditCmd> = Vec::new();
    ui.scope_builder(egui::UiBuilder::new().max_rect(gpu_rect), |ui| {
        let model = &*doc.data.model;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let data = crate::arrange::ArrangeData {
            midi: Some(model as &dyn yinhe_types::NoteSource),
            track_visible: &doc.edit.track_visible,
            track_colors: &doc.edit.track_cache.colors,
            track_info: &doc.edit.track_cache.info,
            quantize: doc
                .edit
                .quantize_arrange
                .resolved(doc.edit.chan_ar_view.base.pixels_per_tick, model.meta.ppq),
            ppq: model.meta.ppq,
            bar_line_data: Some((
                model.meta.ppq,
                def_num,
                def_den,
                model.tempo_map.time_sig_events.as_slice(),
            )),
            total_ticks,
            num_tracks: doc.edit.track_visible.len(),
            tracks: &model.tracks,
            tempo_lane: &model.conductor.tempo,
            conductor_track_idx: doc.edit.track_cache.conductor_idx,
            tempo_map: &model.tempo_map,
            audio_library,
            audio_sources: &model.audio_sources,
        };
        let mut no_eraser: Option<(f64, f64, usize, usize)> = None;
        let mut edit = crate::arrange::ArrangeEdit {
            selected: &mut doc.edit.selected,
            cursor_tick: &mut doc.edit.cursor_tick,
            arr_sel_rect: &mut doc.edit.arr_sel_rect,
            arr_drag_delta,
            arr_eraser_rect: &mut no_eraser,
            track_selected: &mut doc.edit.track_selected,
            selection_anchor,
            info_content,
            arr_am_views: &mut doc.edit.arr_am_views,
            am_edits: &mut am_edits,
            right_tab,
            selected_audio_clips: &mut doc.edit.selected_audio_clips,
            audio_commands: &mut audio_commands,
        };
        let mut cfg = view::CaViewCfg {
            is_playing: cfg.is_playing,
            follow_mode: cfg.follow_mode,
            min_border_width: cfg.min_border_width,
            revision: cfg.revision,
            conductor_color,
        };
        view::show(
            ui,
            gpu_rect.size(),
            arr_renderer,
            arr_render_ctx,
            &mut doc.edit.chan_ar_view,
            &row_layout,
            &groups,
            &data,
            &mut edit,
            &mut doc.edit.chan_ar_sel,
            &mut cfg,
        );
    });

    // ── 应用自动化编辑与音频片段编辑（出 scope 后，借用已释放）──
    if !am_edits.is_empty() {
        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(std::mem::take(&mut am_edits));
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.edit_automation").as_ref(),
            before,
        );
        *needs_audio_notify = true;
    }
    if !audio_commands.is_empty() {
        crate::arrange::apply_audio_edit_commands(
            doc,
            audio_library,
            std::mem::take(&mut audio_commands),
        );
        *needs_audio_notify = true;
    }

    // ── 右下角角落（横纵滚动条交叠区）：拖拽双向缩放、双击 fit ──
    {
        let corner_rect = egui::Rect::from_min_max(gpu_rect.max, arr_rect.max);
        let resp = ui.interact(
            corner_rect,
            ui.id().with("ca_corner_zoom"),
            egui::Sense::click_and_drag(),
        );
        let press_on = ui
            .input(|i| i.pointer.press_origin())
            .is_some_and(|p| corner_rect.contains(p));
        let active = press_on && resp.dragged();
        let base = crate::theme::track_bg();
        let fill = if active {
            crate::theme::pressed_color(base)
        } else if resp.hovered() {
            crate::theme::hover_color(base)
        } else {
            base
        };
        ui.painter().rect_filled(corner_rect, 0.0, fill);
        if resp.hovered() || active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        if active {
            let d = resp.drag_delta();
            let k = 0.005;
            let anchor_x = corner_rect.center().x - arr_rect.min.x;
            let anchor_y = corner_rect.center().y - arr_rect.min.y;
            doc.edit.chan_ar_view.zoom_around_x(anchor_x, 1.0 + d.x * k);
            doc.edit
                .chan_ar_view
                .zoom_lane_height(anchor_y, 1.0 + d.y * k);
            ui.ctx().request_repaint();
        } else if crate::widgets::scrollbar::double_clicked_in(ui, corner_rect) {
            doc.edit
                .chan_ar_view
                .fit_time(music_rect.width(), total_ticks);
            doc.edit
                .chan_ar_view
                .fit_lane_height(gpu_rect.height(), row_layout.total_rows());
            ui.ctx().request_repaint();
        }
        if crate::view_interaction::pointer_hits(ui, corner_rect) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.corner_zoom"));
        }
    }

    // ── 水平滚动条 ──
    {
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x + tp_w + theme::SPLIT_HANDLE_W,
                gpu_rect.max.y,
            ),
            egui::pos2(gpu_rect.max.x, arr_rect.max.y),
        );
        let preview = crate::widgets::scrollbar::DensityPreview {
            density: doc.density.view(),
            track_colors: &doc.edit.track_cache.colors,
        };
        let sb_drag_dy = crate::widgets::scrollbar::show(
            ui,
            sb_rect,
            music_rect.width(),
            &mut doc.edit.chan_ar_view.base.scroll_x,
            &mut doc.edit.chan_ar_view.base.pixels_per_tick,
            total_ticks,
            Some(&preview),
            doc.edit.cursor_tick,
            &mut doc.edit.chan_ar_view.base.dirty,
            yinhe_types::Orientation::Horizontal,
        );
        if crate::view_interaction::pointer_hits(ui, sb_rect) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.hscroll"));
        }
        if sb_drag_dy != 0.0 {
            let factor = 1.0 - sb_drag_dy * 0.005;
            let anchor_x = sb_rect.center().x - arr_rect.min.x;
            doc.edit.chan_ar_view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }
        if crate::widgets::scrollbar::double_clicked_in(ui, sb_rect) {
            doc.edit
                .chan_ar_view
                .fit_time(music_rect.width(), total_ticks);
            ui.ctx().request_repaint();
        }
        if crate::view_interaction::pointer_hits(ui, sb_rect) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                let anchor_x = sb_rect.center().x - arr_rect.min.x;
                doc.edit.chan_ar_view.zoom_around_x(anchor_x, factor);
                ui.ctx().request_repaint();
            }
        }
    }

    // ── 垂直滚动条（行数用等价均匀行数，行高不均时位置为近似）──
    {
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + theme::RULER_H),
            egui::pos2(arr_rect.max.x, gpu_rect.max.y),
        );
        let cells = row_layout.total_rows().max(1);
        let vsb_drag_dx = ui
            .push_id("ca_vscroll", |ui| {
                crate::widgets::scrollbar::show_vertical(
                    ui,
                    vsb_rect,
                    gpu_rect.height(),
                    &mut doc.edit.chan_ar_view.base.scroll_y,
                    &mut doc.edit.chan_ar_view.base.track_panel_row_height,
                    cells,
                    yinhe_types::LANE_HEIGHT_MIN,
                    yinhe_types::LANE_HEIGHT_MAX,
                    Some(yinhe_types::LANE_HEIGHT_STEP),
                    &mut doc.edit.chan_ar_view.base.dirty,
                    yinhe_types::Orientation::Horizontal,
                )
            })
            .inner;
        if crate::view_interaction::pointer_hits(ui, vsb_rect) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.vscroll"));
        }
        if vsb_drag_dx != 0.0 {
            let factor = 1.0 - vsb_drag_dx * 0.005;
            let anchor_y = vsb_rect.center().y - arr_rect.min.y;
            doc.edit.chan_ar_view.zoom_lane_height(anchor_y, factor);
            ui.ctx().request_repaint();
        }
        if crate::widgets::scrollbar::double_clicked_in(ui, vsb_rect) {
            doc.edit
                .chan_ar_view
                .fit_lane_height(gpu_rect.height(), row_layout.total_rows());
            ui.ctx().request_repaint();
        }
        if crate::view_interaction::pointer_hits(ui, vsb_rect) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                let anchor_y = vsb_rect.center().y - arr_rect.min.y;
                doc.edit.chan_ar_view.zoom_lane_height(anchor_y, factor);
                ui.ctx().request_repaint();
            }
        }
    }

    // ── 左上角角落（面板上方、标尺左侧）：仅铺背景 ──
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(arr_rect.min.x, arr_rect.min.y),
            egui::vec2(tp_w, theme::RULER_H),
        ),
        0.0,
        crate::theme::track_bg(),
    );
    // 左下角（面板下方、水平滚动条左侧）铺背景。
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x,
                arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
            ),
            egui::pos2(arr_rect.min.x + tp_w, arr_rect.max.y),
        ),
        0.0,
        crate::theme::track_bg(),
    );

    // ── 面板宽度纵向分割条（最后绘制，压在最上层）──
    let v_handle = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w, arr_rect.min.y),
        egui::pos2(
            arr_rect.min.x + tp_w + theme::SPLIT_HANDLE_W,
            arr_rect.max.y,
        ),
    );
    let v_resp = crate::widgets::split_handle::vertical(ui, "__ca_v_split__", v_handle);
    crate::widgets::hint::hover(ui.ctx(), &v_resp, t!("hint.arrange.split_v"));
    if v_resp.dragged() {
        *layout.panel_width =
            (*layout.panel_width + v_resp.drag_delta().x).clamp(60.0, arr_total_w - 60.0);
    }
    if v_resp.double_clicked() {
        // 双击分割线 → 还原面板默认宽度
        *layout.panel_width = LayoutSettings::default().transport_panel_width;
    }
    if v_resp.drag_stopped() || v_resp.double_clicked() {
        *layout.drag_ended = true;
    }
}
