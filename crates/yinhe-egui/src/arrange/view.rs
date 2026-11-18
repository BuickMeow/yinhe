use super::layout::build_row_layout;
use super::*;

/// 编排视图协调器：聚合 doc/布局/渲染/配置/编辑原料/信号六个职责面的输入，
/// 内部按只读数据、可变编辑状态、视图配置三轴分发（见 ArrangeData 等）。
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    layout: ArrangeLayout<'_>,
    arr_renderer: &mut yinhe_wgpu::InstanceRenderer,
    arr_render_ctx: &mut RenderContext,
    mut cfg: ArrangeViewCfg<'_>,
    last_cursor_tick: &mut Option<f64>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    audio_library: &crate::app::audio_library::AudioLibrary,
    request_pianoroll: &mut bool,
    selection_anchor: &mut Option<u16>,
    arr_drag_delta: &mut Option<ArrDragDelta>,
    arr_eraser_rect: &mut Option<ArrSelRect>,
    arr_scale_span: &mut Option<u64>,
    arr_riff_steps: &mut Option<(i64, i64)>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    // AM 锚点右键打开信息面板（与 PR 自动化面板一致）。
    right_tab: &mut Option<crate::right_panel::RightTab>,
    // 音轨结构变化（add/remove track）需要 teardown + 重建音频引擎。
    // 由调用方 layout.rs 读取后调 `App::teardown_audio()`，下一帧
    // `rebuild_audio_if_needed` 会用新 model 重新 spawn 引擎和 ChannelLayout。
    needs_audio_rebuild: &mut bool,
    // 自动化内容变化（AM lane 增删 / 事件编辑）需要 notify_audio_model_changed。
    needs_audio_notify: &mut bool,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
    // 「添加自动化」：请求打开设备参数选择窗口（由调用方落地）。
    automation_picker_req: &mut Option<usize>,
    // 每轨电平表显示值 `((L,R), (holdL,holdR))`（仅播放时非零；索引 = 音轨）。
    track_meters: &[crate::widgets::meter::StereoMeter],
    // 双击固定 Bus 行：请求打开设备链（调用方落地）。
    dock_req: &mut Option<crate::chrome::device_chain::DockContext>,
    // 音乐区右键菜单动作（调用方 `App` 落地）。
    menu_actions: &mut Vec<context_menu::TimelineMenuAction>,
) {
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

    // 全局事件行固定在标尺之下、音轨之上，不随 track scroll_y 滚动。
    let event_rows_h = doc.edit.event_rows_ar.visible_height();
    let content_top = arr_rect.min.y + RULER_H + event_rows_h;

    // ── Track panel: starts after ruler + event rows, ends at scrollbar top so rows align with GPU lanes ──
    let tp_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, content_top),
        egui::pos2(
            arr_rect.min.x + tp_w,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );

    // ── GPU area: 全宽纹理（含轨道面板列，left_panel_width 同步为面板宽 +
    //    分屏条宽，音符坐标以全宽左缘为原点，视口左边界由 shader clamp 排除）。
    //    y 方向：shifted down by RULER_H + event rows, shifted up by SCROLLBAR_H,
    //    x 方向：shifted left by SCROLLBAR_W to leave room for the vertical scrollbar ──
    let gpu_rect = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x, content_top),
        egui::pos2(
            arr_rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W,
            arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );
    // 音乐区（旧 gpu_rect）：纹理内的可见区域，交互命中与绘制 clip 都限制在这里。
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(
            arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
            content_top,
        ),
        gpu_rect.max,
    );

    // 轨道面板宽度同步进视图坐标模型：left_panel_width = 面板宽 + 分屏条宽
    // （纹理左缘到音乐区左缘的距离），tick_to_x/x_to_tick/clamp_scroll 全部基于它。
    doc.edit.arrange_view.base.left_panel_width = tp_w + crate::theme::SPLIT_HANDLE_W;

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
    let conductor_idx = doc.edit.track_cache.conductor_idx;
    let track_mixer_lanes: Vec<Vec<(yinhe_types::MixerNodeId, Vec<yinhe_types::AutomationLane>)>> =
        doc.data
            .model
            .tracks
            .iter()
            .map(|track| am_lanes::track_mixer_lanes(track, &doc.mixer))
            .collect();
    let lc_labels: std::collections::HashMap<yinhe_types::LogicalChannelId, String> = doc
        .mixer
        .lc_display_map()
        .into_iter()
        .map(|(id, n)| (id, crate::mix::lc_label(n)))
        .collect();
    let mut row_layout = build_row_layout(
        &doc.edit,
        &doc.data.model,
        num_tracks,
        conductor_idx,
        &track_mixer_lanes,
    );
    // 清理已删除 lane 的残留视图状态（undo 删除 lane 后）
    let lane_exists = |t: u16, target: &yinhe_types::AutomationTarget| {
        doc.data
            .model
            .tracks
            .get(t as usize)
            .is_some_and(|tr| tr.automation_lanes.iter().any(|l| &l.target == target))
            || track_mixer_lanes.get(t as usize).is_some_and(|lanes| {
                lanes
                    .iter()
                    .any(|(_, l)| l.iter().any(|x| &x.target == target))
            })
    };
    doc.edit
        .arr_am_views
        .retain(|&(t, ref target), _| lane_exists(t, target));
    // AM M/S 试听/选中键同样只保留仍存在的 lane（删除 lane / undo 后兜底）。
    doc.edit
        .arr_am_ms
        .retain(|&(t, ref target), _| lane_exists(t, target));
    doc.edit
        .arr_am_selected
        .retain(|&(t, ref target)| lane_exists(t, target));
    doc.edit.arrange_view.clamp_scroll(
        gpu_rect.width(),
        gpu_rect.height(),
        total_ticks,
        row_layout.total_rows() + 1,
    );

    draw_ruler_band(ui, doc, arr_rect, gpu_rect, tp_w);

    draw_event_rows_band(ui, doc, arr_rect, gpu_rect, content_top);

    handle_blank_double_click(
        ui,
        doc,
        &row_layout,
        music_rect,
        gpu_rect,
        cfg.clip_enabled,
        request_pianoroll,
    );

    draw_track_panel(
        ui,
        doc,
        tp_rect,
        gpu_rect,
        total_ticks,
        &mut row_layout,
        num_tracks,
        conductor_idx,
        &track_mixer_lanes,
        &lc_labels,
        track_meters,
        request_pianoroll,
        info_content,
        selection_anchor,
        audio,
        automation_picker_req,
        right_tab,
        dock_req,
        needs_audio_rebuild,
        needs_audio_notify,
    );

    let (mut am_edits, mut audio_commands, mut clip_commands) = draw_arrangement_view(
        ui,
        doc,
        gpu_rect,
        &row_layout,
        arr_renderer,
        arr_render_ctx,
        &mut cfg,
        arr_drag_delta,
        arr_eraser_rect,
        arr_scale_span,
        arr_riff_steps,
        selection_anchor,
        info_content,
        right_tab,
        menu_actions,
        &track_mixer_lanes,
        audio_library,
        total_ticks,
        num_tracks,
    );

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

    // 应用 MIDI Clip 编辑（移动/循环扩展）。
    if !clip_commands.is_empty() {
        apply_clip_edit_commands(doc, std::mem::take(&mut clip_commands));
        *needs_audio_notify = true;
    }

    // 应用音频片段编辑（移动/裁剪/淡入淡出/分割/复制/删除/反向/归一化）。
    if !audio_commands.is_empty() {
        apply_audio_edit_commands(doc, audio_library, std::mem::take(&mut audio_commands));
        *needs_audio_notify = true;
    }

    draw_horizontal_scrollbar(
        ui,
        doc,
        arr_rect,
        gpu_rect,
        music_rect,
        tp_w,
        total_ticks,
        &row_layout,
    );
    draw_vertical_scrollbar(ui, doc, arr_rect, gpu_rect, content_top, &row_layout);

    draw_corner_panels(ui, doc, arr_rect, tp_w);

    draw_vertical_splitter(ui, layout, arr_rect, tp_w, arr_total_w);

    draw_hover_hint(
        ui,
        doc,
        arr_rect,
        tp_rect,
        gpu_rect,
        music_rect,
        &row_layout,
        num_tracks,
        tp_w,
        sel_hint,
    );
}

