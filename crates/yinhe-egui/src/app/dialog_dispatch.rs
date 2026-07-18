use eframe::egui;

use crate::app::App;

impl App {
    /// Show all overlay dialogs as independent OS windows.
    pub(in crate::app) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // 记录用户手动触发的对话框打开状态，用于"再次点击按钮→前台"的行为
        let prev_settings = self.audio_settings.show_settings;
        let prev_mem = self.show_mem_breakdown;
        let prev_export_settings = self.export.show_bit_depth;

        // ── GPU device-lost 重启提示 ──
        // 任一 RenderContext 报告 device lost 都触发：同一 device 上后注册的回调
        // 会替换先注册的，所以需要 OR 多个 RenderContext 的结果。详见
        // `RenderContext::device_lost` 的文档。GPU device lost 不可恢复，只能退出。
        let device_lost = self.render_ctx.device_lost() || self.arr_render_ctx.device_lost();

        // ── 音频设备切换检测 ──
        // 两种触发场景，都走同一个"音频设备切换"对话框：
        //
        // 1. cpal `stream_error`（设备热拔/驱动崩溃，流已死）→ 必须切换，不显示"保持当前"按钮
        // 2. 设备列表变更（插拔耳机，流还活着）→ 可选切换，显示"保持当前"按钮
        //
        // 为什么用轮询而不是 cpal 的 error callback：
        // cpal 的 `Output` 模式（绑定具体设备）只有 `DisconnectManager`，
        // 监听 `kAudioDevicePropertyDeviceIsAlive`（设备是否活着），不监听默认设备变更。
        // 插耳机时扬声器仍活着，所以 `stream_error` 不会触发。
        // 只有用 `DefaultOutput` 模式才会有 `DefaultOutputMonitor`，但那会自动 reroute，
        // 不符合"弹对话框手动选"的期望。所以这里自己轮询设备列表。
        let audio_dead = self
            .audio_state
            .handle
            .as_ref()
            .map(|h| h.handle.stream_error())
            .unwrap_or(false);
        if audio_dead && !self.audio_state.device_switch_pending {
            // 场景 1：流已死，必须切换
            self.audio_state.device_switch_pending = true;
            self.audio_state.device_switch_required = true;
            self.audio_state.device_switch_error = None;
        } else if !self.audio_state.device_switch_pending {
            // 场景 2：轮询设备列表变更（每秒一次，避免每帧调 cpal 枚举）
            let now = std::time::Instant::now();
            let should_poll = self
                .audio_state
                .last_device_poll
                .map(|t| now.duration_since(t) >= std::time::Duration::from_secs(1))
                .unwrap_or(true);
            if should_poll {
                let devices = yinhe_audio::list_output_devices();
                // 首次轮询只记录，不触发（last_known_devices 为空表示还没初始化）
                if !self.audio_state.last_known_devices.is_empty()
                    && devices != self.audio_state.last_known_devices
                {
                    // 检测到设备变更 —— 暂停播放
                    if let Some(audio) = &self.audio_state.handle {
                        audio.handle.send(yinhe_audio::AudioCommand::Pause);
                    }
                    self.audio_state.device_switch_pending = true;
                    self.audio_state.device_switch_required = false;
                    self.audio_state.device_switch_error = None;
                }
                self.audio_state.last_known_devices = devices.clone();
                self.audio_settings.available_devices = devices;
                self.audio_state.last_device_poll = Some(now);
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            }
        }

        if self.audio_state.device_switch_pending {
            use crate::dialogs::audio_device_switch::AudioDeviceSwitchAction;
            match crate::dialogs::audio_device_switch::show_viewport(
                &ctx,
                &self.audio_settings.available_devices,
                self.audio_state.device_switch_error.as_deref(),
                !self.audio_state.device_switch_required,
            ) {
                AudioDeviceSwitchAction::None => {}
                AudioDeviceSwitchAction::Switch(name) => {
                    self.switch_audio_device(name);
                }
                AudioDeviceSwitchAction::Refresh => {
                    let devices = yinhe_audio::list_output_devices();
                    self.audio_state.last_known_devices = devices.clone();
                    self.audio_settings.available_devices = devices;
                }
                AudioDeviceSwitchAction::KeepCurrent => {
                    // 用户选择保持当前设备：关闭对话框，更新 last_known_devices
                    // 避免下帧轮询又触发
                    self.audio_state.device_switch_pending = false;
                    self.audio_state.device_switch_error = None;
                    self.audio_state.last_known_devices =
                        self.audio_settings.available_devices.clone();
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

        // ── 用户手动触发对话框的首次打开 → raise_to_front ──
        // 不自动每帧置顶，仅在按钮点击后的第一帧发送 raise 命令。
        if self.audio_settings.show_settings && !prev_settings {
            crate::chrome::dialog::raise_viewport(
                &ctx,
                egui::ViewportId::from_hash_of("settings_dialog"),
            );
        }
        if self.show_mem_breakdown && !prev_mem {
            crate::chrome::dialog::raise_viewport(
                &ctx,
                egui::ViewportId::from_hash_of("memory_breakdown_dialog"),
            );
        }
        if self.export.show_bit_depth && !prev_export_settings {
            crate::chrome::dialog::raise_viewport(
                &ctx,
                egui::ViewportId::from_hash_of("export_settings_dialog"),
            );
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
