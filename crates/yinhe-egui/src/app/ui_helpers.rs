use std::sync::Arc;

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

        let (sel_action, note_drag_delta) = {
            let mut guard = super::main_loop::ReplaceGuard::new(&mut self.documents[idx]);
            let doc = guard.as_mut();
            let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                Some(doc.data.model.as_ref());
            let piano_rect = egui::Rect::from_min_max(
                egui::pos2(layout.remaining.min.x, layout.bottom_y),
                layout.remaining.max,
            );

            let mut action = None;
            let mut note_drag_delta: Option<(i64, i32)> = None;
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
                    let automation_lanes: Vec<yinhe_types::AutomationLane> = Vec::new();
                    action = piano_view::show(
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
                        &mut self.audio_settings.automation_display_mode,
                        &mut self.audio_settings.automation_show_dots,
                        &mut note_drag_delta,
                        &mut doc.edit.sel_rect,
                        doc.data.midi_version,
                    );
                    if let Some(t0) = _piano_total_start {
                        yinhe_memtrace::perf_probe::record_piano_total(t0.elapsed());
                    }
                },
            );
            (action, note_drag_delta)
        };

        // Handle selection actions
        if let Some(action) = sel_action {
            use crate::widgets::selection_actions::SelectionAction;
            match action {
                SelectionAction::Delete => self.delete_selected_notes(),
                SelectionAction::Duplicate => self.duplicate_selected_notes(),
                SelectionAction::TransposeUp => self.transpose_selected_notes(12),
                SelectionAction::TransposeDown => self.transpose_selected_notes(-12),
            }
        }

        // Handle note drag
        self.handle_note_drag(note_drag_delta);
    }

    /// Handle note drag updates or finalize the drag.
    fn handle_note_drag(&mut self, note_drag_delta: Option<(i64, i32)>) {
        if let Some((delta_ticks, delta_keys)) = note_drag_delta {
            let Some(idx) = self.active_doc else {
                return;
            };
            let doc = &mut self.documents[idx];
            if doc.edit.selected.is_empty() {
                self.note_drag_originals = None;
                self.note_drag_undo_snapshot = None;
                self.note_drag_moved = false;
            } else {
                if self.note_drag_originals_note.is_none() {
                    self.note_drag_undo_snapshot =
                        Some(doc.snapshot_with_selection("Move notes"));
                    self.note_drag_moved = false;
                    let mut originals = Vec::new();
                    let model = &doc.data.model;
                    for &(track, start_tick, key) in &doc.edit.selected {
                        let t = track as usize;
                        if t < model.tracks.len() {
                            if let Some(note) = model.tracks[t].notes
                                .iter()
                                .find(|n| n.key == key as u8 && n.start_tick == start_tick)
                            {
                                originals.push((*note, key, track));
                            }
                        }
                    }
                    self.note_drag_originals_note = Some(originals);
                }

                if delta_ticks != 0 || delta_keys != 0 {
                    self.note_drag_moved = true;
                }

                if let Some(ref originals) = self.note_drag_originals_note.clone() {
                    {
                        let model = Arc::make_mut(&mut doc.data.model);
                        // Remove selected notes from their tracks
                        for &(track, start_tick, key) in &doc.edit.selected {
                            let t = track as usize;
                            if t < model.tracks.len() {
                                let td = Arc::make_mut(&mut model.tracks[t]);
                                td.notes
                                    .retain(|n| !(n.key == key as u8 && n.start_tick == start_tick));
                            }
                        }
                        let mut new_selected = std::collections::HashSet::new();
                        for (note, old_key, track) in originals {
                            let t = *track as usize;
                            if t < model.tracks.len() {
                                let new_key =
                                    ((*old_key as i32) + delta_keys).clamp(0, 127) as u8;
                                let new_tick =
                                    (note.start_tick as i64 + delta_ticks).max(0) as u32;
                                let length = note.end_tick - note.start_tick;
                                let moved = yinhe_core::NoteEvent {
                                    start_tick: new_tick,
                                    end_tick: new_tick + length,
                                    key: new_key,
                                    velocity: note.velocity,
                                    dup_index: 0,
                                };
                                let td = Arc::make_mut(&mut model.tracks[t]);
                                let insert_pos =
                                    td.notes.partition_point(|n| n.start_tick < moved.start_tick);
                                td.notes.insert(insert_pos, moved);
                                new_selected.insert((*track, new_tick, new_key));
                            }
                        }
                        doc.edit.selected = new_selected;
                        model.rebuild();
                    }
                    doc.data.midi_version = doc.data.midi_version.wrapping_add(1);
                    self.pianoroll_view.base.dirty = true;
                }
            }
        } else {
            // Drag ended — finalize
            if let Some(_originals) = self.note_drag_originals_note.take() {
                if let Some(idx) = self.active_doc {
                    let doc = &mut self.documents[idx];
                    doc.data.rebuild_model();
                    doc.data.midi_version = doc.data.midi_version.wrapping_add(1);
                    self.pianoroll_view.base.dirty = true;
                    let snap = self.note_drag_undo_snapshot.take();
                    let moved = std::mem::replace(&mut self.note_drag_moved, false);
                    if moved {
                        if let Some(snap) = snap {
                            doc.history.push(snap);
                        }
                    }
                    if let Some(ref audio) = self.audio {
                        let _ =
                            audio
                                .handle
                                .send(yinhe_audio::AudioCommand::ReloadNotes { model: doc.data.model.clone() });
                    }
                }
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

        // Request repaint during playback
        let is_audio_playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        if is_audio_playing {
            ui.ctx().request_repaint();
        }
    }

    /// Show all overlay dialogs: loading, archive picker, export progress, export settings, error modal.
    pub(super) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        // Loading overlay
        self.file_loader.show_loading_overlay(ui);
        if self.file_loader.is_loading() {
            ui.ctx().request_repaint();
        }

        // Archive picker dialog
        if let Some(ref mut state) = self.file_loader.archive_picker {
            use crate::dialogs::archive_picker;
            match archive_picker::show(state, ui.ctx()) {
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
            ui.ctx().request_repaint();
        }

        // Export progress overlay
        if self.export_rx.is_some() {
            crate::dialogs::export::show_export_progress(ui, &self.export_progress);
            ui.ctx().request_repaint();
        }

        // Export bit-depth dialog
        if self.show_export_bit_depth {
            let result = crate::dialogs::export::show_export_settings_dialog(
                ui.ctx(),
                &mut self.export_bit_depth,
                &mut self.export_layer_count,
                &mut self.export_sample_rate,
                self.audio_settings.sample_rate,
                &mut self.show_export_bit_depth,
            );
            if result.started {
                self.start_export();
            }
        }
    }

    /// Show the load-error modal dialog.
    pub(super) fn show_load_error_modal(&mut self, ui: &mut egui::Ui) {
        if self.load_error.is_some() {
            let screen_rect = ui.ctx().content_rect();
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    "load_error_overlay".into(),
                ))
                .rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
                );

            let mut dismiss = false;
            egui::Window::new("无法打开文件")
                .order(egui::Order::Tooltip)
                .collapsible(false)
                .resizable(false)
                .movable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    if let Some(msg) = &self.load_error {
                        ui.set_max_width(420.0);
                        ui.label(msg);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("确定").clicked() {
                                dismiss = true;
                            }
                        });
                    }
                });
            if dismiss {
                self.load_error = None;
            }
        }
    }
}
