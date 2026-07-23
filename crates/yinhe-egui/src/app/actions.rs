use std::sync::mpsc;

use eframe::egui;

use crate::app::{App, PendingFileAction};
use yinhe_editor_core::document::Document;
use crate::chrome::transport_bar;

/// Actions detected from keyboard input in the current frame.
#[derive(Default)]
pub(crate) struct KeyboardActions {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub delete_selected: bool,
    pub duplicate_selected: bool,
    pub transpose_up: bool,
    pub transpose_down: bool,
    pub undo: bool,
    pub redo: bool,
    pub copy: bool,
    pub cut: bool,
    pub paste: bool,
    pub select_all: bool,
}

impl App {
    /// Handle keyboard shortcuts.
    /// Returns a `KeyboardActions` struct describing which actions were triggered.
    pub(crate) fn handle_keyboard_shortcuts(&self, ui: &egui::Ui) -> KeyboardActions {
        let mut actions = KeyboardActions::default();

        let is_playing_any = self
            .audio_state.handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);

        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                if is_playing_any {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            if i.key_pressed(egui::Key::Escape) {
                actions.stop_play = true;
            }

            // Delete / Backspace - delete selected notes
            if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
                actions.delete_selected = true;
            }

            // Ctrl+D / Cmd+D - duplicate selected notes
            if (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(egui::Key::D) {
                actions.duplicate_selected = true;
            }

            // Shift+↑ / Shift+↓ - transpose octave
            if i.modifiers.shift {
                if i.key_pressed(egui::Key::ArrowUp) {
                    actions.transpose_up = true;
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    actions.transpose_down = true;
                }
            }

            // Cmd/Ctrl+Z - undo;  Cmd/Ctrl+Shift+Z or Cmd/Ctrl+Y - redo.
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::Z) {
                if i.modifiers.shift {
                    actions.redo = true;
                } else {
                    actions.undo = true;
                }
            }
            if cmd && i.key_pressed(egui::Key::Y) {
                actions.redo = true;
            }

