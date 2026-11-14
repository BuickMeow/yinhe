//! 主内容区各视图：工程走带、欢迎页、钢琴卷帘分屏。

use super::*;

impl App {
    /// 工程走带（AR）视图渲染 + 事件落地：主窗口与浮动窗口共用同一路径。
    ///
    /// `floating` 选择渲染器/渲染上下文：AR 系视图共享主实例（预览纹理单张，
    /// 同帧只能渲染一个），浮动窗口必须走独立实例，否则与主窗口视图互相覆盖。
    pub(in crate::app) fn show_arrange_view(
        &mut self,
        ui: &mut egui::Ui,
        remaining: egui::Rect,
        arr_h: f32,
        follow_mode: &mut crate::view_interaction::FollowMode,
        floating: bool,
    ) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let is_playing = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        let sel_hint = Self::compute_sel_hint(&self.workspace.documents[idx]);

        let mut request_pianoroll = false;
        let mut arr_drag_delta: Option<crate::arrange::ArrDragDelta> = None;
        let mut arr_eraser_rect: Option<crate::arrange::ArrSelRect> = None;
        let mut arr_scale_span: Option<u64> = None;
        let mut arr_riff_steps: Option<(i64, i64)> = None;
        let mut automation_picker_req: Option<usize> = None;
        // 方案 A：音轨结构变化（add/remove track）→ drop 旧引擎。
        // ChannelLayout 在引擎创建时冻结，旧引擎无法 dispatch 新增通道。
        // 必须在 guard 被释放后调用 —— teardown_audio 借用 &mut self，
        // 与 ReplaceGuard 借用的 &mut self.workspace.documents[idx] 冲突。
        let mut needs_audio_rebuild = false;
        // AR 自动化 lane 增删/编辑 → notify_audio_model_changed（不重建引擎）。
        let mut needs_audio_notify = false;
        {
            let mut guard =
                crate::app::main_loop::ReplaceGuard::new(&mut self.workspace.documents[idx]);
            // 走带视图始终使用普通选框工具：工具选择只作用于钢琴卷帘。
            let arrange_tool = crate::widgets::tools_panel::Tool::Select;
            let cfg = crate::arrange::ArrangeViewCfg {
                is_playing,
                follow_mode,
                active_tool: &arrange_tool,
                min_border_width: self.audio_settings.min_border_width,
                revision: guard.as_ref().data.revision,
                clip_enabled: self.audio_settings.clip_enabled,
            };
            let (renderer, render_ctx) = if floating {
                (&mut self.float_arr_renderer, &mut self.float_arr_render_ctx)
            } else {
                (&mut self.arr_renderer, &mut self.arr_render_ctx)
            };
            arrange::show(
                ui,
                guard.as_mut(),
                crate::arrange::ArrangeLayout {
                    remaining,
                    arr_h,
                    transport_panel_width: &mut self.transport_panel_width,
                    drag_ended: &mut self.layout_needs_save,
                },
                renderer,
                render_ctx,
                cfg,
                &mut self.last_cursor_tick,
                self.audio_state.handle.as_ref(),
                &self.audio_library,
                &mut request_pianoroll,
                &mut self.track_selection_anchor,
                &mut arr_drag_delta,
                &mut arr_eraser_rect,
                &mut arr_scale_span,
                &mut arr_riff_steps,
                &mut self.info_content,
                &mut self.right_tab,
                &mut needs_audio_rebuild,
                &mut needs_audio_notify,
                sel_hint.as_ref(),
                &mut automation_picker_req,
                &self.mix.track_display,
            );
            // guard dropped here
        }
        if request_pianoroll {
            crate::chrome::mode_bar::set_pianoroll_visible(
                &mut self.show_pianoroll_in_arrange,
                &mut self.show_bottom_dock,
                &mut self.show_mixer_panel,
                true,
            );
        }
        // 音轨结构变化 → drop 旧引擎（下一帧 rebuild_audio_if_needed 重建）。
        if needs_audio_rebuild {
            self.teardown_audio();
        }
        // 右键「添加插件参数自动化…」：打开参数选择窗口。
        if let Some(track_idx) = automation_picker_req {
            self.open_automation_picker(ui.ctx(), track_idx);
        }
        // 自动化内容变化（AR lane 增删）：通知音频引擎 model 已变。
        if needs_audio_notify {
            self.notify_audio_model_changed();
        }
        // AR 橡皮擦（guard 已释放，可借 &mut self）。
        if let Some((t_start, t_end, track_lo, track_hi)) = arr_eraser_rect {
            let mut sel = yinhe_core::Selection::default();
            sel.add_rect_track(
                t_start as u32,
                t_end as u32,
                0,
                yinhe_types::MAX_KEY,
                track_lo as u16,
                track_hi as u16,
            );
            if let Some(idx) = self.workspace.active_doc {
                self.workspace.documents[idx].edit.selected = sel;
                self.with_undo(t!("undo.eraser_arrange").as_ref(), |doc| {
                    doc.delete_selected()
                });
            }
        }
        // AR 拖动/复制（guard 已释放）。
        if let Some((delta_ticks, delta_tracks, alt)) = arr_drag_delta {
            self.handle_arr_drag(delta_ticks, delta_tracks, alt);
        }
        // AR 选区整体变速 / Riff 重复。
        self.handle_arr_selection_scale(arr_scale_span);
        self.handle_arr_riff_repeat(arr_riff_steps);
    }

    /// 欢迎内容（无工程占位；与启动欢迎窗口同一实现）。
    /// 主区域必须有画面控件——clear_color 为全透明，没有工程时会露出桌面。
    pub(super) fn show_welcome_content(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        let area = layout.remaining;
        // 主区域铺底（clear_color 为全透明）+ 欢迎内容。
        ui.painter().rect_filled(area, 0.0, crate::theme::app_bg());
        let action = {
            let recent = &self.audio_settings.recent_files;
            let times = &self.audio_settings.recent_file_times;
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                crate::welcome::welcome_content(ui, recent, times)
            })
            .inner
        };
        if let Some(action) = action {
            self.handle_welcome_action(action, ui.ctx());
        }
    }

    /// Show the pianoroll split area, including the split handle and pianoroll view.
    /// 浮动窗口形态由 `app::floating` 以合成布局复用本函数。
    pub(in crate::app) fn show_pianoroll_split(
        &mut self,
        ui: &mut egui::Ui,
        layout: &LayoutInfo,
        idx: usize,
        is_playing: bool,
        follow_mode: &mut crate::view_interaction::FollowMode,
        sel_hint: Option<&SelHintInfo>,
    ) {
        // ── 顶部工具栏（仅 PR 带内）──
        // 工具横向常驻在钢琴卷帘顶部，不再占用左侧竖条。
        let tool_rect = egui::Rect::from_min_max(
            egui::pos2(layout.remaining.min.x, layout.bottom_y),
            egui::pos2(
                layout.remaining.max.x,
                (layout.bottom_y + crate::theme::PR_TOOL_BAR_H).min(layout.remaining.max.y),
            ),
        );
        // Pianoroll GPU view
        let auto_wgpu_state = self.render_ctx.wgpu_state().clone();
        while self.controller_renderers.len() <= idx {
            self.controller_renderers.push(Vec::new());
        }

        let mut auto_edit_events: Vec<crate::piano_view::automation_panel::AutomationEdit> =
            Vec::new();
        let mut velocity_edits: Vec<yinhe_types::VelocityEdit> = Vec::new();

        let mut open_scale_dialog = false;
        let (
            piano_event,
            note_drag_delta,
            pencil_note_drag,
            note_resize_delta,
            selection_scale_delta,
            riff_repeat,
            preview_reqs,
        ) = {
            let mut guard =
                crate::app::main_loop::ReplaceGuard::new(&mut self.workspace.documents[idx]);
            let doc = guard.as_mut();
            let midi_source: Option<&dyn yinhe_types::NoteSource> = Some(doc.data.model.as_ref());
            let piano_rect = egui::Rect::from_min_max(
                egui::pos2(layout.remaining.min.x, tool_rect.max.y),
                layout.remaining.max,
            );

            let mut event = None;
            let mut note_drag_delta: Option<(i64, i32, bool)> = None;
            let mut pencil_note_drag: Option<crate::piano_view::PencilNoteDrag> = None;
            let mut note_resize_delta: Option<(crate::piano_view::ResizeSide, i64)> = None;
            let mut selection_scale_delta: Option<u64> = None;
            let mut riff_repeat: Option<(i64, i64)> = None;
            let mut preview_reqs: Vec<crate::piano_view::PreviewReq> = Vec::new();
            // 顶部工具栏（需要文档视图状态：键数 / Scale / 调式选择）。
            let key_sig = crate::widgets::timecode::current_key_sig(doc);
            let fold_mask = key_sig.map(|(root, scale)| scale.pitch_classes(root));
            let scale_label = key_sig
                .map(|(root, scale)| {
                    format!(
                        "{} {}",
                        yinhe_types::NOTE_NAMES[(root % 12) as usize],
                        scale.english_name()
                    )
                })
                .unwrap_or_else(|| "--".to_string());
            if crate::chrome::tool_bar::show(
                ui,
                tool_rect,
                &mut self.active_tool,
                &mut doc.edit.pianoroll_view,
                fold_mask,
                &scale_label,
                &mut doc.edit.pr_clip_mode,
                self.audio_settings.clip_enabled,
            ) {
                open_scale_dialog = true;
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                let _piano_total_start = if yinhe_memtrace::perf_probe::enabled() {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                // PR 显示 = PR 显示音轨勾选集合（track_pianoroll_visible），
                // 主音轨强制可见。与 AR 显隐（track_visible）分离，互不影响。
                // 同一规则由 EditState::pr_visible 提供（密度预览共用）。
                let main = doc.edit.main_track();
                // PR 编辑目标轨 = 主音轨（= 选中集合最小者）。无选中 = None：
                // 铅笔/选框/自动化一律不响应操作（不再回退到第一个非 Conductor 轨）。
                // 锁定轨不能作为编辑目标（铅笔/录音/粘贴都依赖 write_track）。
                let write_track = main.filter(|&t| {
                    !doc.edit
                        .track_locked
                        .get(t as usize)
                        .copied()
                        .unwrap_or(false)
                });
                let pr_visible: Vec<bool> = doc.edit.pr_visible();
                // CLIP 模式：时间窗由 piano_view 夹到 Active Clip，其他轨灰显并锁定；
                // 这样只有当前 Clip 所在轨可编辑，其他轨点击/框选直接被忽略。
                let clip_mode = doc.edit.pr_clip_mode;
                // PR 其他轨道去色：普通模式保留用户勾选轨；CLIP 模式只保留当前轨。
                let gray_others = self.audio_settings.gray_other_tracks || clip_mode;
                let pr_colors = if gray_others {
                    let keep: Vec<bool> = (0..doc.edit.track_cache.colors.len())
                        .map(|i| {
                            if clip_mode {
                                // CLIP 模式统一灰度；只有当前轨可编辑，其他轨锁定。
                                false
                            } else {
                                doc.edit.track_selected.contains(&(i as u16))
                            }
                        })
                        .collect();
                    yinhe_editor_core::document::gray_other_track_colors(
                        &doc.edit.track_cache.colors,
                        &keep,
                    )
                } else {
                    doc.edit.track_cache.colors.clone()
                };
                // 锁定掩码：CLIP 模式下除当前轨外全部只读。
                let pr_locked: Vec<bool> = (0..doc.data.model.tracks.len())
                    .map(|i| {
                        (clip_mode && Some(i as u16) != write_track)
                            || doc.edit.track_locked.get(i).copied().unwrap_or(false)
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
                // Get automation lanes：以主音轨为唯一编辑目标。
                // Conductor 的 Tempo 由单独的 tempo_lane 传入，此处 lanes 仍按主音轨的
                // automation_lanes 提供（与其它轨道一致，Conductor 也可显示其 CC lanes）。
                let automation_lanes: Vec<yinhe_types::AutomationLane> = {
                    let edit_trk = doc
                        .edit
                        .main_track()
                        // 用 pr_visible 而非原始 track_visible：主音轨强制可见，
                        // 取消勾选显示不应使 automation 编辑静默失效。
                        .filter(|&t| pr_visible.get(t as usize).copied().unwrap_or(false));
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
                // 与音符显示逻辑一致（track_pianoroll_visible ∪ 主音轨）。
                let automation_render_lanes: Vec<&yinhe_types::AutomationLane> = doc
                    .data
                    .model
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| pr_visible.get(*i).copied().unwrap_or(false))
                    .flat_map(|(_, t)| t.automation_lanes.iter())
                    .collect();
                // 新建音符默认长度：该轨 gate 记忆，无记忆回退量化间隔。
                // 量化档位先解析：Auto（自适应）按当前 PR 缩放出具体网格；
                // Off（关闭吸附）没有网格，默认长度回退 1/16 音符间隔。
                let (sig_num, sig_den) = doc.data.model.tempo_map.time_sig_default;
                let pr_quantize = doc.edit.quantize_pianoroll.resolved_with_time_sig(
                    doc.edit.pianoroll_view.base.pixels_per_tick,
                    tpb,
                    sig_num,
                    sig_den,
                    true,
                );
                let default_gate = write_track.map(|t| {
                    let fallback = if pr_quantize == QuantizePreset::Off {
                        QuantizePreset::Fraction(1, 16).tick_interval(tpb)
                    } else {
                        pr_quantize.tick_interval(tpb)
                    };
                    doc.edit.default_gate(t, fallback)
                });
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
                    selection_scale_delta: &mut selection_scale_delta,
                    riff_repeat: &mut riff_repeat,
                    velocity_edits: &mut velocity_edits,
                    preview_reqs: &mut preview_reqs,
                };
                // PR 滚动条预览跟随 PR 专属显隐（track_pianoroll_visible ∪ 主音轨），
                // 与 PR 画布一致；AR/通道 AR 用 doc.density（track_visible）。
                let density_preview = crate::widgets::scrollbar::DensityPreview {
                    density: doc.density_pr.view(),
                    track_colors: &pr_colors,
                };
                event = piano_view::show(
                    ui,
                    ui.available_size(),
                    &mut self.pianoroll,
                    &mut self.pr_note_cache,
                    &mut self.render_ctx,
                    self.render_thread.as_ref(),
                    &mut doc.edit.pianoroll_view,
                    &mut doc.edit.event_rows_pr,
                    &mut doc.edit.playback,
                    doc.edit.pr_clip_mode,
                    &mut self.last_cull_revision,
                    &mut self.last_cull_revision_only,
                    &mut self.last_hidden_hash,
                    &mut self.last_tv_hash,
                    &mut self.last_hidden_keys,
                    &mut self.cull_rebuild,
                    midi_source,
                    Some(&doc.data.model),
                    &mut doc.edit.selected,
                    &pr_visible,
                    Some(&density_preview),
                    &pr_locked,
                    &pr_colors,
                    &mut doc.edit.cursor_tick,
                    is_playing,
                    pr_quantize,
                    &mut doc.edit.quantize_pianoroll,
                    tpb,
                    Some((tpb, ts_num, ts_den, &ts_events)),
                    &doc.data.model.conductor.key_sig,
                    &mut self.piano_last_cursor_tick,
                    follow_mode,
                    &self.active_tool,
                    auto_ctx,
                    self.audio_settings.min_border_width,
                    crate::theme::CONTENT_OPACITY,
                    self.audio_settings.note_outline,
                    self.audio_settings.use_gpu_cull,
                    self.audio_settings.lod_enabled,
                    &doc.data.model.conductor.tempo,
                    &mut doc.edit.sel_rect,
                    &mut doc.edit.pen_path,
                    &mut doc.edit.scissors_line,
                    &doc.edit.track_selected,
                    doc.edit.track_cache.conductor_idx,
                    write_track,
                    doc.data.revision,
                    doc.data.note_revisions(),
                    &mut feedback,
                    sel_hint,
                    self.audio_settings.quick_delete_mode,
                    default_gate,
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
                selection_scale_delta,
                riff_repeat,
                preview_reqs,
            )
        };

        if open_scale_dialog {
            let doc = &self.workspace.documents[idx];
            let cur = crate::widgets::timecode::current_key_sig(doc);
            let override_on = doc.edit.key_sig_override.is_some();
            let use_events = doc.edit.key_sig_use_events;
            self.scale_dialog.open_at(cur, override_on, use_events);
        }

        // 音符听觉预览（铅笔新建/拖拽、选框拖拽触发）。
        // PR 拖动/铅笔：替换式预览（拖动时不叠加，只响当前组）。
        self.send_note_previews(&preview_reqs, true);

        // Handle piano-view events
        if let Some(event) = piano_event {
            use crate::piano_view::PianoViewEvent;
            match event {
                PianoViewEvent::ClipResize {
                    track,
                    clip_id,
                    start_tick,
                    end_tick,
                } => {
                    if let Some(idx) = self.workspace.active_doc {
                        let doc = &mut self.workspace.documents[idx];
                        let before = doc.capture_snapshot();
                        if let Some(action) = doc.resize_clip(track, clip_id, start_tick, end_tick)
                        {
                            doc.push_undo(action, "undo.move_notes", before);
                        }
                    }
                }
                PianoViewEvent::SelectionAction(action) => {
                    self.handle_selection_action(action);
                }
                PianoViewEvent::AddNote { track, note } => {
                    // 新音符默认力度 = 该音轨最近一次 velocity 修改值（无记录 100）。
                    let mut note = note;
                    if let Some(idx) = self.workspace.active_doc {
                        note.velocity = self.workspace.documents[idx].edit.default_velocity(track);
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
                    let Some(idx) = self.workspace.active_doc else {
                        return;
                    };
                    let mut sel = yinhe_core::Selection::default();
                    sel.add_rect_track(t_start, t_end, key_lo, key_hi, track_lo, track_hi);
                    self.workspace.documents[idx].edit.selected = sel;
                    self.with_undo(t!("undo.eraser_delete").as_ref(), |doc| {
                        doc.delete_selected()
                    });
                }
                PianoViewEvent::QuickDelete {
                    track,
                    start_tick,
                    key,
                } => {
                    let Some(idx) = self.workspace.active_doc else {
                        return;
                    };
                    // 快速删除：按 (track, start_tick, key) 定位单个音符
                    // 可见性/选中过滤必须与命中端 hit_test_note（用 pr_visible：
                    // 主音轨强制可见）一致，否则主音轨被隐藏时「命中却删不掉」。
                    let edit = &self.workspace.documents[idx].edit;
                    let visible = edit
                        .track_pianoroll_visible
                        .get(track as usize)
                        .copied()
                        .unwrap_or(true);
                    let is_main = edit.main_track() == Some(track);
                    if !visible && !is_main {
                        return;
                    }
                    if !edit.track_selected.is_empty() && !edit.track_selected.contains(&track) {
                        return;
                    }
                    self.with_undo(t!("undo.delete_notes").as_ref(), |doc| {
                        doc.delete_single_note(track, start_tick, key)
                    });
                }
                PianoViewEvent::QuickErase { notes } => {
                    self.quick_erase_notes(notes);
                }
                PianoViewEvent::AddNotes { track, notes } => {
                    self.add_notes_with_undo(track, notes);
                }
                PianoViewEvent::MarkerEdit(edit) => {
                    if let Some(idx) = self.workspace.active_doc {
                        crate::marker_ops::apply_marker_edit(
                            &mut self.workspace.documents[idx],
                            edit,
                        );
                    }
                }
            }
        }

        // Handle note drag
        self.handle_note_drag(note_drag_delta);

        // Handle note resize (selection edge drag)
        self.handle_note_resize(note_resize_delta);

        // Handle selection overall time stretch / Riff repeat.
        self.handle_selection_scale(selection_scale_delta);
        self.handle_riff_repeat(riff_repeat);

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
}
