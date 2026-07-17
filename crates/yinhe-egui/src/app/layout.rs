use eframe::egui;

use crate::app::App;
use crate::arrange;
use crate::piano_view;

/// Layout geometry computed once per frame, shared by arrangement and pianoroll.
pub(in crate::app) struct LayoutInfo {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub bottom_y: f32,
    pub tools_panel_w: f32,
    pub right_panel_total_w: f32,
    pub has_arr: bool,
    pub has_piano: bool,
}

impl App {
    /// Compute layout geometry for the current frame.
    pub(in crate::app) fn compute_layout(&mut self, ui: &mut egui::Ui) -> LayoutInfo {
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
    pub(in crate::app) fn show_main_content(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
        let Some(idx) = self.active_doc else {
            return;
        };

        let is_playing = self
            .audio_state.handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        let mut follow_mode = self.follow_mode;

        // Arrangement view
        let (arr_drag_delta, arr_eraser_rect, arr_quantize): (Option<(i64, i32)>, Option<(f64, f64, usize, usize)>, Option<yinhe_editor_core::quantize::QuantizePreset>) = if self.show_transport {
            let mut request_pianoroll = false;
            let mut arr_drag_delta: Option<(i64, i32)> = None;
            let mut arr_eraser_rect: Option<(f64, f64, usize, usize)> = None;
            let mut guard = crate::app::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
            let arr_quantize = arrange::show(
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
                self.audio_state.handle.as_ref(),
                &mut request_pianoroll,
                &mut self.track_selection_anchor,
                self.audio_settings.scroll_mode,
                self.audio_settings.min_border_width,
                Some(&self.haptic_engine),
                &mut self.arr_sel_rect,
                &mut arr_drag_delta,
                &mut arr_eraser_rect,
                &mut self.info_content,
            );
            if request_pianoroll {
                self.show_pianoroll = true;
                self.show_pianoroll_in_arrange = true;
            }
            (arr_drag_delta, arr_eraser_rect, arr_quantize) // guard dropped here
        } else {
            (None, None, None)
        };

        // Handle AR eraser (guard is dropped, no outstanding borrow on self.documents)
        if let Some((t_start, t_end, track_lo, track_hi)) = arr_eraser_rect {
            let mut sel = yinhe_core::Selection::default();
            sel.add_rect_track(t_start as u32, t_end as u32, 0, 127, track_lo as u16, track_hi as u16);
            let Some(idx) = self.active_doc else { return };
            self.documents[idx].edit.selected = sel;
            self.with_undo("Eraser delete (arrange)", |doc| doc.delete_selected());
        }

        // Handle AR drag after guard is dropped (no outstanding borrow on self.documents)
        if let Some((delta_ticks, delta_tracks)) = arr_drag_delta {
            self.handle_arr_drag(delta_ticks, delta_tracks);
        }

        // Handle AR quantize preset change from corner button
        if let Some(new_preset) = arr_quantize {
            if let Some(doc) = self.documents.get_mut(idx) {
                doc.edit.quantize_arrange = new_preset;
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
            let mut guard = crate::app::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
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
                        self.render_thread.as_ref(),
                        &mut self.pianoroll_view,
                        &mut self.last_cull_midi_version,
                        midi_source,
                        &mut doc.edit.selected,
                        &pr_visible,
                        &doc.edit.track_colors_cache,
                        &mut doc.edit.cursor_tick,
                        is_playing,
                        doc.edit.quantize_pianoroll,
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
                        self.audio_settings.note_outline,
                        &tempo_events,
                        &mut note_drag_delta,
                        &mut doc.edit.sel_rect,
                        &doc.edit.track_selected,
                        doc.edit.conductor_track_idx,
                        doc.data.midi_version,
                        Some(&self.haptic_engine),
                        &mut pencil_note_drag,
                        &mut auto_edit_events,
                        &mut self.info_content,
                        &mut self.right_tab,
                        &mut self.automation_drag_ghost,
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
                PianoViewEvent::EraserDelete { t_start, t_end, key_lo, key_hi, track_lo, track_hi } => {
                    let Some(idx) = self.active_doc else { return };
                    let mut sel = yinhe_core::Selection::default();
                    sel.add_rect_track(t_start, t_end, key_lo, key_hi, track_lo, track_hi);
                    self.documents[idx].edit.selected = sel;
                    self.with_undo("Eraser delete", |doc| doc.delete_selected());
                }
                PianoViewEvent::QuantizePreset(preset) => {
                    let Some(idx) = self.active_doc else { return };
                    self.documents[idx].edit.quantize_pianoroll = preset;
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
            self.notify_audio_model_changed();
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
                self.notify_audio_model_changed();
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
            self.notify_audio_model_changed();
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

        if let Some(action) = doc.move_selected_arrange(delta_ticks, delta_tracks) {
            self.arrange_view.base.dirty = true;
            doc.history.push(yinhe_editor_core::history::UndoEntry {
                action,
                label: "Move in arrange",
                selected: doc.edit.selected.clone(),
                track_selected: doc.edit.track_selected.clone(),
                sel_rect: doc.edit.sel_rect.clone(),
            });
            self.notify_audio_model_changed();
        }
    }

    /// Show tool panels, right panel, and request repaint if playing.
    pub(in crate::app) fn show_panels_and_overlays(&mut self, ui: &mut egui::Ui, layout: &LayoutInfo) {
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
                self.audio_state.handle.as_ref(),
                &mut self.event_browser_state,
                &mut self.info_content,
                self.automation_drag_ghost,
            );
            if changed {
                self.teardown_audio();
            }
        }

        // Request repaint during playback (or while waiting for audio thread to start)
        let is_audio_playing = self
            .audio_state.handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        if is_audio_playing || self.audio_state.pending_playback {
            ui.ctx().request_repaint();
        }
    }
}