            // Copy / Cut / Paste / Select All
            if cmd && i.key_pressed(egui::Key::C) {
                actions.copy = true;
            }
            if cmd && i.key_pressed(egui::Key::X) {
                actions.cut = true;
            }
            if cmd && i.key_pressed(egui::Key::V) {
                actions.paste = true;
            }
            if cmd && i.key_pressed(egui::Key::A) {
                actions.select_all = true;
            }
        });

        actions
    }

    /// Delete all selected notes from the active document.
    pub(crate) fn delete_selected_notes(&mut self) {
        self.with_undo("Delete notes", |doc| doc.delete_selected());
    }

    /// Duplicate all selected notes (Ctrl+D / Cmd+D).
    /// New notes are placed after the original selection, offset by the selection duration.
    pub(crate) fn duplicate_selected_notes(&mut self) {
        self.with_undo("Duplicate notes", |doc| {
            let action = doc.duplicate_selected();
            if action.is_some() {
                doc.edit.sel_rect.pending_delta = action.as_ref().and_then(|_| {
                    // Re-derive offset from the action
                    None
                });
            }
            action
        });
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave, -12 for down).
    pub(crate) fn transpose_selected_notes(&mut self, semitones: i8) {
        let label = if semitones >= 0 {
            "Transpose up"
        } else {
            "Transpose down"
        };
        self.with_undo(label, |doc| doc.transpose_selected(semitones));
    }

    // ── Copy / Cut / Paste / Select All ──

    /// Copy selection rects to clipboard (no note data, just rects).
    /// Resets cut_past_len since a new copy invalidates the cut undo bridge.
    pub(crate) fn copy_selection(&mut self) {
        let Some(idx) = self.active_doc else { return };
        self.clipboard = self.documents[idx].edit.selected.clone();
        self.cut_past_len = None;
    }

    /// Cut: copy rects to clipboard, then delete selected notes.
    /// Stores the current undo stack length so paste can locate the
    /// correct undo entry (undo bridge) even if intervening edits occur.
    pub(crate) fn cut_selection(&mut self) {
        self.copy_selection();
        // cut_past_len is reset by copy_selection; set it before delete pushes.
        let Some(idx) = self.active_doc else { return };
        self.cut_past_len = Some(self.documents[idx].history.past_len());
        self.delete_selected_notes();
    }

    /// Paste notes from clipboard at cursor position.
    pub(crate) fn paste_clipboard(&mut self) {
        let clipboard = self.clipboard.clone();
        let cut_past_len = self.cut_past_len;
        let Some(idx) = self.active_doc else { return };
        let cursor_tick = self.documents[idx].edit.cursor_tick.unwrap_or(0.0);
        let track_selected = self.documents[idx].edit.track_selected.clone();
        self.with_undo("Paste", |doc| {
            doc.paste_from_selection(&clipboard, cursor_tick, cut_past_len, &track_selected)
        });
    }

    /// Select all notes — PR or AR depending on current view mode.
    pub(crate) fn select_all(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let is_pr = self.view_mode == crate::chrome::mode_bar::ViewMode::Edit;
        if is_pr {
            self.documents[idx].select_all_pr();
        } else {
            self.documents[idx].select_all_ar();
            // AR 视图的选框绘制读的是 arr_sel_rect，不是 doc.edit.sel_rect，
            // 所以需要同步设置。
            let model = &self.documents[idx].data.model;
            let max_end = model.tick_length as u32;
            if max_end > 0 {
                let num_tracks = model.tracks.len();
                self.arr_sel_rect = Some((0.0, max_end as f64 + 1.0, 0, num_tracks - 1));
            }
        }
        self.documents[idx].data.bump_revision();
        self.pianoroll_view.base.dirty = true;
        self.arrange_view.base.dirty = true;
    }

    /// Add a single note to the given track and record an undo entry.
    pub(crate) fn add_note_with_undo(&mut self, track_idx: u16, note: yinhe_core::NoteEvent) {
        self.with_undo("Add note", |doc| doc.add_note(track_idx, note));
    }

    /// Run an edit closure, recording an undo entry from the returned action
    /// and notifying audio afterwards.
    ///
    /// The closure receives `&mut Document` and should return
    /// `Some(UndoAction)` if it actually changed anything; on `None` no
    /// undo entry is pushed and audio is not notified.
    pub(crate) fn with_undo<F>(&mut self, label: &'static str, f: F)
    where
        F: FnOnce(&mut Document) -> Option<yinhe_editor_core::history::UndoAction>,
    {
        let Some(idx) = self.active_doc else { return };
        let action = f(&mut self.documents[idx]);
        let Some(action) = action else { return };
        let doc = &mut self.documents[idx];
        let entry = yinhe_editor_core::history::UndoEntry {
            action,
            label,
            selected: doc.edit.selected.clone(),
            track_selected: doc.edit.track_selected.clone(),
            sel_rect: doc.edit.sel_rect.clone(),
        };
        doc.history.push(entry);
        doc.data.bump_revision();
        self.pianoroll_view.base.dirty = true;
        self.arrange_view.base.dirty = true;
        // 所有 with_undo 调用方目前都是纯音符操作（delete/duplicate/transpose/
        // paste/add_note/eraser/recode_track_names），不触碰 automation lanes，
        // 所以用便宜的 UpdateNotes 路径（不重建 CC，不 chase）。
        // 如果未来有自动化编辑走 with_undo，需要改用 notify_audio_model_changed。
        self.notify_notes_changed();
    }

    /// Restore the previous state on the active document's history stack.
    pub(crate) fn undo(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let doc: &mut Document = &mut self.documents[idx];
        let changed = doc.undo();
        if changed {
            doc.data.bump_revision();
            self.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
        }
    }

    /// Re-apply the most recently undone state on the active document.
    pub(crate) fn redo(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let doc: &mut Document = &mut self.documents[idx];
        let changed = doc.redo();
        if changed {
            doc.data.bump_revision();
            self.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
        }
    }
}

