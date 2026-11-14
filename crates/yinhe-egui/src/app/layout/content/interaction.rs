//! 主内容区的拖动/笔划交互落地：音符、框选、AR、力度与自动化。

use super::*;

impl App {
    ///
    /// 每帧渲染前检查各视图的选框数量与共享选区状态：
    /// - 某视图**新增**了选框（框选提交，含 shift/cmd 加选追加）→ 清除其他视图的选框；
    /// - 共享选区被**清空**（AR/PR 在 press 时清空 selected，即开始新的框选/空白点击）
    ///   → 立即清除全部视图的选框（发起方已自行清空自己的选框）。
    ///
    /// 清除时同步从共享 `Selection` 中精确移除对应视图的矩形，避免误伤其他视图的选区。
    /// `selected` 被三个视图共享，不能整体 clear()。
    pub(super) fn enforce_sel_rect_exclusivity(&mut self, idx: usize) {
        let doc = &mut self.workspace.documents[idx];

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
        // AM 选框计数 = PR 自动化面板 + AR 展开 lane（三视图互斥一体）。
        let am_count: usize = doc
            .edit
            .controller_panels
            .iter()
            .map(|p| p.anchor_sel_rects.len())
            .sum::<usize>()
            + doc
                .edit
                .arr_am_views
                .values()
                .map(|v| v.anchor_sel_rects.len())
                .sum::<usize>();

        let arr_gained = arr_count > self.prev_arr_count;
        let pr_gained = pr_count > self.prev_pr_count;
        let am_gained = am_count > self.prev_am_count;
        let chan_ar_count = doc.edit.chan_ar_sel.len();
        let chan_ar_gained = chan_ar_count > self.prev_chan_ar_count;
        // AR/PR/通道 AR 在 press 开始新框选/空白点击时会清空 selected。
        let selection_cleared = doc.edit.selected.is_empty() && self.prev_selected_nonempty;

        // 新选框只可能来自一个视图（单鼠标交互），各清除条件互不重叠。
        // 例外：Ctrl+A 的 `select_all_ar` 会同时填充 AR 与通道 AR 两套框
        //（程序性设置），不能把它们当作互相竞争的新选框清掉。
        let select_all_both = arr_gained && chan_ar_gained;
        let clear_arr =
            selection_cleared || pr_gained || am_gained || (chan_ar_gained && !select_all_both);
        let clear_pr = selection_cleared || arr_gained || am_gained || chan_ar_gained;
        let clear_am = selection_cleared || arr_gained || pr_gained || chan_ar_gained;
        let clear_chan_ar =
            selection_cleared || pr_gained || (arr_gained && !select_all_both) || am_gained;

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
            for v in doc.edit.arr_am_views.values_mut() {
                v.anchor_sel_rects.clear();
            }
        }
        if clear_chan_ar {
            // 通道 AR 选框的轨道集合移除（按共享 Selection 的精确矩形语义移除）。
            let ca_rects: Vec<(u32, u32, u16, u16)> = doc
                .edit
                .chan_ar_sel
                .iter()
                .flat_map(|s| {
                    s.tracks
                        .iter()
                        .map(move |&t| (s.t_start as u32, s.t_end as u32, t, t))
                })
                .collect();
            doc.edit.chan_ar_sel.clear();
            for (ts, te, lo, hi) in ca_rects {
                doc.edit.selected.remove_rects_track(&[(ts, te, lo, hi)]);
            }
        }