/// 顶部时间标尺（右边界对齐 GPU 区，右侧角落补轨道底色）。
fn draw_ruler_band(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_rect: egui::Rect,
    gpu_rect: egui::Rect,
    tp_w: f32,
) {
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
        // 量化档位解析：Auto（自适应）按当前 AR 缩放出具体网格。
        let ar_quantize = doc.edit.quantize_arrange.resolved_with_time_sig(
            doc.edit.arrange_view.base.pixels_per_tick,
            tpb,
            def_num,
            def_den,
            false,
        );
        let outcome = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            &mut doc.edit.arrange_view,
            tpb,
            def_num,
            def_den,
            sig_events,
            |tick| {
                crate::view_interaction::snap_tick(
                    tick,
                    ar_quantize,
                    tpb,
                    Some((tpb, def_num, def_den, sig_events)),
                )
            },
            "arrange_ruler",
            &mut doc.edit.cursor_tick,
        );
        // 点击/拖动时间标尺跳转位置时，取消已选择的选框（含框选与全选）。
        if outcome.jumped {
            doc.edit.selected.clear_rects();
            doc.edit.arr_sel_rect.clear();
        }
    }
}

/// 标尺下的全局事件行（marker / 拍号 / 调号 / 和弦 / 歌词）。
fn draw_event_rows_band(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_rect: egui::Rect,
    gpu_rect: egui::Rect,
    content_top: f32,
) {
    // ── 全局事件行（固定，不随 track scroll_y）──
    {
        let event_full = egui::Rect::from_min_max(
            egui::pos2(arr_rect.min.x, arr_rect.min.y + RULER_H),
            egui::pos2(gpu_rect.max.x, content_top),
        );
        // 右侧滚动条占位（事件行不覆盖滚动条列）
        let row_corner = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + RULER_H),
            egui::pos2(arr_rect.max.x, content_top),
        );
        ui.painter()
            .rect_filled(row_corner, 0.0, crate::theme::track_bg());

        let model = &*doc.data.model;
        let tpb = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let ar_quantize = doc.edit.quantize_arrange.resolved_with_time_sig(
            doc.edit.arrange_view.base.pixels_per_tick,
            tpb,
            def_num,
            def_den,
            false,
        );
        let rows = crate::widgets::event_rows::show(
            ui,
            &mut doc.edit.event_rows_ar,
            &mut doc.edit.playback,
            crate::widgets::event_rows::EventRowsData {
                markers: &model.conductor.markers,
                time_sig: &model.conductor.time_sig,
                key_sig: &model.conductor.key_sig,
                chords: &model.conductor.chord,
                lyrics: &model.conductor.lyrics,
            },
            &mut doc.edit.arrange_view,
            event_full,
            tpb,
            def_num,
            def_den,
            sig_events,
            &|tick| {
                crate::view_interaction::snap_tick(
                    tick,
                    ar_quantize,
                    tpb,
                    Some((tpb, def_num, def_den, sig_events)),
                )
            },
            "arrange_event_rows",
            &mut doc.edit.cursor_tick,
        );
        if rows.jumped {
            doc.edit.selected.clear_rects();
            doc.edit.arr_sel_rect.clear();
        }
        if let Some(edit) = rows.marker_edit {
            crate::marker_ops::apply_marker_edit(doc, edit);
        }
    }
}

