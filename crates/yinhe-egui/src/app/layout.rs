use eframe::egui;
use rust_i18n::t;

use crate::app::App;
use crate::arrange;
use crate::piano_view;
use crate::right_panel::automation_undo::push_automation_actions;
use crate::right_panel::info_panel::selection::selected_am_events;
use yinhe_editor_core::audio_settings::LayoutSettings;
use yinhe_editor_core::batch_ops::summarize_selected;
use yinhe_types::AnchorSelRect;
use yinhe_types::NoteSource;
use yinhe_types::time_format::{format_tick_bar_beat_with_time_sig, measure_ticks};

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
        // MIX 模式与（无视图但已判断文档分支更下面的）无工程状态都铺背景：
        // 主区域绝不能是全透明（clear_color 为全透明，透明模式下会露出桌面）。
        let has_view = self.view_mode.show_transport()
            || self
                .view_mode
                .show_pianoroll(self.show_pianoroll_in_arrange);
        if !has_view {
            // MIX 模式：有工程时进混音台，否则只铺背景。
            if self.view_mode == crate::chrome::mode_bar::ViewMode::Mix && self.active_doc.is_some()
            {
                crate::mix::show(self, ui, layout.remaining);
            } else {
                ui.painter()
                    .rect_filled(layout.remaining, 0.0, crate::theme::track_bg());
            }
            return;
        }
        let Some(idx) = self.active_doc else {
            // 有视图控件（AR/PR 模式）但未打开工程：铺占位面板而非透明。
            self.show_workspace_placeholder(ui, layout);
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
        // AR 自动化 lane 增删/编辑 → notify_audio_model_changed（不重建引擎）。
        let mut needs_audio_notify = false;
        let (arr_drag_delta, arr_eraser_rect, arr_quantize, float_panel_req): (
            Option<crate::arrange::ArrDragDelta>,
            Option<crate::arrange::ArrSelRect>,
            Option<yinhe_editor_core::quantize::QuantizePreset>,
            Option<crate::right_panel::FloatPanel>,
        ) = if self.view_mode.show_transport() {
            let mut request_pianoroll = false;
            let mut arr_drag_delta: Option<crate::arrange::ArrDragDelta> = None;
            let mut arr_eraser_rect: Option<crate::arrange::ArrSelRect> = None;
            let mut float_panel_req: Option<crate::right_panel::FloatPanel> = None;
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
                    drag_ended: &mut self.layout_needs_save,
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
                &mut self.right_tab,
                &mut needs_audio_rebuild,
                &mut needs_audio_notify,
                &mut self.status_hint,
                sel_hint.as_ref(),
                &mut float_panel_req,
            );
            if request_pianoroll {
                self.show_pianoroll_in_arrange = true;
            }
            (
                arr_drag_delta,
                arr_eraser_rect,
                arr_quantize,
                float_panel_req,
            ) // guard dropped here
        } else {
            (None, None, None, None)
        };
        // 方案 A：音轨结构变化（add/remove track）→ drop 旧引擎。
        // ChannelLayout 在引擎创建时冻结，旧引擎无法 dispatch 新增通道。
        // 必须在 guard 被释放后调用 —— teardown_audio 借用 &mut self，
        // 与 ReplaceGuard 借用的 &mut self.documents[idx] 冲突。
        // 下一帧 rebuild_audio_if_needed 会用新 model 重新 spawn 引擎和 ChannelLayout。
        if needs_audio_rebuild {
            self.teardown_audio();
        }
        // 右键「音轨属性」/ 其他浮动面板请求：设浮窗并收起侧栏 Info（互斥）。
        if let Some(panel) = float_panel_req {
            self.set_float_panel(Some(panel));
        }
        // 自动化内容变化（AR lane 增删 / AM 事件编辑）：与 PR 的
        // handle_automation_edits 同路径，通知音频引擎 model 已变。
        if needs_audio_notify {
            self.notify_audio_model_changed();
        }

        // Handle AR eraser (guard is dropped, no outstanding borrow on self.documents)
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
            let Some(idx) = self.active_doc else { return };
            self.documents[idx].edit.selected = sel;
            self.with_undo(t!("undo.eraser_arrange").as_ref(), |doc| {
                doc.delete_selected()
            });
        }

        // Handle AR drag after guard is dropped (no outstanding borrow on self.documents)
        if let Some((delta_ticks, delta_tracks, alt)) = arr_drag_delta {
            self.handle_arr_drag(delta_ticks, delta_tracks, alt);
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

    /// 未打开工程时的占位面板：铺满背景 + 居中卡片（与 MIX 模式同思路，
    /// 主区域始终有画面控件；否则 clear_color 为全透明，没有工程时会露出桌面）。
    fn show_workspace_placeholder(&self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        let area = layout.remaining;
        ui.painter().rect_filled(area, 0.0, crate::theme::app_bg());

        let card_size = egui::vec2(340.0, 150.0);
        let card_rect = egui::Rect::from_center_size(area.center(), card_size);
        ui.painter()
            .rect_filled(card_rect, 10.0, crate::theme::control_bg());
        ui.painter().rect_stroke(
            card_rect,
            10.0,
            egui::Stroke::new(1.0, crate::theme::line_fg()),
            egui::StrokeKind::Inside,
        );

        let icon = egui_material_icons::icons::ICON_MUSIC_NOTE;
        ui.scope_builder(egui::UiBuilder::new().max_rect(card_rect), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                ui.add(
                    egui::Label::new(
                        icon.rich_text()
                            .size(crate::theme::ICON_FONT_XL)
                            .color(crate::theme::text_muted()),
                    )
                    .selectable(false),
                );
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(t!("common.no_document"))
                        .size(crate::theme::SUB_TITLE_FONT)
                        .color(crate::theme::text_primary()),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(t!("workspace.placeholder_hint"))
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_disabled()),
                );
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(t!("workspace.placeholder_shortcuts"))
                        .size(crate::theme::SMALL_LABEL_FONT)
                        .color(crate::theme::text_label()),
                );
            });
        });
    }
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
            for v in doc.edit.arr_am_views.values_mut() {
                v.anchor_sel_rects.clear();
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
            .sum::<usize>()
            + doc
                .edit
                .arr_am_views
                .values()
                .map(|v| v.anchor_sel_rects.len())
                .sum::<usize>();
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
            // 整条带作为 handle（绘制+交互同一层，避免叠两层同色填充）
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
            let h_split_resp =
                crate::widgets::split_handle::horizontal(ui, "__h_split__", h_split_rect);
            if h_split_resp.dragged() {
                let total_y = layout.remaining.size().y;
                let delta = h_split_resp.drag_delta().y;
                self.arr_split = ((layout.arr_h + delta) / total_y)
                    .clamp(crate::theme::SPLIT_CLAMP_MIN, crate::theme::SPLIT_CLAMP_MAX);
            }
            if h_split_resp.double_clicked() {
                // 双击分割线 → 还原 AR/PR 默认分割比例
                self.arr_split = LayoutSettings::default().arr_split;
            }
            if h_split_resp.drag_stopped() || h_split_resp.double_clicked() {
                self.layout_needs_save = true;
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

        let (
            piano_event,
            note_drag_delta,
            pencil_note_drag,
            note_resize_delta,
            preview_reqs,
            bar_events,
        ) = {
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
            let mut bar_events: Vec<crate::piano_view::control_bar::PrBarEvent> = Vec::new();
            ui.scope_builder(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                let _piano_total_start = if yinhe_memtrace::perf_probe::enabled() {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                // PR 显示 = PR 显示音轨勾选集合（track_pianoroll_visible），
                // 主音轨强制可见。与 AR 显隐（track_visible）分离，互不影响。
                let main = doc.edit.main_track();
                // PR 编辑目标轨 = 主音轨（= 选中集合最小者）。无选中 = None：
                // 铅笔/选框/自动化一律不响应操作（不再回退到第一个非 Conductor 轨）。
                // 提前计算避开后面的可变借用。
                let write_track = main;
                let pr_visible: Vec<bool> = (0..doc.edit.track_pianoroll_visible.len())
                    .map(|i| doc.edit.track_pianoroll_visible[i] || main == Some(i as u16))
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
                // Conductor 不在此处提供 lanes（Tempo 由单独的 tempo_lane 传入）。
                // 主音轨缺失/不可见/是 conductor 时返回空 Vec。
                let automation_lanes: Vec<yinhe_types::AutomationLane> = {
                    let edit_trk = doc
                        .edit
                        .main_track()
                        // 用 pr_visible 而非原始 track_visible：主音轨强制可见，
                        // 取消勾选显示不应使 automation 编辑静默失效。
                        .filter(|&t| pr_visible.get(t as usize).copied().unwrap_or(false))
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
                // 和弦指示器文本：实时 MIDI 按键优先；无按键且播放中 → 播放头和弦。
                // 逐轨识别：主轨优先、PR 可见次之，跳过静音/不可见/力度≤1，择最完善的多音和弦。
                let chord_text = chord_indicator_text(
                    &self.midi_thru_keys,
                    is_playing,
                    doc.edit.cursor_tick,
                    Some(&*doc.data.model),
                    main,
                    &pr_visible,
                    &doc.edit.track_overrides,
                    doc.edit.conductor_track_idx,
                );
                let bar_data = piano_view::control_bar::PrBarData {
                    ppq: tpb,
                    quantize: doc.edit.quantize_pianoroll,
                    track_infos: &doc.edit.track_info_cache,
                    pr_track_visible: &doc.edit.track_pianoroll_visible,
                    // 主音轨纯派生自选中集合：无选中时为空，不显示回退轨。
                    main_track: main,
                    allow_overlap: doc.edit.allow_overlapping_notes,
                    chord: chord_text.as_deref(),
                };
                // 新建音符默认长度：该轨 gate 记忆，无记忆回退量化间隔
                let default_gate = write_track.map(|t| {
                    let fallback = doc.edit.quantize_pianoroll.tick_interval(tpb);
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
                    velocity_edits: &mut velocity_edits,
                    preview_reqs: &mut preview_reqs,
                    status_hint: &mut self.status_hint,
                    bar_events: &mut bar_events,
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
                    &mut self.last_hidden_keys,
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
                    self.audio_settings.content_opacity,
                    self.audio_settings.note_outline,
                    self.audio_settings.use_gpu_cull,
                    &doc.data.model.conductor.tempo,
                    &mut doc.edit.sel_rect,
                    &doc.edit.track_selected,
                    doc.edit.conductor_track_idx,
                    write_track,
                    bar_data,
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
                preview_reqs,
                bar_events,
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
                PianoViewEvent::QuickDelete {
                    track,
                    start_tick,
                    key,
                } => {
                    let Some(idx) = self.active_doc else { return };
                    // 快速删除：按 (track, start_tick, key) 定位单个音符
                    // 仅在音符可见且在选定轨道范围内才删除（与 hit 的 track_selected 过滤一致）
                    let edit = &self.documents[idx].edit;
                    if !edit
                        .track_pianoroll_visible
                        .get(track as usize)
                        .copied()
                        .unwrap_or(true)
                    {
                        return;
                    }
                    if !edit.track_selected.is_empty() && !edit.track_selected.contains(&track) {
                        return;
                    }
                    self.with_undo(t!("undo.delete_notes").as_ref(), |doc| {
                        doc.delete_single_note(track, start_tick, key)
                    });
                }
            }
        }

        // PR 控制栏事件：量化 / 切换主音轨（track_selected 替换）/ 显示音轨勾选。
        // `overlap_change` 记录开关变动，循环后统一写全局设置（避免 edit 借用期间碰 self 其他字段）。
        let mut overlap_change: Option<bool> = None;
        for ev in bar_events {
            use crate::piano_view::control_bar::PrBarEvent;
            let Some(idx) = self.active_doc else { break };
            let edit = &mut self.documents[idx].edit;
            match ev {
                PrBarEvent::Quantize(preset) => edit.quantize_pianoroll = preset,
                PrBarEvent::SwitchMainTrack(t) => {
                    edit.track_selected.clear();
                    edit.track_selected.insert(t);
                }
                // PR 显示开关只写 track_pianoroll_visible，不影响 AR（track_visible）。
                PrBarEvent::SetTrackVisible(t, v) => {
                    if let Some(slot) = edit.track_pianoroll_visible.get_mut(t as usize) {
                        *slot = v;
                    }
                }
                PrBarEvent::SetAllVisible(v) => edit.track_pianoroll_visible.fill(v),
                PrBarEvent::SetAllowOverlap(v) => edit.allow_overlapping_notes = v,
            }
            // 仅当有开关事件时才记录（bar_events 可能为空，避免无谓写入）。
            if let PrBarEvent::SetAllowOverlap(v) = ev {
                overlap_change = Some(v);
            }
        }
        // 全局持久化，跨工程生效；并同步到所有已打开文档保持全局语义一致。
        if let Some(v) = overlap_change {
            self.audio_settings.allow_overlapping_notes = v;
            self.audio_settings.save();
            for doc in &mut self.documents {
                doc.edit.allow_overlapping_notes = v;
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
    /// 若存在 PR 选框，仅允许修改选框内且在选定轨道内的音符力度。
    fn handle_velocity_edits(&mut self, edits: &[yinhe_types::VelocityEdit]) {
        let Some(idx) = self.active_doc else { return };
        // 选框过滤：存在选框时仅保留落在选框且轨道选中的音符
        let filtered: Vec<yinhe_types::VelocityEdit> = {
            let doc = &self.documents[idx];
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
        let doc = &mut self.documents[idx];
        let before = doc.capture_snapshot();
        if let Some(action) = doc.set_notes_velocity(&filtered) {
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
    /// 选中目标音轨（音符/automation 事件）、滚动到中心。
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

        // 2. 选中目标音轨（音符/CC/PB/PC 事件需要）。
        // Conductor 跳过（不作为写入目标）；Tempo 编辑不依赖选中。
        if let Some((track, _key)) = req.note
            && let Some(idx) = self.active_doc
            && self.documents[idx].edit.conductor_track_idx != Some(track)
        {
            let sel = &mut self.documents[idx].edit.track_selected;
            sel.clear();
            sel.insert(track);
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

    /// Handle AR drag: move (or Alt-copy) selected notes + automation events
    /// by `(delta_ticks, delta_tracks)`. Single atomic operation = single undo step.
    fn handle_arr_drag(&mut self, delta_ticks: i64, delta_tracks: i32, alt: bool) {
        if delta_ticks == 0 && delta_tracks == 0 {
            return;
        }
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];

        let before = doc.capture_snapshot();
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
            self.arrange_view.base.dirty = true;
            doc.push_undo(action, &label, before);
            self.notify_audio_model_changed();
        }
    }

    /// Show right panel, and request repaint if playing.
    pub(in crate::app) fn show_panels_and_overlays(
        &mut self,
        ui: &mut egui::Ui,
        layout: &LayoutInfo,
    ) {
        // 互斥切换：用户主动打开右侧栏 Info tab 时收回浮动面板（弹窗内容回到侧栏态）。
        if self.float_panel.is_some() && self.right_tab == Some(crate::right_panel::RightTab::Info)
        {
            self.float_panel = None;
        }
        // Right panel
        if self.right_tab.is_some() {
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(layout.remaining.max.x, layout.remaining.min.y),
                egui::vec2(layout.right_panel_total_w, layout.remaining.height()),
            );
            let doc = self.active_doc.and_then(|idx| self.documents.get_mut(idx));
            let (changed, jump_request, width_drag_ended) = crate::right_panel::show(
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
                &mut self.float_panel,
            );
            if changed {
                self.teardown_audio();
            }
            if width_drag_ended {
                self.layout_needs_save = true;
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

    /// 帧末：把当前布局值同步进设置（内存），拖拽结束/切换帧写盘一次。
    /// 拖拽过程中值每帧变化但不置标志，避免拖拽时每帧刷盘。
    pub(in crate::app) fn sync_layout_settings(&mut self) {
        let l = &mut self.audio_settings.layout;
        l.right_panel_width = self.right_panel_width;
        l.arr_split = self.arr_split;
        l.transport_panel_width = self.transport_panel_width;
        l.show_pianoroll_in_arrange = self.show_pianoroll_in_arrange;
        if self.layout_needs_save {
            self.audio_settings.save();
            self.layout_needs_save = false;
        }
    }
}

/// 和弦指示器文本：实时 MIDI 直通按键优先；无按键且播放中 → 播放头处发声音符的和弦。
///
/// 播放路径逐轨识别：主轨优先、PR 可见次之，跳过静音/不可见/力度≤1 的轨道，
/// 在所有候选轨道中择最完善的多音和弦（音级数最多），无多音和弦时回退单音。
/// 每轨内部复用 `chord::recognize` 的防乱按规则（音级 > 7 或跨度 > 2 八度不识别）。
#[allow(clippy::too_many_arguments)]
fn chord_indicator_text(
    thru_keys: &std::collections::HashMap<u8, u8>,
    is_playing: bool,
    cursor_tick: Option<f64>,
    model: Option<&yinhe_core::YinModel>,
    main_track: Option<u16>,
    pr_visible: &[bool],
    track_overrides: &[yinhe_editor_core::document::TrackOverride],
    conductor_track_idx: Option<u16>,
) -> Option<String> {
    if !thru_keys.is_empty() {
        let mut keys: Vec<u8> = thru_keys.keys().copied().collect();
        keys.sort_unstable();
        return yinhe_editor_core::chord::recognize(&keys);
    }
    if !is_playing {
        return None;
    }
    let tick = cursor_tick?.max(0.0) as u32;
    let model = model?;
    let n = model.tracks.len();
    if n == 0 {
        return None;
    }
    // 静音/独奏遮罩：有独奏时仅独奏轨可发声，其余视为静音。
    let has_solo = track_overrides.iter().any(|ov| ov.soloed);
    let is_muted = |idx: usize| -> bool {
        if let Some(ov) = track_overrides.get(idx) {
            if has_solo {
                return !ov.soloed;
            }
            return ov.muted;
        }
        // 回退到 model 标记（旧工程或长度不一致时）
        model.tracks.get(idx).is_some_and(|t| t.muted)
    };

    // 每首歌自适应的密度阈值：密集视觉轨（黑墙）per_bar 远高于音乐轨，用中位数自适应过滤。
    // tick 处的拍号决定 bar_ticks，避免 3/4、6/8 等误算。
    let (num, den) = model.tempo_map.time_sig_at_tick(tick);
    let bar_ticks = measure_ticks(model.meta.ppq, num, den).max(1);
    let tick_len = model.tick_length.max(1) as f64;
    let mut per_bar_vec = Vec::with_capacity(n);
    for idx in 0..n {
        let cnt = model.track_note_count.get(idx).copied().unwrap_or(0) as f64;
        per_bar_vec.push(cnt * bar_ticks as f64 / tick_len);
    }
    // 中位数 *2.5 自适应，至少 60，避免稀疏歌误杀。
    let mut sorted_pb = per_bar_vec.clone();
    sorted_pb.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_pb = if sorted_pb.is_empty() {
        0.0
    } else if sorted_pb.len() % 2 == 1 {
        sorted_pb[sorted_pb.len() / 2]
    } else {
        (sorted_pb[sorted_pb.len() / 2 - 1] + sorted_pb[sorted_pb.len() / 2]) * 0.5
    };
    let density_thresh = (median_pb * 2.5).max(60.0);
    let is_dense = |idx: usize| -> bool { per_bar_vec[idx] > density_thresh };

    // 每轨发声 key 集合（已过滤静音/不可见/力度≤1），按 key 升序自然有序。
    let mut keys_by_track_sim: Vec<Vec<u8>> = vec![Vec::new(); n];
    // 单次 128 key 扫描，按 tick 覆盖把 key 分发到各轨的 present 标记。
    for key in 0u8..128 {
        // 固定 256 容纳极端多轨（桌面默认 17 轨），避免每 key 分配 HashSet。
        let mut present = [false; 256];
        let mut any = false;
        for note in model.key_notes_in_range(key, tick, tick.saturating_add(1)) {
            if note.velocity <= 1 {
                continue;
            }
            if note.start_tick > tick || tick >= note.end_tick {
                continue;
            }
            let idx = note.track as usize;
            if idx >= n {
                continue;
            }
            if Some(idx as u16) == conductor_track_idx {
                continue;
            }
            if idx >= pr_visible.len() || !pr_visible[idx] {
                continue;
            }
            if is_muted(idx) {
                continue;
            }
            if is_dense(idx) {
                continue;
            }
            // 全局无可听音符的轨道可跳过（由 rebuild_dirty 维护，过滤纯力度1轨道）
            if model.track_audible_count.get(idx).copied().unwrap_or(0) == 0 {
                continue;
            }
            if !present[idx] {
                present[idx] = true;
                any = true;
            }
        }
        if !any {
            continue;
        }
        for idx in 0..n {
            if present[idx] {
                keys_by_track_sim[idx].push(key);
            }
        }
    }

    // 同时发声已能覆盖块状和弦，为降低延迟与开销：若 sim 已有 pcs>=3 的多音和弦，直接返回，不再跑窗口。
    // 仅当 sim 无多音时才做分解和弦回退，回退窗口仅 1 拍（ppq），lookback 避免前瞻预测。
    // 先评估 sim 的 best_multi，若已足够好则提前返回。
    let best_sim: Option<(f64, u8, u16, String)> = {
        let mut tmp_multi: Option<(f64, u8, u16, String)> = None;
        for (idx, keys) in keys_by_track_sim.iter().enumerate() {
            if keys.is_empty() {
                continue;
            }
            if keys.len() == 1 {
                continue;
            }
            if let Some(name) = yinhe_editor_core::chord::recognize(keys) {
                let mut mask = 0u16;
                for &k in keys {
                    mask |= 1 << (k % 12);
                }
                let pcs = mask.count_ones() as f64;
                let per_bar = per_bar_vec[idx];
                let score = pcs * 1000.0 / (per_bar + 10.0);
                let prio = if Some(idx as u16) == main_track { 0 } else { 1 };
                match &tmp_multi {
                    None => tmp_multi = Some((score, prio, idx as u16, name.clone())),
                    Some((best_score, best_prio, _, _)) => {
                        if score > *best_score + f64::EPSILON
                            || (score - *best_score).abs() < f64::EPSILON && prio < *best_prio
                        {
                            tmp_multi = Some((score, prio, idx as u16, name.clone()));
                        }
                    }
                }
            }
        }
        tmp_multi
    };
    if let Some((score, _, _, ref name)) = best_sim {
        // 块状和弦已足够完整则立即返回，避免窗口回退带来的延迟与额外扫描
        if score > 30.0 {
            return Some(name.clone());
        }
    }

    // 分解和弦回退：窗口内起音聚合（lookback 1拍），即使不同时也能还原和弦。
    let mut keys_by_track_win: Vec<Vec<u8>> = vec![Vec::new(); n];
    {
        let window = model.meta.ppq; // 1拍，延迟约 1拍/2，随 ppq 自适应
        let lo = tick.saturating_sub(window);
        let hi = tick.saturating_add(1);
        for key in 0u8..128 {
            let mut present = [false; 256];
            let mut any = false;
            for note in model.key_notes(key).range(lo, hi) {
                if note.velocity <= 1 {
                    continue;
                }
                if note.start_tick < lo || note.start_tick >= hi {
                    continue;
                }
                if note.end_tick.saturating_sub(note.start_tick) <= 60 {
                    continue;
                }
                let idx = note.track as usize;
                if idx >= n {
                    continue;
                }
                if Some(idx as u16) == conductor_track_idx {
                    continue;
                }
                if idx >= pr_visible.len() || !pr_visible[idx] {
                    continue;
                }
                if is_muted(idx) {
                    continue;
                }
                if is_dense(idx) {
                    continue;
                }
                if model.track_audible_count.get(idx).copied().unwrap_or(0) == 0 {
                    continue;
                }
                if !present[idx] {
                    present[idx] = true;
                    any = true;
                }
            }
            if !any {
                continue;
            }
            for idx in 0..n {
                if present[idx] {
                    keys_by_track_win[idx].push(key);
                }
            }
        }
    }

    // 在候选轨道中择优：多音和弦按密度惩罚后的 pcs 择最完善，单音仅作回退。
    // 密集轨 per_bar 高，pcs 相同则稀疏音乐轨胜出，通用任何歌曲。
    let mut best_multi: Option<(f64, u8, u16, String)> = None; // (score, priority, track, name)
    let mut best_single: Option<(u8, u16, String)> = None; // (priority, track, name)
    for idx in 0..n {
        let idx_u16 = idx as u16;
        let priority: u8 = if Some(idx_u16) == main_track { 0 } else { 1 };
        let per_bar = per_bar_vec[idx];
        // 每轨取 sim 与 win 中更好的候选（pcs 更高者）
        let mut candidates: Vec<&Vec<u8>> = Vec::new();
        if !keys_by_track_sim[idx].is_empty() {
            candidates.push(&keys_by_track_sim[idx]);
        }
        if !keys_by_track_win[idx].is_empty() {
            candidates.push(&keys_by_track_win[idx]);
        }
        for keys in candidates {
            if keys.is_empty() {
                continue;
            }
            if let Some(name) = yinhe_editor_core::chord::recognize(keys) {
                if keys.len() == 1 {
                    match &best_single {
                        None => best_single = Some((priority, idx_u16, name.clone())),
                        Some((best_prio, _, _)) if priority < *best_prio => {
                            best_single = Some((priority, idx_u16, name.clone()))
                        }
                        _ => {}
                    }
                } else {
                    let mut mask = 0u16;
                    for &k in keys {
                        mask |= 1 << (k % 12);
                    }
                    let pcs = mask.count_ones() as f64;
                    // 密度惩罚：per_bar 越高分越低，通用任何歌曲
                    let score = pcs * 1000.0 / (per_bar + 10.0);
                    match &best_multi {
                        None => best_multi = Some((score, priority, idx_u16, name.clone())),
                        Some((best_score, best_prio, _, _)) => {
                            if score > *best_score + f64::EPSILON
                                || (score - *best_score).abs() < f64::EPSILON
                                    && priority < *best_prio
                            {
                                best_multi = Some((score, priority, idx_u16, name.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some((_, _, _, name)) = best_multi {
        return Some(name);
    }
    if let Some((_, _, name)) = best_single {
        return Some(name);
    }
    None
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
        doc.edit.track_selected = [1u16].into_iter().collect();
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

    fn chord_tick(doc: &yinhe_editor_core::document::Document, tick: f64) -> Option<String> {
        let main = doc.edit.main_track();
        chord_indicator_text(
            &std::collections::HashMap::new(),
            true,
            Some(tick),
            Some(doc.model()),
            main,
            &doc.edit.track_pianoroll_visible,
            &doc.edit.track_overrides,
            doc.edit.conductor_track_idx,
        )
    }

    fn add_chord(
        doc: &mut yinhe_editor_core::document::Document,
        track: u16,
        keys: &[u8],
        tick: u32,
        vel: u8,
    ) {
        for &key in keys {
            let ev = yinhe_core::NoteEvent {
                id: 0,
                start_tick: tick,
                end_tick: tick + 480,
                key,
                velocity: vel,
            };
            doc.add_note(track, ev).expect("add_note");
        }
    }

    #[test]
    fn chord_per_track_prefers_main_over_visible_when_same_pcs() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        // Track 1 (main) = C (60,64,67), Track 2 = G (67,71,74) 同为 3 pcs，优先主轨
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[67, 71, 74], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should recognize");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_picks_most_complete_across_tracks() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        // 主轨 1 = C 三和弦 (3 pcs), 轨 2 = Cmaj7 (4 pcs) 更完善 → 选轨 2
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should recognize");
        assert_eq!(chord, "Cmaj7");
    }

    #[test]
    fn chord_skips_muted_track() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        // 静音轨 2 → 回退主轨
        doc.edit.track_overrides[2].muted = true;
        let chord = chord_tick(&doc, 0.0).expect("should fallback");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_skips_invisible_track() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        doc.edit.track_pianoroll_visible[2] = false;
        let chord = chord_tick(&doc, 0.0).expect("should fallback");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_skips_velocity_one_notes() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        // 轨 2 全为力度 1 → 应被过滤
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 1);
        let chord = chord_tick(&doc, 0.0).expect("should pick audible");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_per_track_anti_garbage_global_span_would_fail() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        // 轨 1 低八度 C (36,40,43), 轨 2 高八度 C (84,88,91) 合计跨度 55>24，旧全局会 None
        add_chord(&mut doc, 1, &[36, 40, 43], 0, 100);
        add_chord(&mut doc, 2, &[84, 88, 91], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("per-track should still recognize");
        // 主轨优先，故为低八度轨的 C（音名带 slash？识别为 C，转位低音 36 = C2? 实际 low =36 pc 0 root 0 -> C）
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_prefers_multi_over_single() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        // 轨 1 单音 C5 (60), 轨 2 三和弦 C (60,64,67) → 应选多音
        add_chord(&mut doc, 1, &[60], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should prefer multi");
        assert_eq!(chord, "C");
    }
}
