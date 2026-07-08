use eframe::egui;

use crate::app::App;
use crate::arrange;
use crate::file_loader::LoadResult;
use crate::piano_view;
use yinhe_editor_core::document::Document;

/// Layout geometry computed once per frame, shared by arrangement and pianoroll.
pub(super) struct LayoutInfo {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub bottom_y: f32,
    pub tools_panel_w: f32,
    pub right_panel_total_w: f32,
    pub has_arr: bool,
    pub has_piano: bool,
}

impl App {
    /// Poll all async operations: file loading, save completion, export completion.
    pub(super) fn poll_async_operations(&mut self) {
        // Poll async file loading
        match self.file_loader.poll_loading() {
            LoadResult::ModelLoaded { path, model } => {
                let quantize = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| doc.edit.quantize)
                    .unwrap_or_default();
                match Document::from_model(&path, model, quantize, yinhe_yin::ProjectFile::default(), yinhe_yin::MappingFile::default()) {
                    Ok(doc) => {
                        let insert_idx = self.documents.len();
                        self.documents.push(doc);
                        self.active_doc = Some(insert_idx);
                        self.teardown_audio();
                    }
                    Err(msg) => {
                        self.load_error = Some(msg);
                    }
                }
            }
            LoadResult::ModelFromYin {
                path,
                model,
                file_name,
                sf,
                mapping,
            } => {
                let quantize = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| doc.edit.quantize)
                    .unwrap_or_default();
                let project_file = yinhe_yin::ProjectFile::from_meta_with_sf(
                    &model.meta,
                    sf.mode,
                    sf.overrides.clone(),
                );
                let result = Document::from_model(&path, model, quantize, project_file, mapping)
                    .ok()
                    .map(|mut d| {
                        d.file_path = Some(path.clone());

                        d.edit.project_sf.overrides = sf
                            .overrides
                            .iter()
                            .map(|po| {
                                let entries = po
                                    .entries
                                    .iter()
                                    .map(|e| yinhe_editor_core::SfEntry {
                                        path: e.path.clone(),
                                        name: e.name.clone(),
                                        enabled: e.enabled,
                                    })
                                    .collect();
                                (po.port, entries)
                            })
                            .collect();

                        (d, sf.mode)
                    });
                if let Some((doc, sf_project_mode)) = result {
                    self.audio_settings.global_sf_config.global_enabled = !sf_project_mode;
                    let insert_idx = self.documents.len();
                    self.documents.push(doc);
                    self.active_doc = Some(insert_idx);
                    self.teardown_audio();
                } else {
                    self.load_error = Some(format!(
                        "无法打开「{}」：可能不是有效的 .yin 文件，或其内嵌 MIDI 缺少 Conductor 轨道。",
                        file_name
                    ));
                }
            }
            LoadResult::ArchiveError(msg) => {
                self.load_error = Some(msg);
            }
            LoadResult::NotReady => {}
        }

        // Poll async save completion
        if let Some(rx) = &self.save_rx {
            if rx.try_recv().is_ok() {
                self.save_rx = None;
            }
        }

        // Poll async export completion
        if let Some(rx) = &self.export_rx {
            if let Ok(result) = rx.try_recv() {
                self.export_rx = None;
                if let Err(e) = result {
                    self.load_error = Some(e);
                }
            }
        }
    }

    /// Compute layout geometry for the current frame.
    pub(super) fn compute_layout(&mut self, ui: &mut egui::Ui) -> LayoutInfo {
        let mut remaining = ui.available_rect_before_wrap();

        let has_arr = self.show_transport && self.active_doc.is_some();
        let has_piano = self.show_pianoroll && self.active_doc.is_some();
        let tools_panel_w = if has_arr || has_piano {
            crate::widgets::tools_panel::TOOLS_PANEL_W
        } else {
            0.0
        };
        remaining.max.x -= tools_panel_w;

        let right_panel_total_w = if self.right_tab.is_some() {
            let max_w = (remaining.width() - 60.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0);
            let pw =
                (self.right_panel_width + 4.0).clamp(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0, max_w);
            self.right_panel_width = (pw - 4.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH);
            pw
        } else {
            0.0
        };
        remaining.max.x -= right_panel_total_w;

        let total = remaining.size();
        let arr_h = if self.show_transport && self.active_doc.is_some() {
            if self.show_pianoroll {
                (total.y * self.arr_split).max(crate::theme::MIN_ARR_HEIGHT)
            } else {
                total.y
            }
        } else {
            0.0
        };
        let bottom_y = remaining.min.y
            + arr_h
            + if self.show_transport && self.show_pianoroll && self.active_doc.is_some() {
                crate::theme::SPLIT_GAP
            } else {
                0.0
            };

        LayoutInfo {
            remaining,
            arr_h,
            bottom_y,
            tools_panel_w,
            right_panel_total_w,
            has_arr,
            has_piano,
        }
    }

    /// Show the main content area: arrangement view, pianoroll, and note drag handling.
    pub(super) fn show_main_content(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        let Some(idx) = self.active_doc else {
            return;
        };

        let is_playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        let mut follow_mode = self.follow_mode;

        // Arrangement view
        if self.show_transport {
            let mut request_pianoroll = false;
            let mut guard = super::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
            arrange::show(
                ui,
                guard.as_mut(),
                &mut self.arrange_view,
                layout.remaining,
                layout.arr_h,
                &mut self.transport_panel_width,
                &mut self.arr_renderer,
                &mut self.arr_render_ctx,
                &mut self.last_cursor_tick,
                is_playing,
                &mut follow_mode,
                &self.active_tool,
                self.audio.as_ref(),
                &mut request_pianoroll,
                &mut self.track_selection_anchor,
                self.audio_settings.scroll_mode,
                self.audio_settings.min_border_width,
                Some(&self.haptic_engine),
                &mut self.arr_sel_rect,
            );
            if request_pianoroll {
                self.show_pianoroll = true;
                self.show_pianoroll_in_arrange = true;
            }
        }

        // Pianoroll area
        if self.show_pianoroll {
            self.show_pianoroll_split(ui, layout, idx, is_playing, &mut follow_mode);
        }

        self.follow_mode = follow_mode;
    }

    /// Show the pianoroll split area, including the split handle and pianoroll view.
    fn show_pianoroll_split(
        &mut self,
        ui: &mut egui::Ui,
        layout: &LayoutInfo,
        idx: usize,
        is_playing: bool,
        follow_mode: &mut crate::view_interaction::FollowMode,
    ) {
        // Horizontal splitter
        if self.show_transport {
            let split_right = layout.remaining.max.x + layout.tools_panel_w;
            let h_split_rect = egui::Rect::from_min_max(
                egui::pos2(layout.remaining.min.x, layout.remaining.min.y + layout.arr_h),
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

        let mut auto_edit_events: Vec<crate::piano_view::automation_panel::AutomationEdit> = Vec::new();

        let (piano_event, note_drag_delta, pencil_note_drag) = {
            let mut guard = super::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
            let doc = guard.as_mut();
            let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                Some(doc.data.model.as_ref());
            let piano_rect = egui::Rect::from_min_max(
                egui::pos2(layout.remaining.min.x, layout.bottom_y),
                layout.remaining.max,
            );

            let mut event = None;
            let mut note_drag_delta: Option<(i64, i32)> = None;
            let mut pencil_note_drag: Option<crate::piano_view::PencilNoteDrag> = None;
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(piano_rect),
                |ui| {
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
                    let pr_visible: Vec<bool> = (0..doc.edit.track_visible.len())
                        .map(|i| {
                            if show_all {
                                doc.edit.track_visible[i]
                            } else {
                                doc.edit.track_visible[i]
                                    && doc.edit.track_selected.contains(&(i as u16))
                            }
                        })
                        .collect();
                    let tpb = doc.data.model.meta.ppq;
                    let ts_num = doc.data.model.conductor.time_sig.first().map(|t| t.numerator).unwrap_or(4);
                    let ts_den = doc.data.model.conductor.time_sig.first().map(|t| t.denominator).unwrap_or(2);
                    let ts_events: Vec<yinhe_types::TimeSigEvent> = doc.data.model.conductor.time_sig.iter().map(|t| yinhe_types::TimeSigEvent {
                        tick: t.tick,
                        numerator: t.numerator,
                        denominator: t.denominator,
                    }).collect();
                    // Get automation lanes: conductor → all tracks; otherwise → first selected
                    let automation_lanes: Vec<yinhe_types::AutomationLane> = {
                        if show_all {
                            doc.data.model.tracks.iter()
                                .flat_map(|t| t.automation_lanes.iter().cloned())
                                .collect()
                        } else {
                            let first_track = doc.edit.track_selected.iter().next().copied().unwrap_or(0) as usize;
                            doc.data.model.tracks.get(first_track)
                                .map(|t| t.automation_lanes.clone())
                                .unwrap_or_default()
                        }
                    };
                    // Extract tempo events for automation panel
                    let tempo_events: Vec<(u32, f64)> = doc.data.model.conductor.tempo.iter()
                        .map(|t| (t.tick, t.bpm))
                        .collect();
                    event = piano_view::show(
                        ui,
                        ui.available_size(),
                        &mut self.pianoroll,
                        &mut self.render_ctx,
                        &mut self.pianoroll_view,
                        midi_source,
                        &mut doc.edit.selected,
                        &pr_visible,
                        &doc.edit.track_colors_cache,
                        &mut doc.edit.cursor_tick,
                        is_playing,
                        doc.edit.quantize,
                        tpb,
                        Some((tpb, ts_num, ts_den, &ts_events)),
                        &mut self.piano_last_cursor_tick,
                        follow_mode,
                        &self.active_tool,
                        Some(&mut doc.edit.controller_panels),
                        Some(&mut self.controller_renderers[idx]),
                        Some(&automation_lanes),
                        Some(&mut doc.edit.show_controller_panels),
                        Some(&auto_wgpu_state),
                        self.audio_settings.scroll_mode,
                        self.audio_settings.min_border_width,
                        &mut self.audio_settings.velocity_display_mode,
                        self.audio_settings.note_selection_highlight,
                        &tempo_events,
                        &mut note_drag_delta,
                        &mut doc.edit.sel_rect,
                        &doc.edit.track_selected,
                        doc.edit.conductor_track_idx,
                        doc.data.midi_version,
                        Some(&self.haptic_engine),
                        &mut pencil_note_drag,
                        &mut auto_edit_events,
                    );
                    if let Some(t0) = _piano_total_start {
                        yinhe_memtrace::perf_probe::record_piano_total(t0.elapsed());
                    }
                },
            );
            (event, note_drag_delta, pencil_note_drag)
        };

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
                    }
                }
                PianoViewEvent::AddNote { track, note } => {
                    self.add_note_with_undo(track, note);
                }
            }
        }

        // Handle note drag
        self.handle_note_drag(note_drag_delta);

        // Handle pencil note drag
        self.handle_pencil_note_drag(pencil_note_drag);

        // Handle automation edits
        if !auto_edit_events.is_empty() {
            self.handle_automation_edits(auto_edit_events);
        }
    }

    /// 把 automation 面板产生的编辑事件应用到 Document，push undo，并通知音频线程。
    fn handle_automation_edits(
        &mut self,
        edits: Vec<crate::piano_view::automation_panel::AutomationEdit>,
    ) {
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];

        let actions = doc.apply_automation_edits(edits);
        if !actions.is_empty() {
            self.pianoroll_view.base.dirty = true;
            for action in actions {
                doc.history.push(yinhe_editor_core::history::UndoEntry {
                    action,
                    label: "Edit automation",
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
            }
            if let Some(ref audio) = self.audio {
                let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes { model: doc.data.model.clone() });
            }
        }
    }

    /// Handle note drag — called once on release.
    fn handle_note_drag(&mut self, note_drag_delta: Option<(i64, i32)>) {
        if let Some((delta_ticks, delta_keys)) = note_drag_delta {
            let Some(idx) = self.active_doc else { return };
            let doc = &mut self.documents[idx];
            if let Some(action) = doc.move_selected_notes(delta_ticks, delta_keys) {
                self.pianoroll_view.base.dirty = true;
                doc.history.push(yinhe_editor_core::history::UndoEntry {
                    action,
                    label: "Move notes",
                    selected: doc.edit.selected.clone(),
                    track_selected: doc.edit.track_selected.clone(),
                    sel_rect: doc.edit.sel_rect.clone(),
                });
                if let Some(ref audio) = self.audio {
                    let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes { model: doc.data.model.clone() });
                }
            }
        }
    }

    /// Handle pencil note drag updates (move or resize a single note).
    fn handle_pencil_note_drag(&mut self, drag: Option<crate::piano_view::PencilNoteDrag>) {
        let Some(drag) = drag else { return };
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];
        if let Some(action) = doc.pencil_drag_note(&drag) {
            self.pianoroll_view.base.dirty = true;
            doc.history.push(yinhe_editor_core::history::UndoEntry {
                action,
                label: match &drag {
                    crate::piano_view::PencilNoteDrag::Move { .. } => "Move note",
                    _ => "Resize note",
                },
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
            if let Some(ref audio) = self.audio {
                let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes { model: doc.data.model.clone() });
            }
        }
    }

    /// Show tool panels, right panel, and request repaint if playing.
    pub(super) fn show_panels_and_overlays(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        let tools_x = layout.remaining.max.x;

        // Tool panels
        if layout.has_arr {
            let rect = egui::Rect::from_min_size(
                egui::pos2(tools_x, layout.remaining.min.y),
                egui::vec2(layout.tools_panel_w, layout.arr_h),
            );
            crate::widgets::tools_panel::show(
                ui,
                rect,
                &mut self.active_tool,
                &crate::widgets::tools_panel::ALL_TOOLS,
            );
        }
        if layout.has_piano {
            let rect = egui::Rect::from_min_size(
                egui::pos2(tools_x, layout.bottom_y),
                egui::vec2(
                    layout.tools_panel_w,
                    layout.remaining.max.y - layout.bottom_y,
                ),
            );
            crate::widgets::tools_panel::show(
                ui,
                rect,
                &mut self.active_tool,
                &crate::widgets::tools_panel::ALL_TOOLS,
            );
        }

        // Right panel
        if self.right_tab.is_some() {
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(tools_x + layout.tools_panel_w, layout.remaining.min.y),
                egui::vec2(layout.right_panel_total_w, layout.remaining.height()),
            );
            let doc = self
                .active_doc
                .and_then(|idx| self.documents.get_mut(idx));
            let changed = crate::right_panel::show(
                ui,
                right_rect,
                &mut self.right_panel_width,
                &mut self.right_tab,
                &mut self.audio_settings,
                doc,
                self.audio.as_ref(),
                &mut self.event_browser_state,
            );
            if changed {
                self.teardown_audio();
            }
        }

        // Request repaint during playback (or while waiting for audio thread to start)
        let is_audio_playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        if is_audio_playing || self.pending_playback {
            ui.ctx().request_repaint();
        }
    }

    /// Sync `automation_event_density` to the audio engine when it changes.
    pub(super) fn sync_automation_density(&mut self) {
        let density = self.audio_settings.automation_event_density;
        if density != self.last_automation_density {
            self.last_automation_density = density;
            if let Some(audio) = &self.audio {
                audio.handle.send(yinhe_audio::AudioCommand::SetAutomationDensity {
                    density,
                });
            }
        }
    }

    /// Show all overlay dialogs as independent OS windows.
    pub(super) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── Settings dialog (independent window) ──
        if self.audio_settings.show_settings {
            let settings = std::rc::Rc::new(std::cell::RefCell::new(Some(std::mem::take(&mut self.audio_settings))));
            let ctx_clone = ctx.clone();
            let settings_cb = settings.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("settings_dialog"),
                crate::chrome::dialog::viewport_builder("设置", [480.0, 520.0], true),
                move |vctx, _class| {
                    let mut slot = settings_cb.borrow_mut().take();
                    if let Some(ref mut s) = slot {
                        let mut close = false;
                        if vctx.input(|i| i.viewport().close_requested()) {
                            close = true;
                        }
                        egui::CentralPanel::default()
                            .frame(egui::Frame {
                                fill: crate::theme::APP_BG,
                                ..Default::default()
                            })
                            .show(vctx, |ui| {
                            crate::chrome::dialog::title_bar(ui, "设置", &mut close);
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: 12,
                                    right: 12,
                                    top: 0,
                                    bottom: 12,
                                })
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false; 2])
                                        .show(ui, |ui| {
                                            let changed = crate::dialogs::settings::show_content(ui, s);
                                            if changed {
                                                s.save();
                                            }
                                        });
                                });
                        });
                        if close {
                            // Hide window before closing to prevent white flash
                            vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            s.show_settings = false;
                            // Keep slot = Some(s) — don't drop the settings
                        }
                    }
                    *settings_cb.borrow_mut() = slot;
                },
            );

            if let Some(s) = std::rc::Rc::into_inner(settings)
                .and_then(|rc| rc.into_inner())
            {
                self.audio_settings = s;
                // Sync haptic settings to the engine
                self.haptic_engine.apply_settings(
                    self.audio_settings.haptic_enabled,
                    self.audio_settings.haptic_intensity,
                );
                if !self.audio_settings.show_settings {
                    self.teardown_audio();
                }
            }
        }

        // ── Memory breakdown (independent window) ──
        if self.show_mem_breakdown {
            let snapshot = yinhe_memtrace::Snapshot::capture();
            let mem_mb = self.sys_monitor.mem_mb;

            #[cfg(target_os = "macos")]
            let metal_size = self
                .render_ctx
                .metal_allocated_size()
                .unwrap_or(0)
                .saturating_add(self.arr_render_ctx.metal_allocated_size().unwrap_or(0));

            let open = std::rc::Rc::new(std::cell::RefCell::new(true));
            let ctx_clone = ctx.clone();
            let open_cb = open.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("memory_breakdown_dialog"),
                crate::chrome::dialog::viewport_builder(
                    "内存占用详情",
                    crate::theme::MEM_POPUP_SIZE,
                    false,
                ),
                move |vctx, _class| {
                    let mut close = false;
                    if vctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(
                            ui,
                            "内存占用详情",
                            &mut close,
                        );
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        ui.label(format!("系统统计总内存: {:.1} MB", mem_mb));
                                        ui.label(format!("分配器追踪内存: {:.1} MB", snapshot.total_mb()));
                                        ui.label(format!("wgpu 显式 GPU 资源: {:.1} MB", snapshot.gpu_mb()));

                                        #[cfg(target_os = "macos")]
                                        ui.label(format!(
                                            "Metal 驱动真实显存: {:.1} MB",
                                            metal_size as f64 / 1_048_576.0
                                        ));

                                        ui.separator();

                                        ui.heading("按子系统分类");
                                        egui::Grid::new("mem_breakdown_grid")
                                            .num_columns(2)
                                            .spacing([12.0, 8.0])
                                            .show(ui, |ui| {
                                                for tag in yinhe_memtrace::AllocTag::ALL {
                                                    if tag == yinhe_memtrace::AllocTag::Unknown
                                                        && snapshot.get(tag) <= 0
                                                    {
                                                        continue;
                                                    }
                                                    ui.label(tag.name());
                                                    ui.label(format!("{:.1} MB", snapshot.mb(tag)));
                                                    ui.end_row();
                                                }
                                            });

                                        ui.separator();
                                        ui.small(
                                            "注：GPU 资源计数反映应用显式创建的 wgpu Texture/Buffer 大小；\
                                             驱动层额外开销（swapchain、depth、pipeline cache 等）\
                                             不纳入此项统计。",
                                        );

                                        if ui.button("关闭").clicked() {
                                            close = true;
                                        }
                                    });
                            });
                    });
                    if close {
                        // Hide window before closing to prevent white flash
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        *open_cb.borrow_mut() = false;
                    }
                },
            );

            if !*open.borrow() {
                self.show_mem_breakdown = false;
            }
        }

        // ── Loading overlay (independent window) ──
        if self.file_loader.is_loading() {
            let progress = self.file_loader.load_progress().clone();
            let ctx_clone = ctx.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("loading_overlay_dialog"),
                crate::chrome::dialog::viewport_builder("正在加载", [380.0, 160.0], false),
                move |vctx, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        let mut close = false;
                        crate::chrome::dialog::title_bar(ui, "正在加载", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                let p = match progress.lock() {
                                    Ok(p) => p.clone(),
                                    Err(_) => return,
                                };
                                if !p.visible {
                                    return;
                                }
                                for stage in &p.stages {
                                    ui.horizontal(|ui| {
                                        let icon = match stage.status {
                                            yinhe_editor_core::progress::StageStatus::Done => "✅",
                                            yinhe_editor_core::progress::StageStatus::Active => "⏳",
                                            yinhe_editor_core::progress::StageStatus::Pending => "⬜",
                                        };
                                        ui.label(icon);
                                        ui.add(
                                            egui::ProgressBar::new(stage.progress)
                                                .desired_width(200.0)
                                                .show_percentage(),
                                        );
                                        ui.label(egui::RichText::new(&stage.label).size(12.0));
                                    });
                                    if !stage.detail.is_empty() {
                                        ui.label(
                                            egui::RichText::new(&stage.detail)
                                                .size(10.0)
                                                .color(egui::Color32::GRAY),
                                        );
                                    }
                                }
                            });
                    });
                },
            );

            ctx.request_repaint();
        }

        // ── Archive picker dialog (independent window) ──
        if let Some(ref mut state) = self.file_loader.archive_picker {
            use crate::dialogs::archive_picker;

            let taken_state = std::rc::Rc::new(std::cell::RefCell::new(
                std::mem::replace(state, archive_picker::ArchivePickerState::Opening {
                    path: String::new(),
                    rx: std::sync::mpsc::channel().1,
                })
            ));
            let action = std::rc::Rc::new(std::cell::RefCell::new(archive_picker::ArchivePickerAction::None));
            let ctx_clone = ctx.clone();
            let taken_state_cb = taken_state.clone();
            let action_cb = action.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("archive_picker_dialog"),
                crate::chrome::dialog::viewport_builder("选择 MIDI 文件", [500.0, 400.0], true),
                move |vctx, _class| {
                    let close_requested = vctx.input(|i| i.viewport().close_requested());
                    let vctx_cmd = vctx.clone();
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                            let mut close = close_requested;
                            crate::chrome::dialog::title_bar(ui, "选择 MIDI 文件", &mut close);
                            if close {
                                // Hide window before closing to prevent white flash
                                vctx_cmd.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                                *action_cb.borrow_mut() = archive_picker::ArchivePickerAction::Cancel;
                            } else {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin {
                                        left: 12,
                                        right: 12,
                                        top: 0,
                                        bottom: 12,
                                    })
                                    .show(ui, |ui| {
                                        let result = archive_picker::show(
                                            &mut *taken_state_cb.borrow_mut(),
                                            ui,
                                        );
                                        *action_cb.borrow_mut() = result;
                                    });
                            }
                        });
                },
            );

            if let Some(taken_state) = std::rc::Rc::into_inner(taken_state) {
                *state = taken_state.into_inner();
            }

            if let Some(action) = std::rc::Rc::into_inner(action) {
                match action.into_inner() {
                    archive_picker::ArchivePickerAction::LoadFile { archive, entry } => {
                        self.file_loader.start_load_from_archive(archive, entry);
                        self.file_loader.archive_picker = None;
                    }
                    archive_picker::ArchivePickerAction::Cancel => {
                        self.file_loader.archive_picker = None;
                    }
                    archive_picker::ArchivePickerAction::Error(msg) => {
                        self.load_error = Some(msg);
                        self.file_loader.archive_picker = None;
                    }
                    archive_picker::ArchivePickerAction::None => {}
                }
                ctx.request_repaint();
            }
        }

        // ── Export progress overlay (independent window) ──
        if self.export_rx.is_some() {
            let export_progress = self.export_progress.clone();
            let ctx_clone = ctx.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("export_progress_dialog"),
                crate::chrome::dialog::viewport_builder("导出音频", [300.0, 160.0], false),
                move |vctx, _class| {
                    let state = match export_progress.lock() {
                        Ok(s) => s.clone(),
                        Err(_) => return,
                    };
                    if !state.visible {
                        return;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        let mut close = false;
                        crate::chrome::dialog::title_bar(ui, "导出音频", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new("导出音频中…")
                                            .size(18.0)
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.add_space(12.0);
                                    ui.add(
                                        egui::ProgressBar::new(state.progress)
                                            .desired_width(240.0)
                                            .fill(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
                                    );
                                    ui.add_space(8.0);
                                    if !state.status.is_empty() {
                                        ui.label(
                                            egui::RichText::new(&state.status)
                                                .size(13.0)
                                                .color(egui::Color32::LIGHT_GRAY),
                                        );
                                    }
                                });
                            });
                    });
                },
            );

            ctx.request_repaint();
        }

        // ── Export settings dialog (independent window) ──
        if self.show_export_bit_depth {
            let mut bit_depth = self.export_bit_depth;
            let mut layer_count = self.export_layer_count;
            let mut sample_rate = self.export_sample_rate;
            let global_sr = self.audio_settings.sample_rate;
            let open = std::rc::Rc::new(std::cell::RefCell::new(true));
            let started = std::rc::Rc::new(std::cell::RefCell::new(false));
            let ctx_clone = ctx.clone();
            let open_cb = open.clone();
            let started_cb = started.clone();

            ctx_clone.show_viewport_immediate(
                egui::ViewportId::from_hash_of("export_settings_dialog"),
                crate::chrome::dialog::viewport_builder("导出音频", [320.0, 260.0], false),
                move |vctx, _class| {
                    let mut close = false;
                    if vctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(ui, "导出音频", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                ui.set_max_width(280.0);
                                ui.vertical_centered(|ui| {
                                    ui.add_space(8.0);

                                    ui.horizontal(|ui| {
                                        ui.label("位深度：");
                                        let current = match &bit_depth {
                                            yinhe_audio::export::WavBitDepth::Bit16 => "16-bit",
                                            yinhe_audio::export::WavBitDepth::Bit24 => "24-bit",
                                            yinhe_audio::export::WavBitDepth::Bit32Float => "32-bit float",
                                        };
                                        egui::ComboBox::from_id_salt("export_bit_depth")
                                            .selected_text(current)
                                            .show_ui(ui, |ui| {
                                                if ui
                                                    .selectable_label(
                                                        matches!(
                                                            bit_depth,
                                                            yinhe_audio::export::WavBitDepth::Bit16
                                                        ),
                                                        "16-bit",
                                                    )
                                                    .clicked()
                                                {
                                                    bit_depth = yinhe_audio::export::WavBitDepth::Bit16;
                                                }
                                                if ui
                                                    .selectable_label(
                                                        matches!(
                                                            bit_depth,
                                                            yinhe_audio::export::WavBitDepth::Bit24
                                                        ),
                                                        "24-bit",
                                                    )
                                                    .clicked()
                                                {
                                                    bit_depth = yinhe_audio::export::WavBitDepth::Bit24;
                                                }
                                                if ui
                                                    .selectable_label(
                                                        matches!(
                                                            bit_depth,
                                                            yinhe_audio::export::WavBitDepth::Bit32Float
                                                        ),
                                                        "32-bit float",
                                                    )
                                                    .clicked()
                                                {
                                                    bit_depth =
                                                        yinhe_audio::export::WavBitDepth::Bit32Float;
                                                }
                                            });
                                    });

                                    ui.horizontal(|ui| {
                                        ui.label("采样率：");
                                        let sr_text = if sample_rate == 0 {
                                            format!("跟随全局 ({} Hz)", global_sr)
                                        } else {
                                            format!("{} Hz", sample_rate)
                                        };
                                        let sample_rates: [u32; 5] = [0, 44100, 48000, 96000, 192000];
                                        egui::ComboBox::from_id_salt("export_sample_rate")
                                            .selected_text(&sr_text)
                                            .show_ui(ui, |ui| {
                                                for &sr in &sample_rates {
                                                    let label = if sr == 0 {
                                                        format!("跟随全局 ({} Hz)", global_sr)
                                                    } else {
                                                        format!("{} Hz", sr)
                                                    };
                                                    let selected = sample_rate == sr;
                                                    if ui.selectable_label(selected, label).clicked() {
                                                        sample_rate = sr;
                                                    }
                                                }
                                            });
                                    });

                                    ui.horizontal(|ui| {
                                        ui.label("XSynth层数：");
                                        let mut layers = layer_count as usize;
                                        ui.add(
                                            egui::DragValue::new(&mut layers)
                                                .range(0..=128)
                                                .speed(1.0),
                                        );
                                        layer_count = layers as u32;
                                        if layer_count == 0 {
                                            ui.label("无限制");
                                        }
                                    });

                                    ui.add_space(12.0);

                                    if ui.button("导出").clicked() {
                                        *started_cb.borrow_mut() = true;
                                        close = true;
                                    }

                                    ui.add_space(8.0);
                                });
                            });
                    });
                    if close {
                        // Hide window before closing to prevent white flash
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        *open_cb.borrow_mut() = false;
                    }
                },
            );

            if !*open.borrow() {
                self.show_export_bit_depth = false;
            }
            self.export_bit_depth = bit_depth;
            self.export_layer_count = layer_count;
            self.export_sample_rate = sample_rate;

            if *started.borrow() {
                self.start_export();
            }
        }
    }

    /// Show the load-error modal as an independent window.
    pub(super) fn show_load_error_modal(&mut self, ui: &mut egui::Ui) {
        if self.load_error.is_some() {
            let msg = self.load_error.clone().unwrap_or_default();
            let open = std::rc::Rc::new(std::cell::RefCell::new(true));
            let ctx = ui.ctx().clone();
            let open_cb = open.clone();

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("load_error_dialog"),
                crate::chrome::dialog::viewport_builder("无法打开文件", [420.0, 120.0], false),
                move |vctx, _class| {
                    let mut close = false;
                    if vctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame {
                            fill: crate::theme::APP_BG,
                            ..Default::default()
                        })
                        .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(ui, "无法打开文件", &mut close);
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.label(&msg);
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() {
                                        close = true;
                                    }
                                });
                            });
                    });
                    if close {
                        // Hide window before closing to prevent white flash
                        vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        *open_cb.borrow_mut() = false;
                    }
                },
            );

            if !*open.borrow() {
                self.load_error = None;
            }
        }
    }
}