/// AR 空白处双击：创建 1 小节空 Clip；双击 Clip 标题栏进入 CLIP 模式。
fn handle_blank_double_click(
    ui: &mut egui::Ui,
    doc: &mut Document,
    row_layout: &yinhe_types::ArRowLayout,
    music_rect: egui::Rect,
    gpu_rect: egui::Rect,
    clip_enabled: bool,
    request_pianoroll: &mut bool,
) {
    // ── AR 空白处双击：在点击位置创建 1 小节空 Clip ──
    if clip_enabled
        && ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        })
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && music_rect.contains(pos)
        && let Some(yinhe_types::ArRow::Track(track_idx)) = row_layout.hit_at_music_y(
            pos.y - music_rect.min.y + doc.edit.arrange_view.base.scroll_y,
            doc.edit.arrange_view.lane_height(),
        )
    {
        let tick = doc
            .edit
            .arrange_view
            .x_to_tick(pos.x - gpu_rect.min.x)
            .max(0.0) as u32;
        let ppq = doc.data.model.meta.ppq;
        let (num, den) = doc.data.model.tempo_map.time_sig_at_tick(tick);
        let bar = yinhe_types::measure_ticks(ppq, num, den).max(1);
        let start = (tick / bar) * bar;
        let occupied = doc.data.model.tracks.get(track_idx).is_some_and(|t| {
            t.clips
                .iter()
                .any(|c| c.start_tick <= tick && tick < c.end_tick)
        });
        // 只有 Clip 标题栏可以进入 CLIP；主体留给音符选择/框选。
        let lane_h = doc.edit.arrange_view.lane_height();
        let music_y = pos.y - music_rect.min.y + doc.edit.arrange_view.base.scroll_y;
        let track_y = row_layout.track_y(track_idx, lane_h);
        let header_h = view_ui::render::MIDI_CLIP_HEADER_H
            .min(lane_h - 2.0)
            .max(0.0);
        let on_header = music_y >= track_y && music_y <= track_y + header_h;
        if !occupied {
            let before = doc.capture_snapshot();
            if let Some(action) = doc.create_empty_clip(track_idx as u16, start, bar) {
                doc.push_undo(action, "undo.move_notes", before);
            }
        } else if on_header {
            // 双击已有 Clip 标题栏：进入 CLIP 模式并切到 PR。
            let hit = doc.data.model.tracks[track_idx]
                .clips
                .iter()
                .find(|c| c.start_tick <= tick && tick < c.end_tick)
                .map(|c| c.start_tick);
            if let Some(clip_start) = hit {
                doc.edit.pr_clip_mode = true;
                doc.edit.show_controller_panels = false;
                doc.edit.cursor_tick = Some(clip_start as f64);
                *request_pianoroll = true;
            }
        }
    }
}