impl App {
    /// Handle file menu actions from the transport bar.
    /// Checks for unsaved changes before destructive actions (New, Open, Close, Exit).
    pub(crate) fn handle_file_action(
        &mut self,
        action: transport_bar::FileAction,
        ctx: &egui::Context,
    ) {
        // Actions that never need the unsaved dialog
        match action {
            transport_bar::FileAction::Save
            | transport_bar::FileAction::SaveAs
            | transport_bar::FileAction::ExportMidi
            | transport_bar::FileAction::ExportAudio
            | transport_bar::FileAction::Settings
            | transport_bar::FileAction::Open => {
                self.execute_file_action(action, ctx);
                return;
            }
            _ => {}
        }

        // Check for unsaved changes
        if let Some(idx) = self.active_doc
            && self.documents[idx].is_dirty()
        {
            let pending = match action {
                transport_bar::FileAction::NewProject => PendingFileAction::NewProject,
                transport_bar::FileAction::Open => PendingFileAction::Open,
                transport_bar::FileAction::CloseDocument => {
                    PendingFileAction::CloseDocument(idx)
                }
                transport_bar::FileAction::Exit => PendingFileAction::Exit,
                _ => unreachable!(), // filtered above
            };
            self.pending_unsaved = Some(pending);
            return;
        }

        self.execute_file_action(action, ctx);
    }

