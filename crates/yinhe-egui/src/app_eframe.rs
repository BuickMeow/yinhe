use eframe::egui;
use std::collections::HashSet;
use std::sync::Arc;

use crate::app::App;
use crate::dialogs::file_loader::LoadResult;
use crate::document::Document;
use crate::widgets::title_bar;

use crate::arrange;
use crate::piano_view;
use crate::widgets::mode_bar;
use crate::widgets::theme;
use crate::widgets::transport_bar;

// ── Panic-safe take guard ──
/// Restores a taken value back into its slot on drop, preventing data loss
/// if a panic occurs between `std::mem::take` and the manual put-back.
struct ReplaceGuard<'a, T> {
    slot: &'a mut T,
    value: Option<T>,
}

impl<'a, T> ReplaceGuard<'a, T> {
    fn new(slot: &'a mut T) -> Self
    where
        T: Default,
    {
        let value = std::mem::take(slot);
        ReplaceGuard {
            slot,
            value: Some(value),
        }
    }

    fn as_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("ReplaceGuard already consumed")
    }
}

impl<'a, T> Drop for ReplaceGuard<'a, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            *self.slot = value;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let _ui_total_start = if crate::perf_probe::enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };
        // ── Full-viewport background (matching title bar / transport bar) ──
        let bg = crate::widgets::theme::APP_BG;
        ui.painter().rect_filled(ui.ctx().viewport_rect(), 0.0, bg);

        // ── Detect document switch → invalidate GPU caches ──
        if self.active_doc != self.prev_active_doc {
            self.arrange_view.base.dirty = true;
            self.pianoroll_view.base.dirty = true;
            self.prev_active_doc = self.active_doc;
        }

        // ── Force dark mode ──
        ui.ctx().set_visuals(egui::Visuals::dark());

        // ── Custom title bar ──
        let title_bar_action = title_bar::show(
            ui,
            &self.documents,
            &mut self.active_doc,
            &mut self.title_bar_press_pos,
        );
        // Handle deferred title bar actions (e.g. close a document)
        if let Some(title_bar::TitleBarAction::CloseDocument(idx)) = title_bar_action {
            self.close_document(idx);
        }

        // ── Defensive: ensure active_doc is always in bounds ──
        if let Some(idx) = self.active_doc
            && idx >= self.documents.len()
        {
            self.active_doc = if self.documents.is_empty() {
                None
            } else {
                Some(self.documents.len() - 1)
            };
        }

        // ── Keyboard shortcuts ──
        let kb = self.handle_keyboard_shortcuts(ui);

        // Handle note editing shortcuts
        if kb.delete_selected {
            self.delete_selected_notes();
        }
        if kb.duplicate_selected {
            self.duplicate_selected_notes();
        }
        if kb.transpose_up {
            self.transpose_selected_notes(12);
        }
        if kb.transpose_down {
            self.transpose_selected_notes(-12);
        }
        if kb.undo {
            self.undo();
        }
        if kb.redo {
            self.redo();
        }

        // ── System resource monitoring ──
        self.refresh_system_stats();

        // ── Poll async file loading ──
        match self.file_loader.poll_loading() {
            LoadResult::MidiLoaded { path, midi } => {
                let quantize = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| doc.quantize)
                    .unwrap_or_default();
                match Document::from_midi(&path, midi, quantize) {
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
            LoadResult::YinLoaded { path, midi, file_name } => {
                let quantize = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| doc.quantize)
                    .unwrap_or_default();
                let result = Document::from_yin(&path, quantize)
                    .ok()
                    .or_else(|| {
                        // Fallback: build from the embedded MIDI directly.
                        Document::from_midi(&file_name, midi, quantize)
                            .ok()
                            .map(|d| (d, false))
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
            LoadResult::NotReady => {}
        }

        // ── Poll async save completion ──
        if let Some(rx) = &self.save_rx {
            if rx.try_recv().is_ok() {
                self.save_rx = None;
            }
        }

        // ── Poll async export completion ──
        if let Some(rx) = &self.export_rx {
            if let Ok(result) = rx.try_recv() {
                self.export_rx = None;
                if let Err(e) = result {
                    self.load_error = Some(e);
                }
            }
        }

        // ── Ensure audio engine is loaded for the active document ──
        self.rebuild_audio_if_needed();

        // ── Transport bar (renders before handle_playback so button clicks
        //    are processed in the same frame as keyboard shortcuts) ──
        let active_doc = self.active_doc.and_then(|idx| self.documents.get(idx));
        let transport_response = transport_bar::show(
            ui,
            &mut transport_bar::TransportContext {
                file_loader: &mut self.file_loader,
                doc: active_doc,
                cpu_usage: self.sys_monitor.cpu_usage,
                mem_mb: self.sys_monitor.mem_mb,
                follow_mode: &mut self.follow_mode,
                show_mem_breakdown: &mut self.show_mem_breakdown,
            },
        );

        // ── Handle playback actions (merge keyboard + transport bar inputs) ──
        self.handle_playback(
            kb.toggle_play || transport_response.toggle_play,
            kb.pause_return || transport_response.pause_return,
            kb.stop_play || transport_response.stop_play,
        );

        if let (Some(idx), Some(new_preset)) =
            (self.active_doc, transport_response.pending_quantize)
            && let Some(doc) = self.documents.get_mut(idx)
        {
            doc.quantize = new_preset;
        }

        // ── Memory breakdown popup ──
        self.show_memory_breakdown(ui);

        // ── Handle file menu actions ──
        if let Some(action) = transport_response.pending_file_action {
            self.handle_file_action(action, ui.ctx());
        }

        // ── Settings panel ──
        let settings_changed = crate::dialogs::settings::show(ui, &mut self.audio_settings);
        if settings_changed {
            self.teardown_audio();
        }

        // ── MIDI encoding change: re-decode track names in active document ──
        let new_enc = self.audio_settings.midi_import_encoding;
        if new_enc != self.last_midi_encoding {
            self.last_midi_encoding = new_enc;
            if self.active_doc.is_some() {
                self.with_undo("Recode track names", |doc| {
                    doc.recode_track_names(new_enc);
                    true
                });
            }
        }

        // ── Bottom mode bar ──
        mode_bar::show(
            ui,
            &mut self.view_mode,
            &mut self.show_pianoroll_in_arrange,
            &mut self.show_transport,
            &mut self.show_pianoroll,
            &mut self.right_tab,
        );

        // ── Main area: arrangement (top) + pianoroll (bottom) ──
        let mut remaining = ui.available_rect_before_wrap();

        // ── Reserve space for tool panels (one per visible section) ──
        let has_arr = self.show_transport && self.active_doc.is_some();
        let has_piano = self.show_pianoroll && self.active_doc.is_some();
        let tools_panel_w = if has_arr || has_piano {
            crate::widgets::tools_panel::TOOLS_PANEL_W
        } else {
            0.0
        };
        remaining.max.x -= tools_panel_w;

        // ── Reserve space for right panel ──
        let right_panel_total_w = if self.right_tab.is_some() {
            let max_w = (remaining.width() - 60.0).max(theme::RIGHT_PANEL_MIN_WIDTH + 4.0);
            let pw =
                (self.right_panel_width + 4.0).clamp(theme::RIGHT_PANEL_MIN_WIDTH + 4.0, max_w);
            self.right_panel_width = (pw - 4.0).max(theme::RIGHT_PANEL_MIN_WIDTH);
            pw
        } else {
            0.0
        };
        remaining.max.x -= right_panel_total_w;

        let total = remaining.size();
        let arr_h = if self.show_transport && self.active_doc.is_some() {
            if self.show_pianoroll {
                (total.y * self.arr_split).max(theme::MIN_ARR_HEIGHT)
            } else {
                total.y
            }
        } else {
            0.0
        };
        let bottom_y = remaining.min.y
            + arr_h
            + if self.show_transport && self.show_pianoroll && self.active_doc.is_some() {
                theme::SPLIT_GAP
            } else {
                0.0
            };

        if let Some(idx) = self.active_doc {
            let is_playing = self
                .audio
                .as_ref()
                .map(|a| a.handle.is_playing())
                .unwrap_or(false);
            let mut follow_mode = self.follow_mode;

            // ── Arrangement view (transport track panel + arrangement GPU) ──
            if self.show_transport {
                let mut request_pianoroll = false;
                let mut guard = ReplaceGuard::new(&mut self.documents[idx]);
                arrange::show(
                    ui,
                    guard.as_mut(),
                    &mut self.arrange_view,
                    remaining,
                    arr_h,
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
                // guard drops here → document restored even on panic
            }

            // ── Pianoroll area ──
            if self.show_pianoroll {
                // Horizontal splitter (between arrangement and pianoroll)
                if self.show_transport {
                    let split_right = remaining.max.x + tools_panel_w;
                    let h_split_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h),
                        egui::pos2(split_right, remaining.min.y + arr_h + theme::SPLIT_GAP),
                    );
                    let h_int_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h + 0.5),
                        egui::pos2(split_right, remaining.min.y + arr_h + theme::SPLIT_GAP),
                    );
                    let h_split_resp =
                        crate::widgets::split_handle::horizontal(ui, "__h_split__", h_int_rect);
                    ui.painter().rect_filled(
                        h_split_rect,
                        0.0,
                        if h_split_resp.hovered() || h_split_resp.dragged() {
                            theme::SPLIT_HOVER
                        } else {
                            theme::SPLIT_DEFAULT
                        },
                    );
                    if h_split_resp.dragged() {
                        let delta = h_split_resp.drag_delta().y;
                        self.arr_split = ((arr_h + delta) / total.y)
                            .clamp(theme::SPLIT_CLAMP_MIN, theme::SPLIT_CLAMP_MAX);
                    }
                }

                // Pianoroll GPU view (full width, no track panel) — inner block
                // ensures guard drops before we call self.* methods below.
                let auto_wgpu_state = self.render_ctx.wgpu_state().clone();
                // Ensure controller_renderers has an entry for this document
                while self.controller_renderers.len() <= idx {
                    self.controller_renderers.push(Vec::new());
                }
                let (sel_action, note_drag_delta) = {
                    let mut guard = ReplaceGuard::new(&mut self.documents[idx]);
                    let doc = guard.as_mut();
                    let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                        Some(&*doc.midi as &dyn yinhe_pianoroll::NoteSource);
                    let piano_rect =
                        egui::Rect::from_min_max(egui::pos2(remaining.min.x, bottom_y), remaining.max);

                    let mut action = None;
                    let mut note_drag_delta: Option<(i64, i32)> = None;
                    ui.scope_builder(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                        let _piano_total_start = if crate::perf_probe::enabled() {
                            Some(std::time::Instant::now())
                        } else {
                            None
                        };
                        // Effective pianoroll visibility = track_visible AND track_selected.
                        // Conductor track acts as "Master" — shows all tracks.
                        let show_all = doc
                            .conductor_track_idx
                            .map(|c| doc.track_selected.contains(&c))
                            .unwrap_or(false);
                        let pr_visible: Vec<bool> = (0..doc.track_visible.len())
                            .map(|i| {
                                if show_all {
                                    doc.track_visible[i]
                                } else {
                                    doc.track_visible[i]
                                        && doc.track_selected.contains(&(i as u16))
                                }
                            })
                            .collect();
                        action = piano_view::show(
                            ui,
                            ui.available_size(),
                            &mut self.pianoroll,
                            &mut self.render_ctx,
                            &mut self.pianoroll_view,
                            midi_source,
                            &mut doc.selected,
                            &pr_visible,
                            &doc.track_colors_cache,
                            &mut doc.cursor_tick,
                            is_playing,
                            doc.quantize,
                            doc.midi.ticks_per_beat,
                            Some((
                                doc.midi.ticks_per_beat,
                                doc.midi.time_sig_numerator,
                                doc.midi.time_sig_denominator,
                                doc.midi.time_sig_events.as_slice(),
                            )),
                            &mut self.piano_last_cursor_tick,
                            &mut follow_mode,
                            &self.active_tool,
                            // Automation panel data
                            Some(&mut doc.controller_panels),
                            Some(&mut self.controller_renderers[idx]),
                            Some(&doc.midi.automation_lanes),
                            Some(&mut doc.show_controller_panels),
                            Some(&auto_wgpu_state),
                            self.audio_settings.scroll_mode,
                            self.audio_settings.min_border_width,
                            &mut self.audio_settings.velocity_display_mode,
                            &mut self.audio_settings.automation_display_mode,
                            &mut self.audio_settings.automation_show_dots,
                            &mut note_drag_delta,
                            doc.midi_version,
                        );
                        if let Some(t0) = _piano_total_start {
                            crate::perf_probe::record_piano_total(t0.elapsed());
                        }
                    });
                    (action, note_drag_delta)
                    // guard drops here → document restored even on panic
                };
                // Handle floating action bar clicks (after guard/doc borrow is dropped)
                if let Some(action) = sel_action {
                    use crate::widgets::selection_actions::SelectionAction;
                    match action {
                        SelectionAction::Delete => self.delete_selected_notes(),
                        SelectionAction::Duplicate => self.duplicate_selected_notes(),
                        SelectionAction::TransposeUp => self.transpose_selected_notes(12),
                        SelectionAction::TransposeDown => self.transpose_selected_notes(-12),
                    }
                }
                // Handle note drag (after guard/doc borrow is dropped)
                if let Some((delta_ticks, delta_keys)) = note_drag_delta {
                    if let Some(idx) = self.active_doc {
                        let doc = &mut self.documents[idx];
                        if doc.selected.is_empty() {
                            self.note_drag_originals = None;
                            self.note_drag_undo_snapshot = None;
                            self.note_drag_moved = false;
                        } else {
                            // Save originals on first frame of drag
                            if self.note_drag_originals.is_none() {
                                // Capture undo snapshot before any mutation.
                                self.note_drag_undo_snapshot =
                                    Some(crate::history::Snapshot::capture(doc, "Move notes"));
                                self.note_drag_moved = false;
                                let mut originals = Vec::new();
                                let midi = &doc.midi;
                                for &(track, start_tick, key) in &doc.selected {
                                    if let Some(note) = midi.key_notes[key as usize]
                                        .iter()
                                        .find(|n| n.track == track && n.start_tick == start_tick)
                                    {
                                        originals.push((note.clone(), key));
                                    }
                                }
                                self.note_drag_originals = Some(originals);
                            }

                            if delta_ticks != 0 || delta_keys != 0 {
                                self.note_drag_moved = true;
                            }

                            // Rebuild notes from originals + delta each frame.
                            if let Some(ref originals) = self.note_drag_originals.clone() {
                                {
                                    let midi = Arc::make_mut(&mut doc.midi);
                                    // Clear all current selected notes
                                    for &(track, start_tick, key) in &doc.selected {
                                        midi.key_notes[key as usize].retain(|n| !(n.track == track && n.start_tick == start_tick));
                                    }
                                    // Re-insert at new positions
                                    let mut new_selected = HashSet::new();
                                    for (note, old_key) in originals {
                                        let new_key = ((*old_key as i32) + delta_keys).clamp(0, 127) as u8;
                                        let new_start = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                                        let new_end = (note.end_tick as i64 + delta_ticks).max(1) as u32;
                                        let moved = yinhe_types::Note {
                                            start_tick: new_start,
                                            end_tick: new_end,
                                            ..note.clone()
                                        };
                                        let notes = &mut midi.key_notes[new_key as usize];
                                        let insert_pos = notes.partition_point(|n| n.start_tick < moved.start_tick);
                                        notes.insert(insert_pos, moved);
                                        new_selected.insert((note.track, new_start, new_key));
                                    }
                                    doc.selected = new_selected;
                                }
                                doc.midi_version = doc.midi_version.wrapping_add(1);
                                self.pianoroll_view.base.dirty = true;
                            }
                        }
                    }
                } else {
                    // Drag ended — finalize
                    if let Some(_originals) = self.note_drag_originals.take() {
                        if let Some(idx) = self.active_doc {
                            let doc = &mut self.documents[idx];
                            {
                                let midi = Arc::make_mut(&mut doc.midi);
                                crate::app_actions::rebuild_midi_metadata(midi);
                            }
                            doc.midi_version = doc.midi_version.wrapping_add(1);
                            self.pianoroll_view.base.dirty = true;
                            // Push the pre-drag snapshot onto the undo stack
                            // only if the drag actually moved notes.
                            let snap = self.note_drag_undo_snapshot.take();
                            let moved = std::mem::replace(&mut self.note_drag_moved, false);
                            if moved {
                                if let Some(snap) = snap {
                                    doc.history.push(snap);
                                }
                            }
                            if let Some(ref audio) = self.audio {
                                let _ = audio.handle.send(yinhe_audio::AudioCommand::ReloadNotes {
                                    midi: Arc::clone(&doc.midi),
                                });
                            }
                        }
                    }
                }
            }

            self.follow_mode = follow_mode;
        }

        // ── Tool panels ──
        let tools_x = remaining.max.x;
        if has_arr {
            let rect = egui::Rect::from_min_size(
                egui::pos2(tools_x, remaining.min.y),
                egui::vec2(tools_panel_w, arr_h),
            );
            crate::widgets::tools_panel::show(
                ui,
                rect,
                &mut self.active_tool,
                &crate::widgets::tools_panel::ALL_TOOLS,
            );
        }
        if has_piano {
            let rect = egui::Rect::from_min_size(
                egui::pos2(tools_x, bottom_y),
                egui::vec2(tools_panel_w, remaining.max.y - bottom_y),
            );
            crate::widgets::tools_panel::show(
                ui,
                rect,
                &mut self.active_tool,
                &crate::widgets::tools_panel::ALL_TOOLS,
            );
        }

        // ── Right panel ──
        if self.right_tab.is_some() {
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(tools_x + tools_panel_w, remaining.min.y),
                egui::vec2(right_panel_total_w, remaining.height()),
            );
            let doc = self.active_doc.and_then(|idx| self.documents.get_mut(idx));
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

        // ── Request repaint during playback ──
        let is_audio_playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        if is_audio_playing {
            ui.ctx().request_repaint();
        }

        // ── Loading overlay ──
        self.file_loader.show_loading_overlay(ui);
        if self.file_loader.is_loading() {
            ui.ctx().request_repaint();
        }

        // ── Export progress overlay ──
        if self.export_rx.is_some() {
            crate::dialogs::export::show_export_progress(ui, &self.export_progress);
            ui.ctx().request_repaint();
        }

        // ── Export bit-depth dialog ──
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

        // ── Load-error modal ──
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

        if let Some(t0) = _ui_total_start {
            crate::perf_probe::record_ui_total(t0.elapsed());
        }
    }
}