/// 底部横向滚动条 + 右下角缩放抓手。
#[allow(clippy::too_many_arguments)] // 视图几何参数透传
fn draw_horizontal_scrollbar(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_rect: egui::Rect,
    gpu_rect: egui::Rect,
    music_rect: egui::Rect,
    tp_w: f32,
    total_ticks: f64,
    row_layout: &yinhe_types::ArRowLayout,
) {
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

        // 右下角角落：横纵滚动条交叠区（SCROLLBAR_W × SCROLLBAR_H）——缩放抓手。
        // 拖动同时缩放横竖两轴；双击横竖一起 fit（对齐 FL）。
        let corner_rect = egui::Rect::from_min_max(gpu_rect.max, arr_rect.max);
        {
            let resp = ui.interact(
                corner_rect,
                ui.scope_id().with("arr_corner_zoom"),
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
                doc.edit.arrange_view.zoom_around_x(anchor_x, 1.0 + d.x * k);
                doc.edit
                    .arrange_view
                    .zoom_lane_height(anchor_y, 1.0 + d.y * k);
                ui.ctx().request_repaint();
            } else if crate::widgets::scrollbar::double_clicked_in(ui, corner_rect) {
                doc.edit
                    .arrange_view
                    .fit_time(music_rect.width(), total_ticks);
                doc.edit
                    .arrange_view
                    .fit_lane_height(gpu_rect.height(), row_layout.total_rows());
                ui.ctx().request_repaint();
            }
            if crate::view_interaction::pointer_hits(ui, corner_rect) {
                crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.corner_zoom"));
            }
        }

        let preview = crate::widgets::scrollbar::DensityPreview {
            density: doc.density.view(),
            track_colors: &doc.edit.track_cache.colors,
        };
        let sb_drag_dy = crate::widgets::scrollbar::show(
            ui,
            sb_rect,
            music_rect.width(),
            &mut doc.edit.arrange_view.base.scroll_x,
            &mut doc.edit.arrange_view.base.pixels_per_tick,
            total_ticks,
            Some(&preview),
            doc.edit.cursor_tick,
            &mut doc.edit.arrange_view.base.dirty,
            yinhe_types::Orientation::Horizontal,
        );
        if crate::view_interaction::pointer_hits(ui, sb_rect) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.hscroll"));
        }
        // 水平滚动条：thumb 拖 = 平移（x）+ 垂直位移 → x 轴缩放
        // 方向：上拖 = 放大，下拖 = 缩小
        if sb_drag_dy != 0.0 {
            let factor = 1.0 - sb_drag_dy * 0.005;
            let anchor_x = sb_rect.center().x - arr_rect.min.x;
            doc.edit.arrange_view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }
        if crate::widgets::scrollbar::double_clicked_in(ui, sb_rect) {
            doc.edit
                .arrange_view
                .fit_time(music_rect.width(), total_ticks);
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：水平滚动条上滚轮 = x 轴缩放（锚定滚动条中心 x）
        // 方向：上滚 = 放大，下滚 = 缩小
        if crate::view_interaction::pointer_hits(ui, sb_rect) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                let anchor_x = sb_rect.center().x - arr_rect.min.x;
                doc.edit.arrange_view.zoom_around_x(anchor_x, factor);
                ui.ctx().request_repaint();
            }
        }
    }
}