        // 以清除后的状态更新 prev，供下一帧比较。
        self.prev_arr_count = doc.edit.arr_sel_rect.len();
        self.prev_pr_count = doc.edit.sel_rect.rects.len();
        self.prev_chan_ar_count = doc.edit.chan_ar_sel.len();
        self.prev_am_count = doc
            .edit
            .controller_panels
            .iter()
            .map(|p| p.anchor_sel_rects.len())
            .sum::<usize>()
            + doc
                .edit
                .arr_am_views
                .values()
                .map(|v| v.anchor_sel_rects.len())
                .sum::<usize>();
        self.prev_selected_nonempty = !doc.edit.selected.is_empty();
    }

    /// 把 automation 面板 velocity 笔划产生的编辑应用到 Document（一笔 = 一个 undo entry）。
    /// 若存在 PR 选框，仅允许修改选框内且在选定轨道内的音符力度。
    pub(super) fn handle_velocity_edits(&mut self, edits: &[yinhe_types::VelocityEdit]) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        // 选框过滤：存在选框时仅保留落在选框且轨道选中的音符
        let filtered: Vec<yinhe_types::VelocityEdit> = {
            let doc = &self.workspace.documents[idx];
            if doc.edit.sel_rect.is_empty() {
                edits.to_vec()
            } else {
                edits
                    .iter()
                    .copied()
                    .filter(|e| {
                        if !doc.edit.track_selected.is_empty()
                            && !doc.edit.track_selected.contains(&e.track)
                        {
                            return false;
                        }
                        doc.edit.sel_rect.rects.iter().any(|&(ts, te, kl, kh)| {
                            let st = e.start_tick as f64;
                            st >= ts && st < te && e.key >= kl && e.key <= kh
                        })
                    })
                    .collect()
            }
        };
        if filtered.is_empty() {
            return;
        }
        let doc = &mut self.workspace.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.set_notes_velocity(&filtered) {
            doc.edit.pianoroll_view.base.dirty = true;
            doc.push_undo(action, t!("undo.edit_velocity").as_ref(), before);
            // 纯音符 velocity 修改：只换模型引用，不重建 CC，不 chase
            self.notify_notes_changed();
        }
    }

    /// 把 automation 面板产生的编辑事件应用到 Document，push undo，并通知音频线程。
    pub(super) fn handle_automation_edits(
        &mut self,
        edits: Vec<crate::piano_view::automation_panel::AutomationEdit>,
    ) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if !actions.is_empty() {
            doc.edit.pianoroll_view.base.dirty = true;
            push_automation_actions(doc, actions, t!("undo.edit_automation").as_ref(), before);
            self.notify_audio_model_changed();
        }
    }

    /// Handle note drag — called once on release.
    /// 处理事件浏览器的跳转请求：设置 cursor_tick、切到 piano roll 视图、
    /// 选中目标音轨（音符/automation 事件）、滚动到中心。
    pub(super) fn handle_jump_request(
        &mut self,
        req: crate::right_panel::event_browser::JumpRequest,
        layout: &LayoutInfo,
    ) {
        // 1. 切到 piano roll 视图（如果当前不在；浮动 PR 窗口打开时无需切换）
        if !self.float_pianoroll
            && !self
                .view_mode
                .show_pianoroll(self.show_pianoroll_in_arrange)
        {
            self.view_mode = crate::chrome::mode_bar::ViewMode::Edit;
        }

        // 2. 选中目标音轨（音符/CC/PB/PC 事件需要）。
        // Conductor 跳过（不作为写入目标）；Tempo 编辑不依赖选中。
        if let Some((track, _key)) = req.note
            && let Some(idx) = self.workspace.active_doc
            && self.workspace.documents[idx].edit.track_cache.conductor_idx != Some(track)
        {
            let sel = &mut self.workspace.documents[idx].edit.track_selected;
            sel.clear();
            sel.insert(track);
        }

        // 3. 设置 cursor_tick
        if let Some(idx) = self.workspace.active_doc {
            self.workspace.documents[idx].edit.cursor_tick = Some(req.tick as f64);
        }

        // 4. 滚动到中心（参考 follow.rs Page 模式公式）
        // remaining 已由 Panel 扣掉右栏/dock，此处不能再减右栏宽（原双重扣减）。
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let view = &mut self.workspace.documents[idx].edit.pianoroll_view;
        let viewport_w = layout.remaining.width();
        let content_w = viewport_w - view.base.left_panel_width;
        let target_x = req.tick as f32 * view.base.pixels_per_tick;
        view.base.scroll_x = (target_x - content_w * 0.5).max(0.0);
        view.base.dirty = true;
    }

    pub(super) fn handle_note_drag(&mut self, note_drag_delta: Option<(i64, i32, bool)>) {
        if let Some((delta_ticks, delta_keys, alt)) = note_drag_delta {
            let Some(idx) = self.workspace.active_doc else {
                return;
            };
            let doc = &mut self.workspace.documents[idx];
            let mut before = doc.capture_snapshot();
            // 组件在松手帧已把可视选框移到新位置，快照必须用拖动前的位置，
            // 否则 undo 后音符回到旧位置、选框却留在新位置。
            if let Some(rects) = doc.edit.sel_rect.take_before_last_commit() {
                before.sel_rect.rects = rects;
            }
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
                let action = doc.with_clip_normalization(action);
                doc.edit.pianoroll_view.base.dirty = true;
                doc.push_undo(action, &label, before);
                // 纯音符移动/复制：只换模型引用，不重建 CC，不 chase
                self.notify_notes_changed();
            }
        }
    }

    /// Handle note resize: shift one edge of all selected notes by `dt` ticks.
    /// 选框工具边缘拖动伸缩：所有选中音符的 start_tick (Left) 或 end_tick (Right) 统一偏移。
    pub(super) fn handle_note_resize(
        &mut self,
        note_resize_delta: Option<(crate::piano_view::ResizeSide, i64)>,
    ) {
        if let Some((side, dt)) = note_resize_delta {
            let Some(idx) = self.workspace.active_doc else {
                return;
            };
            let doc = &mut self.workspace.documents[idx];
            let ppq = doc.data.model.meta.ppq;
            let (num, den_pow) = doc.data.model.tempo_map.time_sig_default;
            let q = doc.edit.quantize_pianoroll.resolved_with_time_sig(
                doc.edit.pianoroll_view.base.pixels_per_tick,
                ppq,
                num,
                den_pow,
                true,
            );
            let min_gate = q.tick_interval(ppq).max(1);
            let mut before = doc.capture_snapshot();
            if let Some(rects) = doc.edit.sel_rect.take_before_last_commit() {
                before.sel_rect.rects = rects;
            }
            if let Some(action) = doc.resize_selected_notes_min_gate(side, dt, min_gate) {
                let action = doc.with_clip_normalization(action);
                doc.edit.pianoroll_view.base.dirty = true;
                doc.push_undo(action, t!("undo.resize_notes").as_ref(), before);
                self.notify_notes_changed();
            }
        }
    }

    /// Handle selection overall time stretch (right-edge square).
    pub(super) fn handle_selection_scale(&mut self, new_span: Option<u64>) {
        let Some(new_span) = new_span else {
            return;
        };
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.rescale_selection_span(new_span) {
            doc.edit.pianoroll_view.base.dirty = true;
            doc.push_undo(action, t!("undo.rescale_span").as_ref(), before);
            self.notify_notes_changed();
        }
    }

    /// Handle Riff repeat (bottom-right square): extend by `(extent_ticks, sy)`.
    pub(super) fn handle_riff_repeat(&mut self, steps: Option<(i64, i64)>) {
        let Some((extent, extent_y)) = steps else {
            return;
        };
        if extent == 0 && extent_y == 0 {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let Some(&(t0, t1, k0, k1)) = doc.edit.sel_rect.rects.first() else {
            return;
        };
        let sel_start = t0.round() as i64;
        let sel_end = t1.round() as i64;
        let before = doc.capture_snapshot();
        if let Some(action) = doc.repeat_selected_keys_extent(
            sel_start,
            sel_end,
            extent,
            i64::from(k0),
            i64::from(k1) + 1,
            extent_y,
        ) {
            doc.edit.pianoroll_view.base.dirty = true;
            doc.push_undo(action, t!("undo.duplicate_notes").as_ref(), before);
            self.notify_notes_changed();
        }
    }

    /// Handle pencil note drag updates (move or resize a single note).
    pub(super) fn handle_pencil_note_drag(
        &mut self,
        drag: Option<crate::piano_view::PencilNoteDrag>,
    ) {
        let Some(drag) = drag else { return };
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.pencil_drag_note(&drag) {
            doc.edit.pianoroll_view.base.dirty = true;
            let label = match &drag {
                crate::piano_view::PencilNoteDrag::Move { .. } => t!("undo.move_note").to_string(),
                _ => t!("undo.resize_note").to_string(),
            };
            doc.push_undo(action, &label, before);
            // 纯音符拖动/缩放：只换模型引用，不重建 CC，不 chase
            self.notify_notes_changed();
        }
    }

    /// Handle AR drag: move (or Alt-copy) selected notes + automation events
    /// by `(delta_ticks, delta_tracks)`. Single atomic operation = single undo step.
    pub(super) fn handle_arr_drag(&mut self, delta_ticks: i64, delta_tracks: i32, alt: bool) {
        if delta_ticks == 0 && delta_tracks == 0 {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let mut before = doc.capture_snapshot();
        // AR 交互在松手帧已把选框移到目标位置；按 delta 反推拖动前的位置，
        // 让 undo 后选框与音符一起回到原处。
        before.arr_sel_rect = before
            .arr_sel_rect
            .iter()
            .map(|&(t0, t1, lo, hi)| {
                (
                    t0 - delta_ticks as f64,
                    t1 - delta_ticks as f64,
                    lo.saturating_add_signed(-(delta_tracks as isize)),
                    hi.saturating_add_signed(-(delta_tracks as isize)),
                )
            })
            .collect();
        let (action, label) = if alt {
            (
                doc.duplicate_selected_arrange(delta_ticks, delta_tracks),
                t!("undo.duplicate_in_arrange").to_string(),
            )
        } else {
            (
                doc.move_selected_arrange(delta_ticks, delta_tracks),
                t!("undo.move_in_arrange").to_string(),
            )
        };
        if let Some(action) = action {
            doc.edit.arrange_view.base.dirty = true;
            doc.push_undo(action, &label, before);
            self.notify_audio_model_changed();
        }
    }

    /// Handle AR selection overall time stretch.
    pub(super) fn handle_arr_selection_scale(&mut self, new_span: Option<u64>) {
        let Some(new_span) = new_span else {
            return;
        };
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.rescale_selection_span_with_automation(new_span) {
            doc.edit.arrange_view.base.dirty = true;
            doc.push_undo(action, t!("undo.rescale_span").as_ref(), before);
            self.notify_notes_changed();
        }
    }

    /// Handle AR Riff repeat: extend by `(extent_ticks, sy)`.
    pub(super) fn handle_arr_riff_repeat(&mut self, steps: Option<(i64, i64)>) {
        let Some((extent, extent_y)) = steps else {
            return;
        };
        if extent == 0 && extent_y == 0 {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let Some(&(t0, t1, r0, r1)) = doc.edit.arr_sel_rect.first() else {
            return;
        };
        let sel_start = t0.round() as i64;
        let sel_end = t1.round() as i64;
        let before = doc.capture_snapshot();
        if let Some(action) = doc.repeat_selected_tracks_extent(
            sel_start,
            sel_end,
            extent,
            r0 as i64,
            r1 as i64 + 1,
            extent_y,
        ) {
            doc.edit.arrange_view.base.dirty = true;
            doc.push_undo(action, t!("undo.duplicate_notes").as_ref(), before);
            self.notify_notes_changed();
        }
    }
}