    /// Execute a file action immediately without checking for unsaved changes.
    fn execute_file_action(&mut self, action: transport_bar::FileAction, ctx: &egui::Context) {
        match action {
            transport_bar::FileAction::NewProject => {
                self.documents.push(Document::empty());
                self.active_doc = Some(self.documents.len() - 1);
                self.teardown_audio();
            }
            transport_bar::FileAction::Open => {
                self.file_loader.pick_file(self.audio_settings.midi_import_encoding);
            }
            transport_bar::FileAction::Save => {
                if let Some(idx) = self.active_doc {
                    let path = self.documents[idx].file_path.clone();
                    if let Some(path) = path {
                        self.save_project_async(idx, path);
                    } else {
                        self.save_as_dialog();
                    }
                }
            }
            transport_bar::FileAction::SaveAs => {
                self.save_as_dialog();
            }
            transport_bar::FileAction::CloseDocument => {
                if let Some(idx) = self.active_doc {
                    self.close_document(idx);
                }
            }
            transport_bar::FileAction::ExportMidi => {
                self.export_midi_dialog();
            }
            transport_bar::FileAction::ExportAudio => {
                self.export_audio_dialog(ctx);
            }
            transport_bar::FileAction::Exit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            transport_bar::FileAction::Settings => {
                self.audio_settings.show_settings = true;
                crate::chrome::dialog::raise_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("settings_dialog"),
                );
            }
        }
    }

    /// Execute the deferred pending action (called after save completes or on discard).
    pub(crate) fn execute_pending_file_action(&mut self, _ctx: &egui::Context) {
        let Some(pending) = self.pending_unsaved.take() else { return };
        match pending {
            PendingFileAction::NewProject => {
                self.documents.push(Document::empty());
                self.active_doc = Some(self.documents.len() - 1);
                self.teardown_audio();
            }
            PendingFileAction::Open => {
                self.file_loader.pick_file(self.audio_settings.midi_import_encoding);
            }
            PendingFileAction::CloseDocument(idx) => {
                self.close_document(idx);
            }
            PendingFileAction::Exit => {
                self.should_exit = true;
            }
        }
    }

    /// Spawn a background thread to save the project.
    pub(crate) fn save_project_async(&mut self, idx: usize, path: String) {
        let doc = &mut self.documents[idx];
        doc.sync_overrides_to_model();
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();

        // Sync SF state into project_file
        doc.data.project_file.soundfont_project_mode =
            !self.audio_settings.global_sf_config.global_enabled;
        doc.data.project_file.soundfont_overrides = doc
            .edit
            .project_sf
            .overrides
            .iter()
            .map(|(port, entries)| yinhe_yin::SfPortOverride {
                port: *port,
                entries: entries
                    .iter()
                    .map(|e| yinhe_yin::SfEntryJson {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect(),
            })
            .collect();

        let model = doc.data.model.clone();
        let project_file = doc.data.project_file.clone();
        let mapping_file = doc.data.mapping_file.clone();
        let path_for_thread = path.clone();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Err(e) = yinhe_yin::save_yin_with_files(
                &model,
                &path_for_thread,
                &project_file,
                &mapping_file,
            ) {
                tracing::error!("Failed to save project: {}", e);
            }
            let _ = tx.send(());
        });

        if let Some(doc) = self.documents.get_mut(idx) {
            doc.file_path = Some(path);
        }
        self.save_rx = Some(rx);
    }

    pub(crate) fn save_as_dialog(&mut self) {
        let default_name = if let Some(idx) = self.active_doc {
            format!("{}.yin", self.documents[idx].file_name)
        } else {
            "Untitled.yin".to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Yinhe Project", &["yin"])
            .set_file_name(&default_name)
            .save_file()
        {
            let mut path_str = path.to_string_lossy().to_string();
            // Ensure .yin extension
            if !path_str.ends_with(".yin") {
                path_str.push_str(".yin");
            }
            if let Some(idx) = self.active_doc {
                let path2 = path_str.clone();
                self.save_project_async(idx, path2);
                // Update file_name
                if let Some(doc) = self.documents.get_mut(idx) {
                    doc.file_name = path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                }
            }
        }
    }

    fn export_midi_dialog(&mut self) {
        let default_name = if let Some(idx) = self.active_doc {
            format!("{}.mid", self.documents[idx].file_name)
        } else {
            "export.mid".to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .set_file_name(&default_name)
            .save_file()
        {
            let path_str = path.to_string_lossy().to_string();
            if let Some(idx) = self.active_doc {
                let doc = &self.documents[idx];
                match yinhe_mid2::write_to_bytes(&doc.data.model) {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&path_str, &bytes) {
                            tracing::error!("Failed to export MIDI: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to export MIDI: {}", e);
                    }
                }
            }
        }
    }

    fn export_audio_dialog(&mut self, ctx: &egui::Context) {
        if self.active_doc.is_none() {
            return;
        }

        if self.export.rx.is_some() {
            return; // already exporting
        }

        // Show export settings dialog first
        self.export.show_bit_depth = true;
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("export_settings_dialog"),
        );
    }

    /// Called after the bit-depth dialog is confirmed.
    /// Opens the file-save dialog and starts the export.
    pub(crate) fn start_export(&mut self) {
        let idx = match self.active_doc {
            Some(idx) => idx,
            None => return,
        };

        let doc = &self.documents[idx];
        let default_name = format!("{}.wav", doc.file_name);

        let path = match rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_file_name(&default_name)
            .save_file()
        {
            Some(p) => p,
            None => return,
        };

        let mut path_str = path.to_string_lossy().to_string();
        if !path_str.ends_with(".wav") {
            path_str.push_str(".wav");
        }

        // Collect render inputs
        let model = doc.data.model.clone();
        let sr = if self.export.sample_rate > 0 {
            self.export.sample_rate
        } else {
            self.audio_settings.sample_rate
        };
        let port_sf = self.resolve_sf_config(doc);
        eprintln!("[export] port_sf = {:?}", port_sf);
        let skip = doc.compute_skip_mask();
        let bit_depth = self.export.bit_depth;
        let layer_count = if self.export.layer_count == 0 {
            None
        } else {
            Some(self.export.layer_count as usize)
        };
        let export_progress = self.export.progress.clone();
        let cancel_flag = self.export.cancel.clone();
        let use_gpu_synth = self.audio_settings.use_gpu_synth;
        cancel_flag.store(false, std::sync::atomic::Ordering::Relaxed);

        // Reset progress state
        {
            let mut p = export_progress.lock().unwrap();
            p.reset();
        }

        let (tx, rx) = mpsc::channel();

        // Try GPU export first — use the app's existing wgpu Device/Queue.
        #[cfg(feature = "gpu")]
        let gpu_device = std::sync::Arc::new(self.render_ctx.device().clone());
        #[cfg(feature = "gpu")]
        let gpu_queue = std::sync::Arc::new(self.render_ctx.queue().clone());
        // Extract SFZ path from port_sf for GPU export.
        #[cfg(feature = "gpu")]
        let gpu_sfz = port_sf.first().and_then(|(_, paths)| paths.first()).cloned();
        #[cfg(feature = "gpu")]
        eprintln!("[export] gpu_sfz = {:?}", gpu_sfz);
        #[cfg(not(feature = "gpu"))]
        eprintln!("[export] GPU feature NOT enabled");

        std::thread::spawn(move || {
            eprintln!("[export] Thread started");
            // 根据设置选择导出引擎：GPU 还是 CPU
            #[cfg(feature = "gpu")]
            let result = if use_gpu_synth {
                if let Some(ref sfz) = gpu_sfz {
                    eprintln!("[export] Using GPU path (GpuSynth), SFZ: {}", sfz);
                    yinhe_audio::export::export_wav_gpu(
                        model,
                        sr,
                        std::path::Path::new(sfz),
                        &skip,
                        std::path::Path::new(&path_str),
                        bit_depth,
                        |pct, msg| {
                            if let Ok(mut p) = export_progress.lock() {
                                p.progress = pct;
                                if !msg.is_empty() {
                                    p.status = msg.to_string();
                                }
                            }
                        },
                        gpu_device,
                        gpu_queue,
                    )
                } else {
                    eprintln!("[export] GPU selected but no SFZ path, fallback to CPU.");
                    yinhe_audio::export::export_wav(
                        model,
                        sr,
                        &port_sf,
                        &skip,
                        std::path::Path::new(&path_str),
                        bit_depth,
                        layer_count,
                        |pct, msg| {
                            if let Ok(mut p) = export_progress.lock() {
                                p.progress = pct;
                                if !msg.is_empty() {
                                    p.status = msg.to_string();
                                }
                            }
                        },
                        Some(export_progress.clone()),
                        Some(cancel_flag),
                    )
                }
            } else {
                // 用户选择 CPU 引擎 — 使用 xsynth 导出。
                eprintln!("[export] Using CPU path (xsynth).");
                yinhe_audio::export::export_wav(
                    model,
                    sr,
                    &port_sf,
                    &skip,
                    std::path::Path::new(&path_str),
                    bit_depth,
                    layer_count,
                    |pct, msg| {
                        if let Ok(mut p) = export_progress.lock() {
                            p.progress = pct;
                            if !msg.is_empty() {
                                p.status = msg.to_string();
                            }
                        }
                    },
                    Some(export_progress.clone()),
                    Some(cancel_flag),
                )
            };

            #[cfg(not(feature = "gpu"))]
            let result = yinhe_audio::export::export_wav(
                model,
                sr,
                &port_sf,
                &skip,
                std::path::Path::new(&path_str),
                bit_depth,
                layer_count,
                |pct, msg| {
                    if let Ok(mut p) = export_progress.lock() {
                        p.progress = pct;
                        if !msg.is_empty() {
                            p.status = msg.to_string();
                        }
                    }
                },
                Some(export_progress.clone()),
                Some(cancel_flag),
            );
            // Capture final stats before hiding the progress window.
            let (elapsed, speed) = {
                let p = export_progress.lock().unwrap();
                let elapsed = p.started_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
                (elapsed, p.overall_speed)
            };
            // Mark done
            if let Ok(mut p) = export_progress.lock() {
                p.visible = false;
            }
            match result {
                Ok(()) => {
                    let _ = tx.send(Ok((path_str, elapsed, speed)));
                }
                Err(yinhe_audio::export::ExportError::Cancelled) => {
                    // User cancelled — hide progress silently, don't send error.
                    drop(tx);
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                }
            }
        });

        self.export.rx = Some(rx);
    }
}