/// 右侧纵向滚动条（thumb 拖 = 平移 + 横向位移缩放行高）。
fn draw_vertical_scrollbar(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_rect: egui::Rect,
    gpu_rect: egui::Rect,
    content_top: f32,
    row_layout: &yinhe_types::ArRowLayout,
) {
    // ── Vertical scrollbar (right of GPU content, full AR height minus ruler) ──
    //    像素空间：num_cells = num_tracks，cell_size = lane height (track_panel_row_height)
    {
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(gpu_rect.max.x, content_top),
            egui::pos2(arr_rect.max.x, gpu_rect.max.y),
        );
        let vsb_drag_dx = ui
            .push_id("arr_vscroll", |ui| {
                crate::widgets::scrollbar::show_vertical(
                    ui,
                    vsb_rect,
                    gpu_rect.height(),
                    &mut doc.edit.arrange_view.base.scroll_y,
                    &mut doc.edit.arrange_view.base.track_panel_row_height,
                    row_layout.total_rows() as f32 + 1.0,
                    yinhe_types::LANE_HEIGHT_MIN,
                    yinhe_types::LANE_HEIGHT_MAX,
                    Some(yinhe_types::LANE_HEIGHT_STEP),
                    &mut doc.edit.arrange_view.base.dirty,
                    yinhe_types::Orientation::Horizontal,
                )
            })
            .inner;
        if crate::view_interaction::pointer_hits(ui, vsb_rect) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.vscroll"));
        }
        // 垂直滚动条：thumb 拖 = 平移（y）+ 水平位移 → y 轴缩放（轨道行高）
        // 方向：左拖 = 放大，右拖 = 缩小
        if vsb_drag_dx != 0.0 {
            let factor = 1.0 - vsb_drag_dx * 0.005;
            let anchor_y = vsb_rect.center().y - arr_rect.min.y;
            doc.edit.arrange_view.zoom_lane_height(anchor_y, factor);
            ui.ctx().request_repaint();
        }
        if crate::widgets::scrollbar::double_clicked_in(ui, vsb_rect) {
            doc.edit
                .arrange_view
                .fit_lane_height(gpu_rect.height(), row_layout.total_rows());
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：垂直滚动条上滚轮 = y 轴缩放（锚定滚动条中心 y）
        // 方向：上滚 = 放大，下滚 = 缩小
        if crate::view_interaction::pointer_hits(ui, vsb_rect) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                let anchor_y = vsb_rect.center().y - arr_rect.min.y;
                doc.edit.arrange_view.zoom_lane_height(anchor_y, factor);
                ui.ctx().request_repaint();
            }
        }
    }
}

