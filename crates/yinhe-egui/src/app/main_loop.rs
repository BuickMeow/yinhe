use eframe::egui;

use crate::app::{App, PendingFileAction};
use crate::chrome::title_bar;

use crate::chrome::mode_bar;
use crate::chrome::transport_bar;

// ── Panic-safe take guard ──
/// Restores a taken value back into its slot on drop, preventing data loss
/// if a panic occurs between `std::mem::take` and the manual put-back.
pub(super) struct ReplaceGuard<'a, T> {
    slot: &'a mut T,
    value: Option<T>,
}

impl<'a, T> ReplaceGuard<'a, T> {
    pub(super) fn new(slot: &'a mut T) -> Self
    where
        T: Default,
    {
        let value = std::mem::take(slot);
        ReplaceGuard {
            slot,
            value: Some(value),
        }
    }

    pub(super) fn as_mut(&mut self) -> &mut T {
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _ui_total_start = if yinhe_memtrace::perf_probe::enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // ── Full-viewport background ──
        ui.painter()
            .rect_filled(ui.ctx().viewport_rect(), 0.0, crate::theme::APP_BG);

        // ── Intercept native close (macOS traffic light, Alt+F4, etc.) ──
        let close_requested = ui.ctx().input(|i| i.viewport().close_requested());
        if close_requested && !self.should_exit {
            let any_dirty = self.documents.iter().any(|d| d.is_dirty());
            if any_dirty {
                // Cancel the close and show the unsaved dialog instead
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.pending_unsaved = Some(PendingFileAction::Exit);
                // 让 Dock 栏图标跳动，提示用户注意
                crate::platform::request_user_attention();
            }
            // If no dirty documents, let the close proceed normally
        }

        // ── macOS: update document-edited dot in traffic light ──
        let any_dirty = self.documents.iter().any(|d| d.is_dirty());
        if any_dirty != self.last_dirty_state {
            self.last_dirty_state = any_dirty;
            crate::platform::set_document_edited(frame, any_dirty);
        }

        // ── macOS: poll native menu bar actions ──
        for action in self.menu_bar.poll() {
            use crate::platform::MenuAction;
            let file_action = match action {
                MenuAction::NewProject => transport_bar::FileAction::NewProject,
                MenuAction::Open => transport_bar::FileAction::Open,
                MenuAction::Save => transport_bar::FileAction::Save,
                MenuAction::SaveAs => transport_bar::FileAction::SaveAs,
                MenuAction::CloseDocument => transport_bar::FileAction::CloseDocument,
                MenuAction::Undo => {
                    self.undo();
                    continue;
                }
                MenuAction::Redo => {
                    self.redo();
                    continue;
                }
                MenuAction::Cut => {
                    self.cut_selection();
                    continue;
                }
                MenuAction::Copy => {
                    self.copy_selection();
                    continue;
                }
                MenuAction::Paste => {
                    self.paste_clipboard();
                    continue;
                }
                MenuAction::SelectAll => {
                    self.select_all();
                    continue;
                }
            };
            self.handle_file_action(file_action, ui.ctx());
        }

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
            &mut self.tab_scroll_offset,
        );
        if let Some(title_bar::TitleBarAction::CloseDocument(idx)) = title_bar_action {
            if self.documents.get(idx).map_or(false, |d| d.is_dirty()) {
                self.pending_unsaved = Some(PendingFileAction::CloseDocument(idx));
            } else {
                self.close_document(idx);
            }
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
        if kb.copy {
            self.copy_selection();
        }
        if kb.cut {
            self.cut_selection();
        }
        if kb.paste {
            self.paste_clipboard();
        }
        if kb.select_all {
            self.select_all();
        }

        // ── Live FPS (real, EMA-smoothed from egui frame delta) ──
        let dt = ui.input(|i| i.stable_dt);
        let inst_fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        // Reset EMA when frames were paused (e.g. UI was idle), otherwise a
        // stale high value (like 140 fps from quick mouse movement) would
        // take tens of frames to decay back to the real rate (~60 fps).
        let stale = dt > 0.1 || self.fps <= 0.0;
        self.fps = if stale {
            inst_fps
        } else {
            self.fps * 0.9 + inst_fps * 0.1
        };

        // ── System resource monitoring ──
        self.refresh_system_stats();

        // ── Poll async operations ──
        self.poll_async_operations();

        // ── Handle deferred exit ──
        // 不重置 should_exit：保持 true 直到窗口真正关闭，
        // 这样下一帧 close_requested 为 true 时，
        // 守卫 !self.should_exit 为 false，不会重新拦截并弹窗。
        if self.should_exit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // ── Ensure audio engine is loaded for the active document ──
        self.rebuild_audio_if_needed();

        // ── Transport bar ──
        let active_doc = self.active_doc.and_then(|idx| self.documents.get(idx));
        let transport_response = transport_bar::show(
            ui,
            &mut transport_bar::TransportContext {
                file_loader: &mut self.file_loader,
                doc: active_doc,
                follow_mode: &mut self.follow_mode,
            },
        );

        // mode_bar 的 MEM 数字：memtrace 开启时用分类追踪的堆内存，
        // 关闭时用系统 RSS（sys_monitor）。
        let mem_mb = if yinhe_memtrace::enabled() {
            yinhe_memtrace::Snapshot::capture().total_mb()
        } else {
            self.sys_monitor.mem_mb
        };

        // ── Handle playback actions ──
        self.handle_playback(
            kb.toggle_play || transport_response.toggle_play,
            kb.pause_return || transport_response.pause_return,
            kb.stop_play || transport_response.stop_play,
        );

        // ── Smooth cursor interpolation between audio callback updates ──
        self.interpolate_playback_cursor();

        // ── Handle file menu actions ──
        if let Some(action) = transport_response.pending_file_action {
            self.handle_file_action(action, ui.ctx());
        }

        // ── MIDI encoding change ──
        let new_enc = self.audio_settings.midi_import_encoding;
        if new_enc != self.last_midi_encoding {
            self.last_midi_encoding = new_enc;
            if self.active_doc.is_some() {
                self.with_undo("Recode track names", |doc| {
                    doc.recode_track_names(new_enc);
                    None::<yinhe_editor_core::history::UndoAction>
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
            self.sys_monitor.cpu_usage,
            mem_mb,
            self.fps,
            &mut self.show_mem_breakdown,
        );

        // ── Main content area ──
        let layout = self.compute_layout(ui);
        self.show_main_content(ui, &layout);
        self.show_panels_and_overlays(ui, &layout);
        self.show_dialogs(ui);
        self.show_load_error_modal(ui);
        self.sync_automation_density();

        if let Some(t0) = _ui_total_start {
            yinhe_memtrace::perf_probe::record_ui_total(t0.elapsed());
        }
    }
}
