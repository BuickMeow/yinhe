use eframe::egui;
use rust_i18n::t;

use crate::app::App;
use crate::arrange;
use crate::piano_view;
use crate::right_panel::automation_undo::push_automation_actions;
use crate::right_panel::info_panel::selection::selected_am_events;
use yinhe_editor_core::batch_ops::summarize_selected;
use yinhe_types::AnchorSelRect;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

/// Layout geometry computed once per frame, shared by arrangement and pianoroll.
pub(in crate::app) struct LayoutInfo {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub bottom_y: f32,
    pub right_panel_total_w: f32,
}

/// 讲解行的选框统计（layout.rs 每帧计算，视图命中选框后显示）。
/// 三视图选框互斥，同一时刻最多只有一个来源为 Some。
#[derive(Clone)]
pub(crate) struct SelHintInfo {
    /// 选中音符数（PR/AR）或事件数（AM）。
    pub count: u64,
    /// 时间跨度（bar.beat.tick→bar.beat.tick）。
    pub span: String,
}

impl App {
    /// 存在选框时计算讲解行的选框统计（无选框返回 None）。
    /// PR/AR/AM 三视图选框互斥，同一时刻最多一个来源。
    fn compute_sel_hint(doc: &yinhe_editor_core::document::Document) -> Option<SelHintInfo> {
        let model = &doc.data.model;
        let ppq = model.meta.ppq;
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let sig_events = model.tempo_map.time_sig_events.as_slice();
        let fmt = |t: f64| format_tick_bar_beat_with_time_sig(t, ppq, sig_events, def_num, def_den);

        let pr_rects = doc.edit.sel_rect.effective_rects();
        let ar_rects = &doc.edit.arr_sel_rect;
        let am_rects: Vec<&AnchorSelRect> = doc
            .edit
            .controller_panels
            .iter()
            .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
            .flat_map(|p| p.anchor_sel_rects.iter())
            .collect();

        if !pr_rects.is_empty() {
            let (t0, t1) = pr_rects.iter().fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(a, b), &(ts, te, _, _)| (a.min(ts), b.max(te)),
            );
            Some(SelHintInfo {
                count: summarize_selected(model, &doc.edit.selected).count,
                span: format!("{}→{}", fmt(t0), fmt(t1)),
            })
        } else if !ar_rects.is_empty() {
            let (t0, t1) = ar_rects.iter().fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(a, b), &(ts, te, _, _)| (a.min(ts), b.max(te)),
            );
            Some(SelHintInfo {
                count: summarize_selected(model, &doc.edit.selected).count,
                span: format!("{}→{}", fmt(t0), fmt(t1)),
            })
        } else if !am_rects.is_empty() {
            let (t0, t1) = am_rects
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| {
                    (
                        a.min(r.tick_start.min(r.tick_end)),
                        b.max(r.tick_start.max(r.tick_end)),
                    )
                });
            let count: usize = doc
                .edit
                .controller_panels
                .iter()
                .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
                .map(|p| selected_am_events(doc, p, &p.anchor_sel_rects).len())
                .sum();
            Some(SelHintInfo {
                count: count as u64,
                span: format!("{}→{}", fmt(t0), fmt(t1)),
            })
        } else {
            None
        }
    }

    /// Compute layout geometry for the current frame.
    pub(in crate::app) fn compute_layout(&mut self, ui: &mut egui::Ui) -> LayoutInfo {
        let mut remaining = ui.available_rect_before_wrap();

        let has_arr = self.view_mode.show_transport() && self.active_doc.is_some();
        let has_piano = self
            .view_mode
            .show_pianoroll(self.show_pianoroll_in_arrange)
            && self.active_doc.is_some();

        let right_panel_total_w = if self.right_tab.is_some() {
            let max_w = (remaining.width() - 60.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0);
            let pw = (self.right_panel_width + 4.0)
                .clamp(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0, max_w);
            self.right_panel_width = (pw - 4.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH);
            pw
        } else {
            0.0
        };
        remaining.max.x -= right_panel_total_w;

        let total = remaining.size();
        let arr_h = if has_arr {
            if has_piano {
                (total.y * self.arr_split).max(crate::theme::MIN_ARR_HEIGHT)
            } else {
                total.y
            }
        } else {
            0.0
        };
        let bottom_y = remaining.min.y
            + arr_h
            + if has_arr && has_piano {
                crate::theme::SPLIT_GAP
            } else {
                0.0
            };

        LayoutInfo {
            remaining,
            arr_h,
            bottom_y,
            right_panel_total_w,
        }
    }

    /// Show the main content area: arrangement view, pianoroll, and note drag handling.
    pub(in crate::app) fn show_main_content(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        let Some(idx) = self.active_doc else {
            return;
        };
        // 三视图选框互斥：先于任何渲染执行，保证同一时刻只有一个视图拥有选框。
        self.enforce_sel_rect_exclusivity(idx);

        let is_playing = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        let mut follow_mode = self.follow_mode;

        // 讲解行选框统计（存在选框时 Some；PR/AR/AM 互斥，单一来源）
        let sel_hint = Self::compute_sel_hint(&self.documents[idx]);

        // Arrangement view
        let mut needs_audio_rebuild = false;
        let (arr_drag_delta, arr_eraser_rect, arr_quantize): (
            Option<crate::arrange::ArrDragDelta>,
            Option<crate::arrange::ArrSelRect>,
            Option<yinhe_editor_core::quantize::QuantizePreset>,
        ) = if self.view_mode.show_transport() {
            let mut request_pianoroll = false;
            let mut arr_drag_delta: Option<crate::arrange::ArrDragDelta> = None;
            let mut arr_eraser_rect: Option<crate::arrange::ArrSelRect> = None;
            let mut guard = crate::app::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
            let cfg = crate::arrange::ArrangeViewCfg {
                is_playing,
                follow_mode: &mut follow_mode,
                active_tool: &self.active_tool,
                scroll_mode: self.audio_settings.scroll_mode,
                min_border_width: self.audio_settings.min_border_width,
                revision: guard.as_ref().data.revision,
            };
            let arr_quantize = arrange::show(
                ui,
                guard.as_mut(),
                &mut self.arrange_view,
                crate::arrange::ArrangeLayout {
                    remaining: layout.remaining,
                    arr_h: layout.arr_h,
                    transport_panel_width: &mut self.transport_panel_width,
                },
                &mut self.arr_renderer,
                &mut self.arr_render_ctx,
                cfg,
                &mut self.last_cursor_tick,
                self.audio_state.handle.as_ref(),
                &mut request_pianoroll,
                &mut self.track_selection_anchor,
                &mut arr_drag_delta,
                &mut arr_eraser_rect,
                &mut self.info_content,
                &mut needs_audio_rebuild,
                &mut self.status_hint,
                sel_hint.as_ref(),
            );
            if request_pianoroll {
                self.show_pianoroll_in_arrange = true;
            }
            (arr_drag_delta, arr_eraser_rect, arr_quantize) // guard dropped here
        } else {
            (None, None, None)
        };
        // 方案 A：音轨结构变化（add/remove track）→ drop 旧引擎。
        // ChannelLayout 在引擎创建时冻结，旧引擎无法 dispatch 新增通道。
        // 必须在 guard 被释放后调用 —— teardown_audio 借用 &mut self，
        // 与 ReplaceGuard 借用的 &mut self.documents[idx] 冲突。
        // 下一帧 rebuild_audio_if_needed 会用新 model 重新 spawn 引擎和 ChannelLayout。
        if needs_audio_rebuild {
            self.teardown_audio();
        }

        // Handle AR eraser (guard is dropped, no outstanding borrow on self.documents)
        if let Some((t_start, t_end, track_lo, track_hi)) = arr_eraser_rect {
            let mut sel = yinhe_core::Selection::default();
            sel.add_rect_track(
                t_start as u32,
                t_end as u32,
                0,
                127,
                track_lo as u16,
                track_hi as u16,
            );
            let Some(idx) = self.active_doc else { return };
            self.documents[idx].edit.selected = sel;
            self.with_undo(t!("undo.eraser_arrange").as_ref(), |doc| {
                doc.delete_selected()
            });
        }

        // Handle AR drag after guard is dropped (no outstanding borrow on self.documents)
        if let Some((delta_ticks, delta_tracks)) = arr_drag_delta {
            self.handle_arr_drag(delta_ticks, delta_tracks);
        }

        // Handle AR quantize preset change from corner button
        if let Some(new_preset) = arr_quantize
            && let Some(doc) = self.documents.get_mut(idx)
        {
            doc.edit.quantize_arrange = new_preset;
        }

        // Pianoroll area
        if self
            .view_mode
            .show_pianoroll(self.show_pianoroll_in_arrange)
        {
            self.show_pianoroll_split(
                ui,
                layout,
                idx,
                is_playing,
                &mut follow_mode,
                sel_hint.as_ref(),
            );
        }

        self.follow_mode = follow_mode;
    }

    /// PR/AR/AM 三视图选框互斥。
    ///
    /// 每帧渲染前检查各视图的选框数量与共享选区状态：
    /// - 某视图**新增**了选框（框选提交，含 shift/cmd 加选追加）→ 清除其他视图的选框；
    /// - 共享选区被**清空**（AR/PR 在 press 时清空 selected，即开始新的框选/空白点击）
    ///   → 立即清除全部视图的选框（发起方已自行清空自己的选框）。
    ///
    /// 清除时同步从共享 `Selection` 中精确移除对应视图的矩形，避免误伤其他视图的选区。
    /// `selected` 被三个视图共享，不能整体 clear()。
    fn enforce_sel_rect_exclusivity(&mut self, idx: usize) {
        let doc = &mut self.documents[idx];

        // 清除前先采集各视图选框快照（含 f64→整数转换），供精确移除共享 Selection 中的矩形。
        let arr_rects: Vec<(u32, u32, u16, u16)> = doc
            .edit
            .arr_sel_rect
            .iter()
            .map(|&(ts, te, tl, th)| (ts as u32, te as u32, tl as u16, th as u16))
            .collect();
        let pr_rects: Vec<(u32, u32, u8, u8)> = doc
            .edit
            .sel_rect
            .rects
            .iter()
            .map(|&(ts, te, kl, kh)| (ts as u32, te as u32, kl, kh))
            .collect();

        let arr_count = doc.edit.arr_sel_rect.len();
        let pr_count = doc.edit.sel_rect.rects.len();
        let am_count: usize = doc
            .edit
            .controller_panels
            .iter()
            .map(|p| p.anchor_sel_rects.len())
            .sum();

        let arr_gained = arr_count > self.prev_arr_count;
        let pr_gained = pr_count > self.prev_pr_count;
        let am_gained = am_count > self.prev_am_count;
        // AR/PR 在 press 开始新框选/空白点击时会清空 selected。
        let selection_cleared = doc.edit.selected.is_empty() && self.prev_selected_nonempty;

        // 新选框只可能来自一个视图（单鼠标交互），各清除条件互不重叠。
        let clear_arr = selection_cleared || pr_gained || am_gained;
        let clear_pr = selection_cleared || arr_gained || am_gained;
        let clear_am = selection_cleared || arr_gained || pr_gained;

        if clear_arr {
            doc.edit.arr_sel_rect.clear();
            doc.edit.selected.remove_rects_track(&arr_rects);
        }
        if clear_pr {
            doc.edit.sel_rect.clear();
            doc.edit.selected.remove_rects(&pr_rects);
        }
        if clear_am {
            for panel in &mut doc.edit.controller_panels {
                panel.anchor_sel_rects.clear();
            }
        }

        // 以清除后的状态更新 prev，供下一帧比较。
        self.prev_arr_count = doc.edit.arr_sel_rect.len();
        self.prev_pr_count = doc.edit.sel_rect.rects.len();
        self.prev_am_count = doc
            .edit
            .controller_panels
            .iter()
            .map(|p| p.anchor_sel_rects.len())
            .sum();
        self.prev_selected_nonempty = !doc.edit.selected.is_empty();
    }

    /// Show the pianoroll split area, including the split handle and pianoroll view.
    fn show_pianoroll_split(
        &mut self,
        ui: &mut egui::Ui,
        layout: &LayoutInfo,
        idx: usize,
        is_playing: bool,
        follow_mode: &mut crate::view_interaction::FollowMode,
        sel_hint: Option<&SelHintInfo>,
    ) {
        // Horizontal splitter
        if self.view_mode.show_transport() {
            let split_right = layout.remaining.max.x;
            let h_split_rect = egui::Rect::from_min_max(
                egui::pos2(
                    layout.remaining.min.x,
                    layout.remaining.min.y + layout.arr_h,
                ),
                egui::pos2(
                    split_right,
                    layout.remaining.min.y + layout.arr_h + crate::theme::SPLIT_GAP,
                ),
            );
            let h_int_rect = egui::Rect::from_min_max(
                egui::pos2(
                    layout.remaining.min.x,
                    layout.remaining.min.y + layout.arr_h + 0.5,
                ),
                egui::pos2(
                    split_right,
                    layout.remaining.min.y + layout.arr_h + crate::theme::SPLIT_GAP,
                ),
            );
            let h_split_resp =
                crate::widgets::split_handle::horizontal(ui, "__h_split__", h_int_rect);
            ui.painter().rect_filled(
                h_split_rect,
                0.0,
                if h_split_resp.hovered() || h_split_resp.dragged() {
                    crate::theme::SPLIT_HOVER
                } else {
                    crate::theme::SPLIT_DEFAULT
                },
            );
            if h_split_resp.dragged() {
                let total_y = layout.remaining.size().y;
                let delta = h_split_resp.drag_delta().y;
                self.arr_split = ((layout.arr_h + delta) / total_y)
                    .clamp(crate::theme::SPLIT_CLAMP_MIN, crate::theme::SPLIT_CLAMP_MAX);
            }
        }

        // Pianoroll GPU view
        let auto_wgpu_state = self.render_ctx.wgpu_state().clone();
        while self.controller_renderers.len() <= idx {
            self.controller_renderers.push(Vec::new());
        }

        let mut auto_edit_events: Vec<crate::piano_view::automation_panel::AutomationEdit> =
            Vec::new();
        let mut velocity_edits: Vec<yinhe_types::VelocityEdit> = Vec::new();

        let (piano_event, note_drag_delta, pencil_note_drag, note_resize_delta, preview_reqs) = {
            let mut guard = crate::app::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
            let doc = guard.as_mut();
            let midi_source: Option<&dyn yinhe_types::NoteSource> = Some(doc.data.model.as_ref());
            let piano_rect = egui::Rect::from_min_max(
                egui::pos2(layout.remaining.min.x, layout.bottom_y),
                layout.remaining.max,
            );

            let mut event = None;
            let mut note_drag_delta: Option<(i64, i32, bool)> = None;
            let mut pencil_note_drag: Option<crate::piano_view::PencilNoteDrag> = None;
            let mut note_resize_delta: Option<(crate::piano_view::ResizeSide, i64)> = None;
            let mut preview_reqs: Vec<crate::piano_view::PreviewReq> = Vec::new();
            ui.scope_builder(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                let _piano_total_start = if yinhe_memtrace::perf_probe::enabled() {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                let show_all = doc
                    .edit
                    .conductor_track_idx
                    .map(|c| doc.edit.track_selected.contains(&c))
                    .unwrap_or(false);
                // PR 显示音轨 = 选中音轨（含 Conductor 意为全选）∪ editing_track。
                // 有铅笔图标的音轨就像被选择了一样，一直显示在 PR 上。
                let editing = doc.edit.editing_track;
                let pr_visible: Vec<bool> = (0..doc.edit.track_visible.len())
                    .map(|i| {
                        let selected = show_all || doc.edit.track_selected.contains(&(i as u16));
                        let is_editing = editing == Some(i as u16);
                        doc.edit.track_visible[i] && (selected || is_editing)
                    })
                    .collect();
                let tpb = doc.data.model.meta.ppq;
                let ts_num = doc
                    .data
                    .model
                    .conductor
                    .time_sig
                    .first()
                    .map(|t| t.numerator)
                    .unwrap_or(4);
                let ts_den = doc
                    .data
                    .model
                    .conductor
                    .time_sig
                    .first()
                    .map(|t| t.denominator)
                    .unwrap_or(2);
                let ts_events: Vec<yinhe_types::TimeSigEvent> = doc
                    .data
                    .model
                    .conductor
                    .time_sig
                    .iter()
                    .map(|t| yinhe_types::TimeSigEvent {
                        tick: t.tick,
                        numerator: t.numerator,
                        denominator: t.denominator,
                    })
                    .collect();
                // Get automation lanes：以 editing_track 为唯一编辑目标。
                // Conductor 不在此处提供 lanes（Tempo 由单独的 tempo_lane 传入）。
                // editing_track 缺失/不可见/是 conductor 时返回空 Vec。
                // （editing_track 已常驻 PR 显示，不再要求 track_selected。）
                let automation_lanes: Vec<yinhe_types::AutomationLane> = {
                    let edit_trk = doc
                        .edit
                        .editing_track
                        .filter(|&t| {
                            doc.edit
                                .track_visible
                                .get(t as usize)
                                .copied()
                                .unwrap_or(true)
                        })
                        .filter(|&t| Some(t) != doc.edit.conductor_track_idx);
                    match edit_trk {
                        Some(trk_idx) => doc
                            .data
                            .model
                            .tracks
                            .get(trk_idx as usize)
                            .map(|t| t.automation_lanes.clone())
                            .unwrap_or_default(),
                        None => Vec::new(),
                    }
                };
                // 渲染 lanes：所有 PR 可见音轨的 lanes（引用，零拷贝）。
                // 与音符显示逻辑一致（选中音轨 ∪ editing_track，Conductor 选中 = 全部）。
                let automation_render_lanes: Vec<&yinhe_types::AutomationLane> = doc
                    .data
                    .model
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| pr_visible.get(*i).copied().unwrap_or(false))
                    .flat_map(|(_, t)| t.automation_lanes.iter())
                    .collect();
                let auto_ctx = Some(piano_view::AutomationPanelsCtx {
                    panels: &mut doc.edit.controller_panels,
                    renderers: &mut self.controller_renderers[idx],
                    lanes: &automation_lanes,
                    render_lanes: &automation_render_lanes,
                    show: &mut doc.edit.show_controller_panels,
                    wgpu_state: &auto_wgpu_state,
                });
                let mut feedback = piano_view::PianoViewFeedback {
                    auto_edit_events: &mut auto_edit_events,
                    info_content: &mut self.info_content,
                    right_tab: &mut self.right_tab,
                    automation_drag_ghost: &mut self.automation_drag_ghost,
                    note_drag_delta: &mut note_drag_delta,
                    pencil_note_drag: &mut pencil_note_drag,
                    note_resize_delta: &mut note_resize_delta,
                    velocity_edits: &mut velocity_edits,
                    preview_reqs: &mut preview_reqs,
                    status_hint: &mut self.status_hint,
                };
                event = piano_view::show(
                    ui,
                    ui.available_size(),
                    &mut self.pianoroll,
                    &mut self.render_ctx,
                    self.render_thread.as_ref(),
                    &mut self.pianoroll_view,
                    &mut self.last_cull_revision,
                    &mut self.last_cull_revision_only,
                    &mut self.last_hidden_hash,
                    &mut self.last_tv_hash,
                    &mut self.cull_rebuild,
                    midi_source,
                    Some(&doc.data.model),
                    &mut doc.edit.selected,
                    &pr_visible,
                    &doc.edit.track_colors_cache,
                    &mut doc.edit.cursor_tick,
                    is_playing,
                    doc.edit.quantize_pianoroll,
                    tpb,
                    Some((tpb, ts_num, ts_den, &ts_events)),
                    &doc.data.model.conductor.key_sig,
                    &mut self.piano_last_cursor_tick,
                    follow_mode,
                    &self.active_tool,
                    auto_ctx,
                    self.audio_settings.scroll_mode,
                    self.audio_settings.min_border_width,
                    self.audio_settings.note_outline,
                    self.audio_settings.use_gpu_cull,
                    &doc.data.model.conductor.tempo,
                    &mut doc.edit.sel_rect,
                    &doc.edit.track_selected,
                    doc.edit.conductor_track_idx,
                    doc.edit.editing_track,
                    doc.data.revision,
                    doc.data.note_revisions(),
                    &mut feedback,
                    sel_hint,
                );
                if let Some(t0) = _piano_total_start {
                    yinhe_memtrace::perf_probe::record_piano_total(t0.elapsed());
                }
            });
            (
                event,
                note_drag_delta,
                pencil_note_drag,
                note_resize_delta,
                preview_reqs,
            )
        };

        // 音符听觉预览（铅笔新建/拖拽、选框拖拽触发）。
        self.send_note_previews(&preview_reqs);

        // Handle piano-view events
        if let Some(event) = piano_event {
            use crate::piano_view::PianoViewEvent;
            match event {
                PianoViewEvent::SelectionAction(action) => {
                    use crate::widgets::selection_actions::SelectionAction;
                    match action {
                        SelectionAction::Delete => self.delete_selected_notes(),
                        SelectionAction::Duplicate => self.duplicate_selected_notes(),
                        SelectionAction::TransposeUp => self.transpose_selected_notes(12),
                        SelectionAction::TransposeDown => self.transpose_selected_notes(-12),
                        SelectionAction::FlipHorizontal => {
                            self.flip_selected_notes(yinhe_editor_core::FlipAxis::Horizontal)
                        }
                        SelectionAction::FlipVertical => {
                            self.flip_selected_notes(yinhe_editor_core::FlipAxis::Vertical)
                        }
                    }
                }
                PianoViewEvent::AddNote { track, note } => {
                    // 新音符默认力度 = 该音轨最近一次 velocity 修改值（无记录 100）。
                    let mut note = note;
                    if let Some(idx) = self.active_doc {
                        note.velocity = self.documents[idx].edit.default_velocity(track);
                    }
                    self.add_note_with_undo(track, note);
                }
                PianoViewEvent::EraserDelete {
                    t_start,
                    t_end,
                    key_lo,
                    key_hi,
                    track_lo,
                    track_hi,
                } => {
                    let Some(idx) = self.active_doc else { return };
                    let mut sel = yinhe_core::Selection::default();
                    sel.add_rect_track(t_start, t_end, key_lo, key_hi, track_lo, track_hi);
                    self.documents[idx].edit.selected = sel;
                    self.with_undo(t!("undo.eraser_delete").as_ref(), |doc| {
                        doc.delete_selected()
                    });
                }
                PianoViewEvent::QuantizePreset(preset) => {
                    let Some(idx) = self.active_doc else { return };
                    self.documents[idx].edit.quantize_pianoroll = preset;
                }
            }
        }

        // Handle note drag
        self.handle_note_drag(note_drag_delta);

        // Handle note resize (selection edge drag)
        self.handle_note_resize(note_resize_delta);

        // Handle pencil note drag
        self.handle_pencil_note_drag(pencil_note_drag);

        // Handle automation edits
        if !auto_edit_events.is_empty() {
            self.handle_automation_edits(auto_edit_events);
        }

        // Handle velocity stroke edits
        if !velocity_edits.is_empty() {
            self.handle_velocity_edits(&velocity_edits);
        }
    }

    /// 把 automation 面板 velocity 笔划产生的编辑应用到 Document（一笔 = 一个 undo entry）。
    fn handle_velocity_edits(&mut self, edits: &[yinhe_types::VelocityEdit]) {
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.set_notes_velocity(edits) {
            self.pianoroll_view.base.dirty = true;
            doc.push_undo(action, t!("undo.edit_velocity").as_ref(), before);
            // 纯音符 velocity 修改：只更新 audible_notes，不重建 CC，不 chase
            self.notify_notes_changed();
        }
    }

    /// 把 automation 面板产生的编辑事件应用到 Document，push undo，并通知音频线程。
    fn handle_automation_edits(
        &mut self,
        edits: Vec<crate::piano_view::automation_panel::AutomationEdit>,
    ) {
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if !actions.is_empty() {
            self.pianoroll_view.base.dirty = true;
            push_automation_actions(doc, actions, t!("undo.edit_automation").as_ref(), before);
            self.notify_audio_model_changed();
        }
    }

    /// Handle note drag — called once on release.
    /// 处理事件浏览器的跳转请求：设置 cursor_tick、切到 piano roll 视图、
    /// 切 editing_track（音符/automation 事件）、滚动到中心。
    fn handle_jump_request(
        &mut self,
        req: crate::right_panel::event_browser::JumpRequest,
        layout: &LayoutInfo,
    ) {
        // 1. 切到 piano roll 视图（如果当前不在）
        if !self
            .view_mode
            .show_pianoroll(self.show_pianoroll_in_arrange)
        {
            self.view_mode = crate::chrome::mode_bar::ViewMode::Edit;
        }

        // 2. 切 editing_track（音符/CC/PB/PC 事件需要）。
        // Conductor 不可作为编辑目标（无铅笔图标）；Tempo 编辑不依赖它。
        if let Some((track, _key)) = req.note
            && let Some(idx) = self.active_doc
            && self.documents[idx].edit.conductor_track_idx != Some(track)
        {
            self.documents[idx].edit.editing_track = Some(track);
        }

        // 3. 设置 cursor_tick
        if let Some(idx) = self.active_doc {
            self.documents[idx].edit.cursor_tick = Some(req.tick as f64);
        }

        // 4. 滚动到中心（参考 follow.rs Page 模式公式）
        let view = &mut self.pianoroll_view;
        let viewport_w = layout.remaining.width() - layout.right_panel_total_w;
        let content_w = viewport_w - view.base.left_panel_width;
        let target_x = req.tick as f32 * view.base.pixels_per_tick;
        view.base.scroll_x = (target_x - content_w * 0.5).max(0.0);
        view.base.dirty = true;
    }

    fn handle_note_drag(&mut self, note_drag_delta: Option<(i64, i32, bool)>) {
        if let Some((delta_ticks, delta_keys, alt)) = note_drag_delta {
            let Some(idx) = self.active_doc else { return };
            let doc = &mut self.documents[idx];
            let before = doc.capture_snapshot();
            let (action, label) = if alt {
                (
                    doc.duplicate_selected_to(delta_ticks, delta_keys),
                    t!("undo.duplicate_move").to_string(),
                )
            } else {
                (
                    doc.move_selected_notes(delta_ticks, delta_keys),
                    t!("undo.move_notes").to_string(),
                )
            };
            if let Some(action) = action {
                self.pianoroll_view.base.dirty = true;
                doc.push_undo(action, &label, before);
                // 纯音符移动/复制：只更新 audible_notes，不重建 CC，不 chase
                self.notify_notes_changed();
            }
        }
    }

    /// Handle note resize: shift one edge of all selected notes by `dt` ticks.
    /// 选框工具边缘拖动伸缩：所有选中音符的 start_tick (Left) 或 end_tick (Right) 统一偏移。
    fn handle_note_resize(
        &mut self,
        note_resize_delta: Option<(crate::piano_view::ResizeSide, i64)>,
    ) {
        if let Some((side, dt)) = note_resize_delta {
            let Some(idx) = self.active_doc else { return };
            let doc = &mut self.documents[idx];
            let before = doc.capture_snapshot();
            if let Some(action) = doc.resize_selected_notes(side, dt) {
                self.pianoroll_view.base.dirty = true;
                doc.push_undo(action, t!("undo.resize_notes").as_ref(), before);
                self.notify_notes_changed();
            }
        }
    }

    /// Handle pencil note drag updates (move or resize a single note).
    fn handle_pencil_note_drag(&mut self, drag: Option<crate::piano_view::PencilNoteDrag>) {
        let Some(drag) = drag else { return };
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.pencil_drag_note(&drag) {
            self.pianoroll_view.base.dirty = true;
            let label = match &drag {
                crate::piano_view::PencilNoteDrag::Move { .. } => t!("undo.move_note").to_string(),
                _ => t!("undo.resize_note").to_string(),
            };
            doc.push_undo(action, &label, before);
            // 纯音符拖动/缩放：只更新 audible_notes，不重建 CC，不 chase
            self.notify_notes_changed();
        }
    }

    /// Handle AR drag: move selected notes + automation events by `(delta_ticks, delta_tracks)`.
    /// Single atomic operation = single undo step.
    fn handle_arr_drag(&mut self, delta_ticks: i64, delta_tracks: i32) {
        if delta_ticks == 0 && delta_tracks == 0 {
            return;
        }
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];

        let before = doc.capture_snapshot();
        if let Some(action) = doc.move_selected_arrange(delta_ticks, delta_tracks) {
            self.arrange_view.base.dirty = true;
            doc.push_undo(action, t!("undo.move_in_arrange").as_ref(), before);
            self.notify_audio_model_changed();
        }
    }

    /// Show right panel, and request repaint if playing.
    pub(in crate::app) fn show_panels_and_overlays(
        &mut self,
        ui: &mut egui::Ui,
        layout: &LayoutInfo,
    ) {
        // Right panel
        if self.right_tab.is_some() {
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(layout.remaining.max.x, layout.remaining.min.y),
                egui::vec2(layout.right_panel_total_w, layout.remaining.height()),
            );
            let doc = self.active_doc.and_then(|idx| self.documents.get_mut(idx));
            let (changed, jump_request) = crate::right_panel::show(
                ui,
                right_rect,
                &mut self.right_panel_width,
                &mut self.right_tab,
                &mut self.audio_settings,
                doc,
                self.audio_state.handle.as_ref(),
                &mut self.event_browser_state,
                &mut self.info_content,
                self.automation_drag_ghost,
                &mut self.status_hint,
            );
            if changed {
                self.teardown_audio();
            }
            if let Some(req) = jump_request {
                self.handle_jump_request(req, layout);
            }
        }

        // Request repaint during playback (or while waiting for audio thread to start)
        let is_audio_playing = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        // macOS: 播放时阻止 App Nap（防止系统降低定时器精度导致播放卡顿），
        // 仅在播放状态翻转时调用平台 API。
        let playing = is_audio_playing || self.audio_state.pending_playback;
        if playing != self.app_nap_active {
            self.app_nap_active = playing;
            crate::platform::set_app_nap_enabled(playing);
        }
        // macOS: 窗口被完全遮挡时暂停动画重绘省 CPU/电（音频在独立线程继续播）。
        // 恢复可见时 Occluded(false) 事件会触发一次重绘，playhead 按 anchor+Instant 重算，不会跳帧丢位置。
        let occluded = ui.ctx().input(|i| i.viewport().occluded == Some(true));
        if playing && !occluded {
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_test_document;
    use yinhe_types::{AnchorSelRect, AutomationPanelView, AutomationTarget};

    /// PR 选框：音符数 + 时间跨度（4/4、ppq=480 下 960 tick = 1.3.000）。
    #[test]
    fn sel_hint_pr_selection() {
        let mut doc = make_test_document();
        doc.edit.sel_rect.rects = vec![(0.0, 960.0, 60, 60)];
        doc.edit
            .selected
            .add_rect_track(0, 960, 60, 60, 0, u16::MAX);

        let hint = App::compute_sel_hint(&doc).expect("PR 选框应生成选框信息");
        assert_eq!(hint.count, 2); // key 60 的两个音符（0-480、480-960）
        assert_eq!(hint.span, "1.1.000→1.3.000");
    }

    /// AR 选框：与 PR 同源共享 selected，统计一致。
    /// 注意 from_model 会插入 Conductor 轨，音符 track 索引 +1。
    #[test]
    fn sel_hint_ar_selection() {
        let mut doc = make_test_document();
        doc.edit.arr_sel_rect = vec![(0.0, 960.0, 0, 0)];
        doc.edit
            .selected
            .add_rect_track(0, 960, 60, 60, 0, u16::MAX);

        let hint = App::compute_sel_hint(&doc).expect("AR 选框应生成选框信息");
        assert_eq!(hint.count, 2); // key 60 的两个音符（track 0..=MAX）
        assert_eq!(hint.span, "1.1.000→1.3.000");
    }

    /// AM 选框：事件数统计（CC7 lane 两个锚点均在选框内）。
    /// 插入 Conductor 轨后 Lead（含 CC7 lane）位于 index 1。
    #[test]
    fn sel_hint_am_selection() {
        let mut doc = make_test_document();
        doc.edit.editing_track = Some(1);
        doc.edit.controller_panels.clear();
        doc.edit.controller_panels.push(AutomationPanelView {
            show_velocity: false,
            selected_target: AutomationTarget::CC { controller: 7 },
            anchor_sel_rects: vec![AnchorSelRect {
                tick_start: 0.0,
                tick_end: 240.0,
                value_range: None,
            }],
            ..Default::default()
        });

        let hint = App::compute_sel_hint(&doc).expect("AM 选框应生成选框信息");
        assert_eq!(hint.count, 2); // CC7 lane：tick 0 / 240 两个事件
        assert_eq!(hint.span, "1.1.000→1.1.240");
    }

    /// 无选框 → None。
    #[test]
    fn sel_hint_none_without_selection() {
        let doc = make_test_document();
        assert!(App::compute_sel_hint(&doc).is_none());
    }
}