/// AR 音轨面板：缩放、并行数组补齐、行绘制与轨道动作落地。
///
/// 行布局可能因展开/删除 lane 变化，函数结束时重建并重新 clamp
/// （`row_layout` 原地更新）。
#[allow(clippy::too_many_arguments)] // 视图上下文透传
fn draw_track_panel(
    ui: &mut egui::Ui,
    doc: &mut Document,
    tp_rect: egui::Rect,
    gpu_rect: egui::Rect,
    total_ticks: f64,
    row_layout: &mut yinhe_types::ArRowLayout,
    num_tracks: usize,
    conductor_idx: Option<u16>,
    track_mixer_lanes: &[Vec<(yinhe_types::MixerNodeId, Vec<yinhe_types::AutomationLane>)>],
    lc_labels: &std::collections::HashMap<yinhe_types::LogicalChannelId, String>,
    track_meters: &[crate::widgets::meter::StereoMeter],
    request_pianoroll: &mut bool,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    selection_anchor: &mut Option<u16>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    automation_picker_req: &mut Option<usize>,
    right_tab: &mut Option<crate::right_panel::RightTab>,
    dock_req: &mut Option<crate::chrome::device_chain::DockContext>,
    needs_audio_rebuild: &mut bool,
    needs_audio_notify: &mut bool,
) {
    // 只在本面板绘制期间裁剪；返回前恢复，否则后续 GPU 主区继承 tp_rect，
    // 与 music_rect 求交后变成空矩形，整块编曲区（wgpu 纹理）被裁掉。
    let prev_clip = ui.clip_rect();
    ui.set_clip_rect(tp_rect);
    ui.painter()
        .rect_filled(tp_rect, 0.0, crate::theme::app_bg());

    doc.edit.arrange_view.base.track_panel_scroll_y = doc.edit.arrange_view.base.scroll_y;

    let zoom_delta = ui.input(|i| i.zoom_delta());
    if (zoom_delta - 1.0).abs() > 0.001
        && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
        && tp_rect.contains(hover)
    {
        let pointer_y = hover.y - tp_rect.min.y;
        let prev_h = doc.edit.arrange_view.lane_height();
        // 与滚轮/滚动条共用同一套累积器：细微输入不跳档。
        doc.edit
            .arrange_view
            .zoom_lane_height(pointer_y, zoom_delta);
        if doc.edit.arrange_view.lane_height() != prev_h {
            // zoom_lane_height 锚定的是 base.scroll_y，写回面板滚动值
            // （面板 scope 结束处会把 track_panel_scroll_y 同步回 scroll_y）。
            doc.edit.arrange_view.base.track_panel_scroll_y = doc.edit.arrange_view.base.scroll_y;
        }
    }

    // Ensure parallel arrays are correctly sized (track count may have grown).
    let n = doc.edit.track_cache.info.len();
    if doc.edit.track_pianoroll_visible.len() < n {
        doc.edit.track_pianoroll_visible.resize(n, true);
    }
    if doc.edit.track_overrides.len() < n {
        doc.edit
            .track_overrides
            .resize(n, yinhe_editor_core::document::TrackOverride::default());
    }
    if doc.edit.track_cache.colors.len() < n {
        doc.edit.track_cache.ensure_colors_len(&doc.data.model, n);
    }

    // 音轨面板的布局宽高由 tp_rect 决定（track_panel::show 读 ui.max_rect()）。
    // 不套子 UI 时它会拿到整个编曲区的 max_rect，行内右侧按钮/名称被排到面板
    // 之外，再被 clip 裁掉，看起来就是面板空白、内容跑到左上角。
    let (audio_dirty, am_ms_dirty, track_actions) = ui
        .scope_builder(egui::UiBuilder::new().max_rect(tp_rect), |ui| {
            track_panel::show(
                ui,
                &doc.edit.track_cache.info,
                &doc.edit.track_visible,
                &mut doc.edit.track_overrides,
                &mut doc.edit.track_selected,
                selection_anchor,
                doc.edit.track_cache.conductor_idx,
                &doc.edit.track_cache.colors,
                &mut doc.edit.arrange_view.base.track_panel_row_height,
                &mut doc.edit.arrange_view.base.track_panel_scroll_y,
                request_pianoroll,
                info_content,
                row_layout,
                &doc.data.model.tracks,
                lc_labels,
                &mut doc.edit.arr_am_expanded,
                &mut doc.edit.arr_am_selected,
                &mut doc.edit.arr_am_ms,
                track_meters,
                track_mixer_lanes,
                doc.edit.cursor_tick,
                &doc.data.model.conductor.tempo,
            )
        })
        .inner;

    if audio_dirty {
        // Track solo 是内容门：进入 Track solo 时清空 Channel solo，
        // 避免两层 solo 叠加。清空后把受影响的通道参数推给引擎。
        if doc.edit.track_overrides.iter().any(|o| o.soloed) {
            for node in doc.clear_channel_solos() {
                let params = doc.mixer.node_strip(node);
                if let Some(audio) = audio {
                    audio
                        .handle
                        .send(yinhe_audio::AudioCommand::SetNodeStrip { node, params });
                }
            }
        }
        crate::right_panel::info_panel::send_skip_tracks(doc, audio);
    }
    // AM lane M/S 试听开关：只换动态掩码 + 异步 chase（不重建模型、不 seek），
    // 与轨道 M/S 的 SkipTracks 同机制——播放中切换不跳跃、不卡顿。
    if am_ms_dirty && let Some(audio) = audio {
        audio.set_am_ms(std::sync::Arc::new(doc.edit.arr_am_ms.clone()));
    }

    actions::apply_track_actions(
        doc,
        track_actions,
        actions::ActionCtx {
            automation_picker_req,
            info_content,
            right_tab,
            dock_req,
            needs_audio_rebuild,
            needs_audio_notify,
        },
    );

    // 展开状态 / lane 数可能刚变：重建行布局并重新 clamp。
    *row_layout = build_row_layout(
        &doc.edit,
        &doc.data.model,
        num_tracks,
        conductor_idx,
        track_mixer_lanes,
    );
    doc.edit.arrange_view.clamp_scroll(
        gpu_rect.width(),
        gpu_rect.height(),
        total_ticks,
        row_layout.total_rows() + 1,
    );

    doc.edit.arrange_view.base.scroll_y = doc.edit.arrange_view.base.track_panel_scroll_y;
    ui.set_clip_rect(prev_clip);
}

