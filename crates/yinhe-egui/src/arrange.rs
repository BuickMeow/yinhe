mod am_lanes;
mod track_panel;
mod view_ui;

use eframe::egui;
use rust_i18n::t;

use yinhe_types::ArrangementView;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;
use yinhe_editor_core::audio_settings::LayoutSettings;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

/// Height of the time ruler band at the top of the arrangement view.
use crate::theme;
const RULER_H: f32 = theme::RULER_H;

/// Arrange 拖拽偏移量：(tick delta, track 行 delta, alt 复制标志)。
pub(crate) type ArrDragDelta = (i64, i32, bool);
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
    /// 音轨数据（AM lane 渲染/交互读取 automation_lanes）。
    pub tracks: &'a [std::sync::Arc<yinhe_core::TrackData>],
    /// Conductor 的 Tempo lane（Conductor 主行直显/直编）。
    pub tempo_lane: &'a yinhe_types::AutomationLane,
    pub conductor_track_idx: Option<u16>,
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
    /// 每条 AM lane 的持久视图状态（锚点选框等），key = (音轨, target)。
    pub arr_am_views: &'a mut std::collections::HashMap<
        (u16, yinhe_types::AutomationTarget),
        yinhe_types::AutomationPanelView,
    >,
    /// AR 自动化 lane 交互产生的编辑（arrange.rs 在 GPU scope 后统一应用）。
    pub am_edits: &'a mut Vec<yinhe_types::AutomationEdit>,
    /// AM 锚点右键打开信息面板用。
    pub right_tab: &'a mut Option<crate::right_panel::RightTab>,
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
    /// 分割线拖拽刚结束（本帧释放），调用方据此持久化布局设置。
    pub drag_ended: &'a mut bool,
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
    // AM 锚点右键打开信息面板（与 PR 自动化面板一致）。
    right_tab: &mut Option<crate::right_panel::RightTab>,
    // 音轨结构变化（add/remove track）需要 teardown + 重建音频引擎。
    // 由调用方 layout.rs 读取后调 `App::teardown_audio()`，下一帧
    // `rebuild_audio_if_needed` 会用新 model 重新 spawn 引擎和 ChannelLayout。
    needs_audio_rebuild: &mut bool,
    // 自动化内容变化（AM lane 增删 / 事件编辑）需要 notify_audio_model_changed。
    needs_audio_notify: &mut bool,
    status_hint: &mut Option<String>,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
    // 右键「音轨属性」等请求：请求打开属性浮窗（由调用方 set_float_panel 落地）。
    float_panel_req: &mut Option<crate::right_panel::FloatPanel>,
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

    // ── GPU area: 全宽纹理（含轨道面板列，left_panel_width 同步为面板宽 +
    //    分屏条宽，音符坐标以全宽左缘为原点，视口左边界由 shader clamp 排除）。
    //    y 方向：shifted down by RULER_H, shifted up by SCROLLBAR_H,
    //    x 方向：shifted left by SCROLLBAR_W to leave room for the vertical scrollbar ──
    let gpu_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, arr_rect.min.y + RULER_H),
        egui::pos2(
            arr_rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );
    // 音乐区（旧 gpu_rect）：纹理内的可见区域，交互命中与绘制 clip 都限制在这里。
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(
            arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
            arr_rect.min.y + RULER_H,
        ),
        gpu_rect.max,
    );

    // 轨道面板宽度同步进视图坐标模型：left_panel_width = 面板宽 + 分屏条宽
    // （纹理左缘到音乐区左缘的距离），tick_to_x/x_to_tick/clamp_scroll 全部基于它。
    arr_view.base.left_panel_width = tp_w + crate::theme::SPLIT_HANDLE_W;

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

    // ── AM 展开状态与行布局：音轨面板与 GPU 视图共用（子行高 = 音轨行高）──
    // Conductor 不展开（主行直显 Tempo）。
    doc.edit.arr_am_expanded.resize(num_tracks, false);
    let conductor_idx = doc.edit.conductor_track_idx;
    let build_row_layout = |edit: &yinhe_editor_core::edit_state::EditState,
                            model: &yinhe_core::YinModel| {
        yinhe_types::ArRowLayout::new((0..num_tracks).map(|i| {
            if edit.arr_am_expanded.get(i).copied().unwrap_or(false)
                && Some(i as u16) != conductor_idx
            {
                model
                    .tracks
                    .get(i)
                    .map(|t| t.automation_lanes.len())
                    .unwrap_or(0) as u32
            } else {
                0
            }
        }))
    };
    let mut row_layout = build_row_layout(&doc.edit, &doc.data.model);
    // 清理已删除 lane 的残留视图状态（undo 删除 lane 后）
    doc.edit.arr_am_views.retain(|&(t, ref target), _| {
        doc.data
            .model
            .tracks
            .get(t as usize)
            .is_some_and(|tr| tr.automation_lanes.iter().any(|l| &l.target == target))
    });
    arr_view.clamp_scroll(
        gpu_rect.width(),
        gpu_rect.height(),
        total_ticks,
        row_layout.total_rows(),
    );

    // ── Ruler: top-right band, drawn with parent painter ──
    //    ruler 右边界对齐 gpu_rect.max.x，让出垂直滚动条空间
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
                arr_rect.min.y,
            ),
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + RULER_H),
        );

        // 右上角角落：标尺右缘到垂直滚动条之间（SCROLLBAR_W × RULER_H）
        let corner_rect = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, arr_rect.min.y),
            egui::pos2(arr_rect.max.x, arr_rect.min.y + RULER_H),
        );
        ui.painter()
            .rect_filled(corner_rect, 0.0, crate::theme::track_bg());

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
            .rect_filled(ui.max_rect(), 0.0, crate::theme::app_bg());

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

        let (audio_dirty, am_ms_dirty, track_actions) = track_panel::show(
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
            &row_layout,
            &doc.data.model.tracks,
            &mut doc.edit.arr_am_expanded,
            &mut doc.edit.arr_am_selected,
            &mut doc.edit.arr_am_ms,
        );

        if audio_dirty {
            crate::right_panel::info_panel::send_skip_tracks(doc, audio);
        }
        // AM lane M/S 试听开关：触发模型重载（App::notify_audio_model_changed 会带上 arr_am_ms）。
        if am_ms_dirty {
            *needs_audio_notify = true;
        }

        // Handle track management actions (add/remove/move/AM lane)
        for action in track_actions {
            let before = doc.capture_snapshot();
            // 结构变化（增删移动音轨）→ 重建引擎；AM 内容变化 → 仅 notify。
            let mut structural = true;
            let (undo_action, label) = match &action {
                // 右键「音轨属性」：不产生 undo，选中目标轨后请求打开浮窗。
                track_panel::TrackAction::ShowProperties { idx } => {
                    let track_idx = doc
                        .edit
                        .track_info_cache
                        .get(*idx)
                        .map(|t| t.index)
                        .unwrap_or(*idx as u16);
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(track_idx);
                    *info_content = Some(crate::right_panel::InfoContent::Track);
                    *float_panel_req =
                        Some(crate::right_panel::FloatPanel::TrackProps { track_idx });
                    (None, String::new())
                }
                track_panel::TrackAction::CreateAutomation { idx, target } => {
                    structural = false;
                    let r = doc.add_automation_lane(*idx, target.clone());
                    if r.is_some()
                        && let Some(e) = doc.edit.arr_am_expanded.get_mut(*idx)
                    {
                        // 创建后自动展开该轨
                        *e = true;
                    }
                    (r.map(|(_, a)| a), t!("undo.create_automation").to_string())
                }
                track_panel::TrackAction::DeleteAutomation { idx, lane_idx } => {
                    structural = false;
                    (
                        doc.remove_automation_lane(*idx, *lane_idx),
                        t!("undo.delete_automation_lane").to_string(),
                    )
                }
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
                track_panel::TrackAction::MoveTracks { indices, insert_at } => {
                    // 拖拽排序：拆成逐个 move_track，合并为一个 Composite undo。
                    let moves = crate::widgets::reorder::plan_moves(
                        doc.data.model.tracks.len(),
                        indices,
                        *insert_at,
                    );
                    let mut sub_actions = Vec::new();
                    for (from, to) in moves {
                        if let Some(a) = doc.move_track(from, to) {
                            sub_actions.push(a);
                        }
                    }
                    (
                        if sub_actions.is_empty() {
                            None
                        } else {
                            Some(yinhe_editor_core::history::UndoAction::Composite(
                                sub_actions,
                            ))
                        },
                        t!("undo.move_track").to_string(),
                    )
                }
            };
            if let Some(action) = undo_action {
                doc.push_undo(action, &label, before);
                if structural {
                    // 方案 A：音轨结构变化（add/remove/move）→ teardown + 下帧重建。
                    // 不再调 audio.reload_notes —— ChannelLayout 在引擎创建时冻结，
                    // reload_notes 不会更新 active_mask/channel_map，旧引擎无法 dispatch 新通道。
                    *needs_audio_rebuild = true;
                } else {
                    *needs_audio_notify = true;
                }
            }
        }

        // 展开状态 / lane 数可能刚变：重建行布局并重新 clamp。
        row_layout = build_row_layout(&doc.edit, &doc.data.model);
        arr_view.clamp_scroll(
            gpu_rect.width(),
            gpu_rect.height(),
            total_ticks,
            row_layout.total_rows(),
        );

        arr_view.base.scroll_y = arr_view.base.track_panel_scroll_y;
    });

    // ── Arrangement GPU view (below ruler) ──
    let gpu_size = gpu_rect.size();
    // AM lane 交互产生的编辑：GPU scope 内借用 doc.edit/model，出 scope 后统一应用。
    let mut am_edits: Vec<yinhe_types::AutomationEdit> = Vec::new();
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
            tracks: &model.tracks,
            tempo_lane: &model.conductor.tempo,
            conductor_track_idx: doc.edit.conductor_track_idx,
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
            arr_am_views: &mut doc.edit.arr_am_views,
            am_edits: &mut am_edits,
            right_tab,
        };
        view_ui::show(
            ui,
            gpu_size,
            arr_renderer,
            arr_render_ctx,
            arr_view,
            &row_layout,
            data,
            &mut edit,
            &mut cfg,
        );
    });

    // 应用 AR 自动化 lane 的编辑（与 PR 的 handle_automation_edits 同一路径）
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

    // ── Horizontal scrollbar (right of track panel, below GPU content) ──
    //    让出右下角 SCROLLBAR_W × SCROLLBAR_H 给垂直滚动条+水平滚动条的交叠区
    {
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
                gpu_rect.max.y,
            ),
            egui::pos2(gpu_rect.max.x, arr_rect.max.y),
        );

        // 右下角角落：横纵滚动条交叠区（SCROLLBAR_W × SCROLLBAR_H）
        let corner_rect = egui::Rect::from_min_max(gpu_rect.max, arr_rect.max);
        ui.painter()
            .rect_filled(corner_rect, 0.0, crate::theme::track_bg());

        let sb_drag_dy = crate::widgets::scrollbar::show(
            ui,
            sb_rect,
            gpu_rect.width(),
            &mut arr_view.base.scroll_x,
            &mut arr_view.base.pixels_per_tick,
            total_ticks,
            &mut arr_view.base.dirty,
        );
        // 水平滚动条：thumb 拖 = 平移（x）+ 垂直位移 → x 轴缩放
        // 方向：上拖 = 放大，下拖 = 缩小
        if sb_drag_dy != 0.0 {
            let factor = 1.0 - sb_drag_dy * 0.005;
            let anchor_x = sb_rect.center().x - arr_rect.min.x;
            arr_view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：水平滚动条上滚轮 = x 轴缩放（锚定滚动条中心 x）
        // 方向：上滚 = 放大，下滚 = 缩小
        if crate::view_interaction::pointer_hits(ui, sb_rect) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                let anchor_x = sb_rect.center().x - arr_rect.min.x;
                arr_view.zoom_around_x(anchor_x, factor);
                ui.ctx().request_repaint();
            }
        }
    }

    // ── Vertical scrollbar (right of GPU content, full AR height minus ruler) ──
    //    像素空间：num_cells = num_tracks，cell_size = lane height (track_panel_row_height)
    {
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + RULER_H),
            egui::pos2(arr_rect.max.x, gpu_rect.max.y),
        );
        let vsb_drag_dx = ui
            .push_id("arr_vscroll", |ui| {
                crate::widgets::scrollbar::show_vertical(
                    ui,
                    vsb_rect,
                    gpu_rect.height(),
                    &mut arr_view.base.scroll_y,
                    &mut arr_view.base.track_panel_row_height,
                    row_layout.total_rows(),
                    16.0,
                    120.0,
                    &mut arr_view.base.dirty,
                )
            })
            .inner;
        // 垂直滚动条：thumb 拖 = 平移（y）+ 水平位移 → y 轴缩放（轨道行高）
        // 方向：左拖 = 放大，右拖 = 缩小
        if vsb_drag_dx != 0.0 {
            let factor = 1.0 - vsb_drag_dx * 0.005;
            let anchor_y = vsb_rect.center().y - arr_rect.min.y;
            arr_view.zoom_lane_height(anchor_y, factor);
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：垂直滚动条上滚轮 = y 轴缩放（锚定滚动条中心 y）
        // 方向：上滚 = 放大，下滚 = 缩小
        if crate::view_interaction::pointer_hits(ui, vsb_rect) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                let anchor_y = vsb_rect.center().y - arr_rect.min.y;
                arr_view.zoom_lane_height(anchor_y, factor);
                ui.ctx().request_repaint();
            }
        }
    }

    // ── AR quantize button in the top-left corner (left of ruler, above track panel) ──
    // 与 PR 共用 quantize_button 组件（角落矩形 + 弹窗逻辑一致）。
    let pending_quantize = crate::widgets::quantize_button::show(
        ui,
        crate::widgets::quantize_button::QuantizeBtnCtx {
            corner_rect: egui::Rect::from_min_size(
                egui::pos2(arr_rect.min.x, arr_rect.min.y),
                egui::vec2(tp_w, RULER_H),
            ),
            id_salt: "arr_quantize_btn",
            ppq: doc.data.model.meta.ppq,
            quantize: doc.edit.quantize_arrange,
        },
    );

    // ── "+" track add button in the corner (below track panel, left of scrollbar) ──
    {
        let corner_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x,
                arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
            ),
            egui::pos2(arr_rect.min.x + tp_w, arr_rect.max.y),
        );
        // 角落背景：track panel 下方、水平滚动条左侧（未来可放其他控件）
        ui.painter()
            .rect_filled(corner_rect, 0.0, crate::theme::track_bg());

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
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_muted()
        };
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_ADD.codepoint,
            egui::FontId::new(crate::theme::ICON_BTN_FONT, ICON_ADD.font_family()),
            icon_color,
        );

        if btn_resp.clicked() {
            // 弹出新建音轨对话框：写 ctx memory 标志，由 dialog_dispatch
            // 每帧检测并打开独立 viewport；确认后批量创建并 teardown 音频
            // （方案 A 同原 add_track 路径，在 dialog_dispatch 内完成）。
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new(crate::dialogs::new_track::OPEN_REQUEST_ID),
                    true,
                )
            });
        }
    }

    // ── Vertical splitter handle (drawn last so it sits on top) ──
    let v_handle = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w, arr_rect.min.y),
        egui::pos2(
            arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
            arr_rect.max.y,
        ),
    );
    let v_resp = crate::widgets::split_handle::vertical(ui, "__v_split__", v_handle);
    if v_resp.dragged() {
        *layout.transport_panel_width =
            (*layout.transport_panel_width + v_resp.drag_delta().x).clamp(60.0, arr_total_w - 60.0);
    }
    if v_resp.double_clicked() {
        // 双击分割线 → 还原走带面板默认宽度
        *layout.transport_panel_width = LayoutSettings::default().transport_panel_width;
    }
    if v_resp.drag_stopped() || v_resp.double_clicked() {
        *layout.drag_ended = true;
    }

    // ── 状态栏讲解行：走带悬停提示（位置 + 音轨号；有选框时优先显示选框统计）──
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
        let model = &*doc.data.model;
        let tpb = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let lh = arr_view.lane_height();
        let scroll_y = arr_view.base.scroll_y;
        // hover_pos 是全局坐标，需先减去 tp_rect.min.y（title bar + transport
        // bar + ruler 的高度），否则音轨号会整体偏大。
        // 行布局命中：AM 子行附带 target 名（如「CC 007」）。
        let track_str = |track: usize| t!("hint.track", n = format!("{:03}", track)).to_string();
        let hover_desc = |y: f32| {
            let my = y - tp_rect.min.y + scroll_y;
            match row_layout.hit_at_music_y(my, lh) {
                Some(yinhe_types::ArRow::Automation(t, sub)) => {
                    match doc
                        .data
                        .model
                        .tracks
                        .get(t)
                        .and_then(|tr| tr.automation_lanes.get(sub))
                    {
                        Some(lane) => {
                            format!("{} · {}", track_str(t), am_lanes::lane_label(&lane.target))
                        }
                        None => track_str(t),
                    }
                }
                Some(yinhe_types::ArRow::Track(t)) => track_str(t),
                None => track_str(num_tracks.saturating_sub(1)),
            }
        };
        // 本视图有选框 → 讲解行显示选框统计（参考 info panel）
        let sel_text = if !doc.edit.arr_sel_rect.is_empty()
            && let Some(sh) = sel_hint
        {
            Some(t!("hint.sel_notes", n = sh.count, span = &sh.span).to_string())
        } else {
            None
        };
        if music_rect.contains(pos) {
            let tick = arr_view.x_to_tick(pos.x - gpu_rect.min.x).max(0.0);
            let pos_str =
                format_tick_bar_beat_with_time_sig(tick, tpb, sig_events, def_num, def_den);
            *status_hint = Some(if let Some(s) = sel_text {
                s
            } else {
                format!("{} {}", pos_str, hover_desc(pos.y))
            });
        } else if tp_rect.contains(pos) {
            *status_hint = Some(if let Some(s) = sel_text {
                s
            } else {
                hover_desc(pos.y)
            });
        } else if arr_rect.contains(pos) {
            // 走带视图内但不在内容区（标尺/滚动条）→ 清空
            *status_hint = None;
        }
    }

    pending_quantize
}
