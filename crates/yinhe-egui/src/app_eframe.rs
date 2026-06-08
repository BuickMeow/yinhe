use eframe::egui;

use crate::app::App;
use crate::dialogs::file_loader::MidiLoadResult;
use crate::document::Document;
use crate::widgets::title_bar;

use crate::arrange;
use crate::piano_view;
use crate::widgets::mode_bar;
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
        // ── Full-viewport background (matching title bar / transport bar) ──
        let bg = crate::widgets::theme::APP_BG;
        ui.painter().rect_filled(ui.ctx().screen_rect(), 0.0, bg);

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
        let (kb_toggle, kb_pause, kb_stop) = self.handle_keyboard_shortcuts(ui);

        // ── System resource monitoring ──
        self.refresh_system_stats();

        // ── Poll async MIDI loading ──
        match self.file_loader.poll_midi_loading() {
            MidiLoadResult::Loaded { path, midi } => {
                // Inherit quantize from the current active document.
                let quantize = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| doc.quantize)
                    .unwrap_or_default();

                let doc = Document::from_midi(&path, midi, quantize);
                let insert_idx = self.documents.len();
                self.documents.push(doc);
                self.active_doc = Some(insert_idx);
                self.teardown_audio();
            }
            MidiLoadResult::NotReady => {}
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
            kb_toggle || transport_response.toggle_play,
            kb_pause || transport_response.pause_return,
            kb_stop || transport_response.stop_play,
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

        // ── Bottom mode bar ──
        mode_bar::show(
            ui,
            &mut self.view_mode,
            &mut self.show_pianoroll_in_arrange,
            &mut self.show_transport,
            &mut self.show_pianoroll,
        );

        // ── RACK view ──
        if self.view_mode == crate::widgets::mode_bar::ViewMode::Rack {
            let rack_doc = self.active_doc.and_then(|idx| self.documents.get_mut(idx));
            let changed = crate::rack::show(ui, &mut self.audio_settings, rack_doc);
            if changed {
                self.teardown_audio();
            }
            // Skip arrangement/pianoroll rendering
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
            self.file_loader.show_midi_loading_overlay(ui);
            if self.file_loader.is_loading() {
                ui.ctx().request_repaint();
            }
            return;
        }

        // ── Main area: arrangement (top) + pianoroll (bottom) ──
        let remaining = ui.available_rect_before_wrap();

        if let Some(idx) = self.active_doc {
            let total = remaining.size();
            let is_playing = self
                .audio
                .as_ref()
                .map(|a| a.handle.is_playing())
                .unwrap_or(false);
            let mut follow_mode = self.follow_mode;

            let arr_h = if self.show_transport {
                if self.show_pianoroll {
                    (total.y * self.arr_split).max(crate::widgets::theme::MIN_ARR_HEIGHT)
                } else {
                    total.y
                }
            } else {
                0.0
            };
            let bottom_y = remaining.min.y
                + arr_h
                + if self.show_transport && self.show_pianoroll {
                    crate::widgets::theme::SPLIT_GAP
                } else {
                    0.0
                };

            // ── Arrangement view (transport track panel + arrangement GPU) ──
            if self.show_transport {
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
                );
                // guard drops here → document restored even on panic
            }

            // ── Pianoroll area ──
            if self.show_pianoroll {
                let mut guard = ReplaceGuard::new(&mut self.documents[idx]);

                // Horizontal splitter (between arrangement and pianoroll)
                // Interact rect inset 0.5px at top so it never shares a
                // boundary with the arrangement scrollbar above.
                if self.show_transport {
                    let h_split_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h),
                        egui::pos2(
                            remaining.max.x,
                            remaining.min.y + arr_h + crate::widgets::theme::SPLIT_GAP,
                        ),
                    );
                    let h_int_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h + 0.5),
                        egui::pos2(
                            remaining.max.x,
                            remaining.min.y + arr_h + crate::widgets::theme::SPLIT_GAP,
                        ),
                    );
                    let h_split_resp =
                        crate::widgets::split_handle::horizontal(ui, "__h_split__", h_int_rect);
                    // Overdraw visual rect — interaction rect is inset 0.5px
                    ui.painter().rect_filled(
                        h_split_rect,
                        0.0,
                        if h_split_resp.hovered() || h_split_resp.dragged() {
                            crate::widgets::theme::SPLIT_HOVER
                        } else {
                            crate::widgets::theme::SPLIT_DEFAULT
                        },
                    );
                    if h_split_resp.dragged() {
                        let delta = h_split_resp.drag_delta().y;
                        self.arr_split = ((arr_h + delta) / total.y).clamp(
                            crate::widgets::theme::SPLIT_CLAMP_MIN,
                            crate::widgets::theme::SPLIT_CLAMP_MAX,
                        );
                    }
                }

                // Pianoroll GPU view (full width, no track panel)
                let doc = guard.as_mut();
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    Some(&*doc.midi as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect =
                    egui::Rect::from_min_max(egui::pos2(remaining.min.x, bottom_y), remaining.max);

                // Clone wgpu_state for automation panels before closure borrows render_ctx
                let auto_wgpu_state = self.render_ctx.wgpu_state().clone();
                let auto_lanes = doc.midi.automation_lanes.clone();
                // Ensure controller_renderers has an entry for this document
                while self.controller_renderers.len() <= idx {
                    self.controller_renderers.push(Vec::new());
                }

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui,
                        ui.available_size(),
                        &mut self.pianoroll,
                        &mut self.render_ctx,
                        &mut self.pianoroll_view,
                        midi_source,
                        &doc.selected,
                        &doc.track_visible,
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
                        // Automation panel data
                        Some(&mut doc.controller_panels),
                        Some(&mut self.controller_renderers[idx]),
                        Some(&auto_lanes),
                        Some(&mut doc.show_controller_panels),
                        Some(&auto_wgpu_state),
                    );
                });
                // guard drops here → document restored even on panic
            }

            self.follow_mode = follow_mode;
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
        self.file_loader.show_midi_loading_overlay(ui);
        if self.file_loader.is_loading() {
            ui.ctx().request_repaint();
        }
    }
}