/// AR 主区（GPU 纹理 + 片段/自动化交互），返回出 scope 后要应用的编辑命令。
#[allow(clippy::too_many_arguments)] // 视图上下文透传
fn draw_arrangement_view<'a>(
    ui: &mut egui::Ui,
    doc: &mut Document,
    gpu_rect: egui::Rect,
    row_layout: &yinhe_types::ArRowLayout,
    arr_renderer: &mut yinhe_wgpu::InstanceRenderer,
    arr_render_ctx: &mut RenderContext,
    cfg: &mut ArrangeViewCfg<'a>,
    arr_drag_delta: &mut Option<ArrDragDelta>,
    arr_eraser_rect: &mut Option<ArrSelRect>,
    arr_scale_span: &mut Option<u64>,
    arr_riff_steps: &mut Option<(i64, i64)>,
    selection_anchor: &mut Option<u16>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    right_tab: &mut Option<crate::right_panel::RightTab>,
    menu_actions: &mut Vec<context_menu::TimelineMenuAction>,
    track_mixer_lanes: &[Vec<(yinhe_types::MixerNodeId, Vec<yinhe_types::AutomationLane>)>],
    audio_library: &crate::app::audio_library::AudioLibrary,
    total_ticks: f64,
    num_tracks: usize,
) -> (
    Vec<yinhe_types::AutomationEdit>,
    Vec<view_ui::AudioEditCmd>,
    Vec<view_ui::ClipEditCmd>,
) {
    // ── Arrangement GPU view (below ruler) ──
    let gpu_size = gpu_rect.size();
    // AM lane 交互产生的编辑：GPU scope 内借用 doc.edit/model，出 scope 后统一应用。
    let mut am_edits: Vec<yinhe_types::AutomationEdit> = Vec::new();
    // 片段交互命令：同一帧出 scope 后应用（含 undo）。
    let mut audio_commands: Vec<view_ui::AudioEditCmd> = Vec::new();
    let mut clip_commands: Vec<view_ui::ClipEditCmd> = Vec::new();
    // 折叠文件夹的子轨对 GPU/命中测试不可见；与用户手动 eye 显隐合并。
    let folder_visible = doc.data.model.folder_visible_mask();
    let effective_visible: Vec<bool> = doc
        .edit
        .track_visible
        .iter()
        .zip(folder_visible.iter())
        .map(|(user, folder)| *user && *folder)
        .collect();

    ui.scope_builder(egui::UiBuilder::new().max_rect(gpu_rect), |ui| {
        let model = &*doc.data.model;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let data = ArrangeData {
            midi: Some(model as &dyn yinhe_types::NoteSource),
            track_visible: &effective_visible,
            track_colors: &doc.edit.track_cache.colors,
            track_info: &doc.edit.track_cache.info,
            quantize: doc.edit.quantize_arrange.resolved_with_time_sig(
                doc.edit.arrange_view.base.pixels_per_tick,
                model.meta.ppq,
                def_num,
                def_den,
                false,
            ),
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
            track_mixer_lanes,
            conductor_track_idx: doc.edit.track_cache.conductor_idx,
            tempo_map: &model.tempo_map,
            audio_library,
            audio_sources: &model.audio_sources,
            clip_enabled: cfg.clip_enabled,
        };
        let mut edit = ArrangeEdit {
            selected: &mut doc.edit.selected,
            cursor_tick: &mut doc.edit.cursor_tick,
            arr_sel_rect: &mut doc.edit.arr_sel_rect,
            arr_drag_delta,
            arr_eraser_rect,
            arr_scale_span,
            arr_riff_steps,
            track_selected: &mut doc.edit.track_selected,
            selection_anchor,
            info_content,
            arr_am_views: &mut doc.edit.arr_am_views,
            am_edits: &mut am_edits,
            right_tab,
            selected_audio_clips: &mut doc.edit.selected_audio_clips,
            audio_commands: &mut audio_commands,
            clip_commands: &mut clip_commands,
            menu_actions,
        };
        view_ui::show(
            ui,
            gpu_size,
            arr_renderer,
            arr_render_ctx,
            &mut doc.edit.arrange_view,
            row_layout,
            data,
            &mut edit,
            cfg,
        );
    });
    (am_edits, audio_commands, clip_commands)
}

