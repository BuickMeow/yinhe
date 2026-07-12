use eframe::egui;

use crate::app::App;

impl App {
    /// Show all overlay dialogs as independent OS windows.
    pub(in crate::app) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── Settings dialog ──
        if crate::dialogs::settings::show_viewport(
            &ctx,
            &mut self.audio_settings,
            &mut self.haptic_engine,
            &self.audio,
        ) {
            self.teardown_audio();
        }

        // ── Memory breakdown ──
        #[cfg(target_os = "macos")]
        let metal_size = self
            .render_ctx
            .metal_allocated_size()
            .unwrap_or(0)
            .saturating_add(self.arr_render_ctx.metal_allocated_size().unwrap_or(0));
        #[cfg(not(target_os = "macos"))]
        let metal_size = 0u64;
        crate::dialogs::memory_breakdown::show_viewport(
            &ctx,
            &mut self.show_mem_breakdown,
            self.sys_monitor.mem_mb,
            metal_size,
        );

        // ── Loading overlay ──
        if self.file_loader.is_loading() {
            let progress = self.file_loader.load_progress().clone();
            if crate::dialogs::loading_overlay::show_viewport(&ctx, progress) {
                self.file_loader.cancel_loading();
            }
        }

        // ── Archive picker ──
        let picker_action = crate::dialogs::archive_picker::show_viewport(
            &ctx,
            &mut self.file_loader.archive_picker,
        );
        use crate::dialogs::archive_picker::ArchivePickerAction;
        let picker_handled = !matches!(picker_action, ArchivePickerAction::None);
        match picker_action {
            ArchivePickerAction::LoadFile { archive, entry } => {
                self.file_loader.start_load_from_archive(archive, entry);
                self.file_loader.archive_picker = None;
            }
            ArchivePickerAction::Cancel => {
                self.file_loader.archive_picker = None;
            }
            ArchivePickerAction::Error(ref msg) => {
                self.load_error = Some(msg.clone());
                self.file_loader.archive_picker = None;
            }
            ArchivePickerAction::None => {}
        }
        if picker_handled {
            ctx.request_repaint();
        }

        // ── Export progress ──
        if self.export_rx.is_some() {
            let export_progress = self.export_progress.clone();
            let cancel_flag = self.export_cancel.clone();
            crate::dialogs::export::show_progress_viewport(&ctx, export_progress, cancel_flag);
        }

        // ── Export completed ──
        crate::dialogs::export::show_completed_viewport(&ctx, &mut self.export_completed);

        // ── Export settings ──
        if crate::dialogs::export::show_settings_viewport(
            &ctx,
            &mut self.show_export_bit_depth,
            self.audio_settings.sample_rate,
            &mut self.export_bit_depth,
            &mut self.export_layer_count,
            &mut self.export_sample_rate,
        ) {
            self.start_export();
        }
    }

    /// Show the load-error modal as an independent window.
    pub(in crate::app) fn show_load_error_modal(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── Load error ──
        crate::dialogs::load_error::show_viewport(&ctx, &mut self.load_error);

        // ── Unsaved changes confirmation ──
        let action = crate::dialogs::unsaved::show_viewport(
            &ctx,
            &self.pending_unsaved,
            &self.save_rx,
        );
        match action {
            crate::dialogs::unsaved::Action::Save => {
                if let Some(idx) = self.active_doc {
                    if let Some(path) = self.documents[idx].file_path.clone() {
                        self.save_project_async(idx, path);
                    } else {
                        self.save_as_dialog();
                    }
                }
            }
            crate::dialogs::unsaved::Action::Discard => {
                let ctx = ui.ctx().clone();
                self.execute_pending_file_action(&ctx);
            }
            crate::dialogs::unsaved::Action::Cancel => {
                self.pending_unsaved = None;
            }
            crate::dialogs::unsaved::Action::None => {}
        }
    }
}
