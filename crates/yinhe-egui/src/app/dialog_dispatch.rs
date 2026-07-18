use eframe::egui;

use crate::app::App;

impl App {
    /// Show all overlay dialogs as independent OS windows.
    pub(in crate::app) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── GPU device-lost 重启提示 ──
        // 任一 RenderContext 报告 device lost 都触发：同一 device 上后注册的回调
        // 会替换先注册的，所以需要 OR 多个 RenderContext 的结果。详见
        // `RenderContext::device_lost` 的文档。GPU device lost 不可恢复，只能退出。
        let device_lost = self.render_ctx.device_lost() || self.arr_render_ctx.device_lost();

        // ── 音频流错误 → 设备切换对话框 ──
        // cpal `stream_error` 一旦置位就不可恢复，但和 GPU device lost 不一样：
        // 音频可以换一个输出设备重建流。所以分流到"音频设备切换"对话框，
        // 让用户挑新设备；只有 GPU device lost 才走"需要重启"退出对话框。
        let audio_dead = self
            .audio_state
            .handle
            .as_ref()
            .map(|h| h.handle.stream_error())
            .unwrap_or(false);
        if audio_dead && !self.audio_state.device_switch_pending {
            // 首次检测到音频流断开 —— 弹设备切换对话框
            self.audio_state.device_switch_pending = true;
            self.audio_state.device_switch_error = None;
        }
        if self.audio_state.device_switch_pending {
            use crate::dialogs::audio_device_switch::AudioDeviceSwitchAction;
            match crate::dialogs::audio_device_switch::show_viewport(
                &ctx,
                &self.audio_settings.available_devices,
                self.audio_state.device_switch_error.as_deref(),
            ) {
                AudioDeviceSwitchAction::None => {}
                AudioDeviceSwitchAction::Switch(name) => {
                    self.switch_audio_device(name);
                }
                AudioDeviceSwitchAction::Refresh => {
                    self.audio_settings.available_devices = yinhe_audio::list_output_devices();
                }
                AudioDeviceSwitchAction::Exit => {
                    self.should_exit = true;
                }
            }
        }

        if device_lost && crate::dialogs::gpu_device_lost::show_viewport(&ctx) {
            self.should_exit = true;
        }

        // ── Settings dialog ──
        if crate::dialogs::settings::show_viewport(
            &ctx,
            &mut self.audio_settings,
            &mut self.haptic_engine,
            &self.audio_state.handle,
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
        if self.export.rx.is_some() {
            let export_progress = self.export.progress.clone();
            let cancel_flag = self.export.cancel.clone();
            crate::dialogs::export::show_progress_viewport(&ctx, export_progress, cancel_flag);
        }

        // ── Export completed ──
        crate::dialogs::export::show_completed_viewport(&ctx, &mut self.export.completed);

        // ── Export settings ──
        if crate::dialogs::export::show_settings_viewport(
            &ctx,
            &mut self.export.show_bit_depth,
            self.audio_settings.sample_rate,
            &mut self.export.bit_depth,
            &mut self.export.layer_count,
            &mut self.export.sample_rate,
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