/// 左上角量化控件与该角落、底部角落的背景。
fn draw_corner_panels(ui: &mut egui::Ui, doc: &mut Document, arr_rect: egui::Rect, tp_w: f32) {
    // ── 左上角角落（音轨面板上方、标尺左侧）：仅铺背景 ──
    // （量化按钮已移至 transport bar 时间码左侧，按聚焦视图显示 AR/PR 量化）
    let events_corner = egui::Rect::from_min_size(
        egui::pos2(arr_rect.min.x, arr_rect.min.y),
        egui::vec2(tp_w, RULER_H),
    );
    ui.painter()
        .rect_filled(events_corner, 0.0, crate::theme::track_bg());
    let tpb = doc.data.model.meta.ppq;
    let (sig_num, sig_den) = doc.data.model.tempo_map.time_sig_default;
    let ar_quantize = doc.edit.quantize_arrange.resolved_with_time_sig(
        doc.edit.arrange_view.base.pixels_per_tick,
        tpb,
        sig_num,
        sig_den,
        false,
    );
    crate::widgets::event_rows::controls(
        ui,
        events_corner,
        &mut doc.edit.event_rows_ar,
        &mut doc.edit.playback,
        tpb,
        Some(crate::widgets::event_rows::QuantizeControl {
            current: &mut doc.edit.quantize_arrange,
            ppq: tpb,
            auto_resolved: ar_quantize,
            id_salt: "arr_quantize",
        }),
    );

    // ── 底部角落（音轨面板下方、横向滚动条左侧）：只铺背景 ──
    // 「添加音轨」入口已移到音轨列表末尾的占位行（与移动端一致）。
    {
        let corner_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x,
                arr_rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H,
            ),
            egui::pos2(arr_rect.min.x + tp_w, arr_rect.max.y),
        );
        ui.painter()
            .rect_filled(corner_rect, 0.0, crate::theme::track_bg());
    }
}

/// 音轨面板与主区之间的竖向分割条。
fn draw_vertical_splitter(
    ui: &mut egui::Ui,
    layout: ArrangeLayout<'_>,
    arr_rect: egui::Rect,
    tp_w: f32,
    arr_total_w: f32,
) {
    // ── Vertical splitter handle (drawn last so it sits on top) ──
    let v_handle = egui::Rect::from_min_max(
        egui::pos2(arr_rect.min.x + tp_w, arr_rect.min.y),
        egui::pos2(
            arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
            arr_rect.max.y,
        ),
    );
    let v_resp = crate::widgets::split_handle::vertical(ui, "__v_split__", v_handle);
    crate::widgets::hint::hover(ui.ctx(), &v_resp, t!("hint.arrange.split_v"));
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
}

/// 状态栏讲解行：位置 / 音轨号 / 选框统计。
#[allow(clippy::too_many_arguments)] // 视图几何参数透传
fn draw_hover_hint(
    ui: &mut egui::Ui,
    doc: &mut Document,
    arr_rect: egui::Rect,
    tp_rect: egui::Rect,
    gpu_rect: egui::Rect,
    music_rect: egui::Rect,
    row_layout: &yinhe_types::ArRowLayout,
    num_tracks: usize,
    tp_w: f32,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
) {
    // ── 状态栏讲解行：走带悬停提示（位置 + 音轨号；有选框时优先显示选框统计）──
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
        let model = &*doc.data.model;
        let tpb = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let lh = doc.edit.arrange_view.lane_height();
        let scroll_y = doc.edit.arrange_view.base.scroll_y;
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
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(
                arr_rect.min.x + tp_w + crate::theme::SPLIT_HANDLE_W,
                arr_rect.min.y,
            ),
            egui::pos2(gpu_rect.max.x, arr_rect.min.y + RULER_H),
        );
        if music_rect.contains(pos) {
            let tick = doc
                .edit
                .arrange_view
                .x_to_tick(pos.x - gpu_rect.min.x)
                .max(0.0);
            let pos_str =
                format_tick_bar_beat_with_time_sig(tick, tpb, sig_events, def_num, def_den);
            let hint = if let Some(s) = sel_text {
                s
            } else {
                let info = format!("{} {}", pos_str, hover_desc(pos.y));
                format!("{} · {}", t!("hint.panel.arrange_content"), info)
            };
            crate::widgets::hint::set_region(ui.ctx(), hint);
        } else if tp_rect.contains(pos) {
            let hint = if let Some(s) = sel_text {
                s
            } else {
                format!(
                    "{} · {}",
                    t!("hint.panel.arrange_track_panel"),
                    hover_desc(pos.y)
                )
            };
            crate::widgets::hint::set_region(ui.ctx(), hint);
        } else if ruler_rect.contains(pos) {
            crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.arrange_ruler"));
        } else if arr_rect.contains(pos) {
            // 走带视图内但不在内容区（标尺/滚动条）→ 清空区域提示
            crate::widgets::hint::clear_region(ui.ctx());
        }
    }
}
