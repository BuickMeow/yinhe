use eframe::egui;
use rust_i18n::t;

use crate::app::App;

impl App {
    /// Show all overlay dialogs as independent OS windows.
    pub(in crate::app) fn show_dialogs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── GPU device-lost 处理 ──
        // 全局标志：所有 RenderContext（含自动化面板）共享同一回调，
        // 不再依赖 OR 多个实例的结果。见 `render_context::device_lost_global`。
        let device_lost = crate::render_context::device_lost_global();

        // ── spawn 失败检测 ──
        // rebuild_audio_if_needed 里 spawn_cpal_audio 失败时设置 spawn_error。
        // 这里检测到后弹出设备切换对话框（必须切换，不显示"保持当前"按钮），
        // 让用户选一个可用设备。spawn_error 会在用户选设备走 switch_audio_device
        // 时清除，或在切换文档/改设置时清除。
        if self.audio_state.spawn_error.is_some() && !self.audio_state.device_switch_pending {
            self.audio_state.device_switch_pending = true;
            self.audio_state.device_switch_required = true;
            self.audio_state.device_switch_error = self.audio_state.spawn_error.clone();
        }

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

        if device_lost {
            // 自动保全 + 重启恢复；失败时内部回退到手动重启弹窗
            self.handle_device_lost(&ctx);
        }

        // ── 自动保存恢复询问 ──
        if self.autosave.show_recovery_dialog {
            let entries = self.autosave.recovery.clone().unwrap_or_default();
            match crate::dialogs::autosave_recovery::show_viewport(
                &ctx,
                &entries,
                &mut self.autosave.show_recovery_dialog,
            ) {
                crate::dialogs::autosave_recovery::RecoveryAction::Restore => {
                    self.restore_autosave_entries(entries);
                }
                crate::dialogs::autosave_recovery::RecoveryAction::Discard => {
                    self.discard_recovery(&entries);
                }
                crate::dialogs::autosave_recovery::RecoveryAction::None => {}
            }
        }

        // ── Settings dialog ──
        let prev_allow = self.audio_settings.allow_overlapping_notes;
        let prev_behavior = self.audio_settings.overlap_blocked_behavior;
        // show_viewport 的返回值语义是「设置窗口已关闭」（dialogs/settings.rs 末尾
        // `!settings.show_settings`），不是「有修改」：直接用它会变成"开关一下
        // 设置窗口就重建音频引擎"（音色库解析/采样上传/预热全部重来）。
        // 只有真正影响 spawn 的设置变化才 teardown。
        let prev_engine = EngineSettingsKey::of(&self.audio_settings);
        if crate::dialogs::settings::show_viewport(
            &ctx,
            &mut self.audio_settings,
            &self.audio_state.handle,
        ) && prev_engine.differs(&EngineSettingsKey::of(&self.audio_settings))
        {
            self.teardown_audio();
        }
        if self.audio_settings.allow_overlapping_notes != prev_allow
            || self.audio_settings.overlap_blocked_behavior != prev_behavior
        {
            for doc in &mut self.workspace.documents {
                doc.edit.allow_overlapping_notes = self.audio_settings.allow_overlapping_notes;
                doc.edit.overlap_blocked_behavior = self.audio_settings.overlap_blocked_behavior;
            }
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

        // ── Loading toast（替代 loading_overlay viewport）──
        // 保留 archive_picker/password 仍为 dialog，仅文件加载阶段用 toast
        // 若 toast 正在离开（用户点了 X/取消），不再 upsert，避免复活
        if self
            .notifications
            .is_leaving(crate::widgets::toast::LOADING_PROGRESS_ID)
        {
            // 检测取消：toast 的 stop 按钮置位 cancel flag 则真正取消加载
            if let Some(flag) = self
                .notifications
                .get_cancel_flag(crate::widgets::toast::LOADING_PROGRESS_ID)
                && flag.load(std::sync::atomic::Ordering::Relaxed)
            {
                self.file_loader.cancel_loading();
                self.notifications
                    .dismiss(crate::widgets::toast::LOADING_PROGRESS_ID);
            }
        } else if self.file_loader.is_loading() {
            // 检测 toast 侧取消（用户点 stop 按钮置位 cancel flag；X 只收起不取消）
            if let Some(flag) = self
                .notifications
                .get_cancel_flag(crate::widgets::toast::LOADING_PROGRESS_ID)
                && flag.load(std::sync::atomic::Ordering::Relaxed)
            {
                self.file_loader.cancel_loading();
                self.notifications
                    .dismiss(crate::widgets::toast::LOADING_PROGRESS_ID);
            } else if self.file_loader.progress_visible() {
                // 卡片只建一次，进度文案渲染时 pull，不再每帧拷贝
                let src = std::sync::Arc::new(self.file_loader.toast_source());
                self.notifications.ensure_progress(
                    crate::widgets::toast::LOADING_PROGRESS_ID,
                    crate::widgets::toast::ToastKind::Info,
                    src,
                );
            }
        }

        // ── 保存进度 toast（替代 save_overlay viewport）──
        // “准备中…”由 source 在状态缺失时自行返回，卡片只建一次，进度渲染时 pull
        if self
            .notifications
            .is_leaving(crate::widgets::toast::SAVE_PROGRESS_ID)
        {
        } else if self.save_rx.is_some() {
            let src = std::sync::Arc::new(crate::dialogs::save_overlay::SaveToastSource {
                state: self.save_progress.clone(),
            });
            self.notifications.ensure_progress(
                crate::widgets::toast::SAVE_PROGRESS_ID,
                crate::widgets::toast::ToastKind::Info,
                src,
            );
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
                self.show_error(t!("toast.open_failed"), msg);
                self.file_loader.archive_picker = None;
            }
            ArchivePickerAction::None => {}
        }
        if picker_handled {
            ctx.request_repaint();
        }

        // ── Archive password prompt ──
        let pwd_action = crate::dialogs::archive_picker::show_password_prompt_viewport(
            &ctx,
            &mut self.file_loader.password_prompt,
        );
        use crate::dialogs::archive_picker::PasswordPromptAction;
        let pwd_handled = !matches!(pwd_action, PasswordPromptAction::None);
        match pwd_action {
            PasswordPromptAction::Confirm { path, password } => {
                yinhe_editor_core::progress::set_visible(self.file_loader.load_progress(), true);
                self.file_loader.start_archive(path, Some(password));
                self.file_loader.password_prompt = None;
            }
            PasswordPromptAction::Cancel => {
                self.file_loader.password_prompt = None;
            }
            PasswordPromptAction::None => {}
        }
        if pwd_handled {
            ctx.request_repaint();
        }

        // ── Export progress toast（替代 export_progress viewport）──
        if self
            .notifications
            .is_leaving(crate::widgets::toast::EXPORT_PROGRESS_ID)
        {
            // 用户点了 X（»）只是收起卡片，不置 cancel 标志，任务继续后台跑；
            // 只有 stop 按钮才会置位 cancel flag（下一帧 poll 线程退出）。
        } else if self.export.running {
            // 若 toast 侧点了取消，已置位则不再建卡，直接让线程退出
            let cancelled = self
                .notifications
                .get_cancel_flag(crate::widgets::toast::EXPORT_PROGRESS_ID)
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed));
            if !cancelled {
                let src = std::sync::Arc::new(crate::app::export_state::ExportToastSource {
                    progress: self.export.progress.clone(),
                    cancel: self.export.cancel.clone(),
                    pause: self.export.pause.clone(),
                });
                self.notifications.ensure_progress(
                    crate::widgets::toast::EXPORT_PROGRESS_ID,
                    crate::widgets::toast::ToastKind::Info,
                    src,
                );
            }
        }

        // ── PPQ rescale progress toast（替代 rescale_overlay viewport）──
        if self
            .notifications
            .is_leaving(crate::widgets::toast::RESCALE_PROGRESS_ID)
        {
        } else if self.rescale.rx.is_some() {
            let cancelled = self
                .notifications
                .get_cancel_flag(crate::widgets::toast::RESCALE_PROGRESS_ID)
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed));
            if !cancelled {
                let src = std::sync::Arc::new(crate::app::rescale_state::RescaleToastSource {
                    progress: self.rescale.progress.clone(),
                    cancel: self.rescale.cancel.clone(),
                });
                self.notifications.ensure_progress(
                    crate::widgets::toast::RESCALE_PROGRESS_ID,
                    crate::widgets::toast::ToastKind::Info,
                    src,
                );
            }
        }

        // ── PPQ rescale 确认对话框（标准 viewport 形式）──
        // project_info.rs 检测到 PPQ 变更且有音符时，写入 ctx memory pending。
        // 这里每帧检测 pending，弹出独立 viewport 确认框，用户选择后执行操作。
        self.show_ppq_rescale_confirm(&ctx);

        // ── 新建音轨对话框 ──
        self.show_new_track_dialog(&ctx);

        // ── 敲击测速对话框 ──
        self.show_tap_tempo_dialog(&ctx);

        // ── 选择筛选对话框 ──
        self.show_filter_dialog(&ctx);

        // ── 插件参数自动化选择窗口 ──
        self.show_automation_picker(&ctx);

        // ── 属性浮动面板（音轨属性与侧栏互斥；工程设置仅浮窗）──
        self.show_float_panels(&ctx);

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

    /// 显示未保存确认弹窗（独立窗口）。
    pub(in crate::app) fn show_unsaved_dialog(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        // ── Unsaved changes confirmation ──
        let action =
            crate::dialogs::unsaved::show_viewport(&ctx, &self.pending_unsaved, &self.save_rx);
        match action {
            crate::dialogs::unsaved::Action::Save => {
                if let Some(idx) = self.workspace.active_doc {
                    if let Some(path) = self.workspace.documents[idx].file_path.clone() {
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

    /// 检测 ctx memory 中的 PPQ rescale pending，弹出独立 viewport 确认框。
    ///
    /// 用户选择后执行对应操作并清除 pending：
    /// - **Rescale**：还原 meta.ppq = old，写出 `RescaleRequest`（main_loop 启动异步线程）。
    /// - **NoRescale**：rebuild_tempo_map + commit_ppq(rescale=false)。
    /// - **Cancel**：还原 meta.ppq = old，清除 pending edit（不推 undo）。
    fn show_ppq_rescale_confirm(&mut self, ctx: &egui::Context) {
        let pending: Option<(u32, u32, u64)> = ctx.data(|d| {
            d.get_temp(egui::Id::new(
                crate::right_panel::project_info::PPQ_RESCALE_PENDING_ID,
            ))
        });
        let Some((old_val, new_val, dragvalue_id)) = pending else {
            return;
        };

        let action = crate::dialogs::ppq_rescale_confirm::show_viewport(ctx, old_val, new_val);
        if action == crate::dialogs::ppq_rescale_confirm::PpqRescaleAction::None {
            return; // 用户还没选择，保持弹框打开
        }

        let Some(doc_idx) = self.workspace.active_doc else {
            return;
        };
        let Some(doc) = self.workspace.documents.get_mut(doc_idx) else {
            return;
        };

        use crate::dialogs::ppq_rescale_confirm::PpqRescaleAction;
        match action {
            PpqRescaleAction::Rescale => {
                // 异步 rescale：先把 meta.ppq 还原为 old_val（子线程用 old_ppq 作基准），
                // 再写出 RescaleRequest 让 main_loop 启动子线程。
                // commit_ppq 在 poll.rs 检测到子线程完成后才调用。
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                model.meta.ppq = old_val;
                ctx.data_mut(|d| {
                    d.insert_temp(
                        egui::Id::new(crate::app::rescale_state::RESCALE_REQUEST_ID),
                        crate::app::rescale_state::RescaleRequest {
                            old_ppq: old_val,
                            new_ppq: new_val,
                            dragvalue_id,
                        },
                    )
                });
            }
            PpqRescaleAction::NoRescale => {
                // 不 rescale，但 rebuild_tempo_map（meta.ppq 已是 new_val）。
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                model.rebuild_tempo_map();
                yinhe_editor_core::history::commit_ppq(doc, dragvalue_id, new_val, false);
            }
            PpqRescaleAction::Cancel => {
                // 取消：还原 meta.ppq = old_val，清掉 pending edit（不推 undo）。
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                model.meta.ppq = old_val;
                doc.edit.pending_edits.take(dragvalue_id);
            }
            PpqRescaleAction::None => unreachable!(),
        }

        // 清除 pending（dialog_dispatch 已处理完，避免下帧重复弹）。
        ctx.data_mut(|d| {
            d.remove::<(u32, u32, u64)>(egui::Id::new(
                crate::right_panel::project_info::PPQ_RESCALE_PENDING_ID,
            ))
        });
    }

    /// 新建音轨对话框：AR 走带面板「+」按钮把 OPEN_REQUEST_ID 写进 ctx memory
    /// 触发，这里每帧检测并弹独立 viewport。确认后批量创建音轨并 teardown
    /// 音频引擎（音轨结构变化 → ChannelLayout 需按新 model 重建，
    /// 同 arrange.rs 里 add_track 的方案 A，下一帧 rebuild_audio_if_needed 重建）。
    fn show_new_track_dialog(&mut self, ctx: &egui::Context) {
        let open_req = ctx.data_mut(|d| {
            let id = egui::Id::new(crate::dialogs::new_track::OPEN_REQUEST_ID);
            let v = d.get_temp::<bool>(id).unwrap_or(false);
            d.remove::<bool>(id);
            v
        });
        if open_req {
            self.new_track_dialog.open();
            // 弹窗已存在但失焦时，再次点「+」需显式拉回前台
            crate::chrome::dialog::raise_viewport(
                ctx,
                egui::ViewportId::from_hash_of("new_track_dialog"),
            );
        }
        if !self.new_track_dialog.open {
            return;
        }
        // 没有活动文档时不应被触发（「+」按钮在有文档时才渲染），防御性关闭。
        let Some(doc_idx) = self.workspace.active_doc else {
            self.new_track_dialog.open = false;
            return;
        };

        let mut created = false;
        if let Some(doc) = self.workspace.documents.get_mut(doc_idx) {
            let action = crate::dialogs::new_track::show_viewport(
                ctx,
                &mut self.new_track_dialog,
                &doc.data.model.tracks,
            );
            match action {
                crate::dialogs::new_track::NewTrackAction::Confirm(specs) => {
                    let before = doc.capture_snapshot();
                    if let Some(a) = doc.add_tracks_batch(&specs) {
                        doc.push_undo(a, rust_i18n::t!("undo.add_track").as_ref(), before);
                        created = true;
                    }
                    self.new_track_dialog.open = false;
                }
                crate::dialogs::new_track::NewTrackAction::Cancel => {
                    self.new_track_dialog.open = false;
                }
                crate::dialogs::new_track::NewTrackAction::None => {}
            }
        }
        // teardown 借 &mut self，必须在 doc 借用的作用域外调用。
        if created {
            self.teardown_audio();
        }
    }

    /// 打开敲击测速对话框（传输栏播放菜单 / macOS 原生菜单共用）。
    pub(in crate::app) fn open_tap_tempo_dialog(&mut self, ctx: &egui::Context) {
        self.tap_tempo_dialog.open();
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("tap_tempo_dialog"),
        );
    }

    /// 「添加自动化」窗口：按该轨的乐器设备（XSynth / 插件）收集条目并打开。
    ///
    /// 自动化属于设备：XSynth 设备列内置参数（CC/PB/RPN + 自定义 CC）；
    /// 插件设备列插件参数（数千个走窗口虚拟滚动）。
    pub(in crate::app) fn open_automation_picker(&mut self, ctx: &egui::Context, track_idx: usize) {
        use crate::dialogs::automation_picker::AutomationEntry;
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let device_instrument = {
            let doc = &self.workspace.documents[idx];
            crate::arrange::plugin_instrument_of(&doc.data.model.tracks, &doc.mixer, track_idx)
        };
        let (channel_label, device_name, entries, show_custom_cc) = {
            let doc = &self.workspace.documents[idx];
            let Some(track) = doc.data.model.tracks.get(track_idx) else {
                return;
            };
            let existing: Vec<yinhe_types::AutomationTarget> = track
                .automation_lanes
                .iter()
                .map(|l| l.target.clone())
                .collect();
            match device_instrument {
                Some(ich) => {
                    // 插件设备：实例可用才有参数（未加载时空列表，窗口提示）。
                    let (name, params) = match self
                        .instrument_racks
                        .get_mut(idx)
                        .and_then(|rack| rack.instance_mut(ich))
                    {
                        Some(instance) => (instance.name().to_string(), instance.param_list()),
                        None => (String::new(), Vec::new()),
                    };
                    let entries = params
                        .into_iter()
                        .map(|p| AutomationEntry {
                            existing: existing.iter().any(|t| {
                                matches!(
                                    t,
                                    yinhe_types::AutomationTarget::PluginParam {
                                        channel,
                                        param_id,
                                        ..
                                    } if *channel == ich && *param_id == p.id
                                )
                            }),
                            label: if p.module.is_empty() {
                                p.name.clone()
                            } else {
                                format!("{}/{}", p.module, p.name)
                            },
                            target: yinhe_types::AutomationTarget::PluginParam {
                                channel: ich,
                                param_id: p.id,
                                name: p.name,
                            },
                        })
                        .collect();
                    (crate::mix::channel_label(ich), name, entries, false)
                }
                None => {
                    // XSynth 设备：内置参数（跳过 Tempo，那是工程级）。
                    let entries = crate::piano_view::automation_panel::AUTOMATION_TARGETS
                        .iter()
                        .filter(|t| !matches!(t, yinhe_types::AutomationTarget::Tempo))
                        .map(|t| AutomationEntry {
                            target: t.clone(),
                            label: crate::arrange::lane_label(t),
                            existing: existing.contains(t),
                        })
                        .collect();
                    (
                        crate::mix::channel_label(track.global_channel()),
                        "XSynth".to_string(),
                        entries,
                        true,
                    )
                }
            }
        };
        self.automation_picker.open(
            track_idx,
            channel_label,
            device_name,
            entries,
            show_custom_cc,
        );
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("automation_picker"),
        );
    }

    /// 「添加自动化」窗口：每帧渲染；点条目添加/移除 AM lane。
    fn show_automation_picker(&mut self, ctx: &egui::Context) {
        if !self.automation_picker.open {
            return;
        }
        if self.automation_picker.just_opened {
            self.automation_picker.just_opened = false;
            crate::chrome::dialog::raise_viewport(
                ctx,
                egui::ViewportId::from_hash_of("automation_picker"),
            );
        }
        use crate::dialogs::automation_picker::AutomationPickerAction as A;
        match crate::dialogs::automation_picker::show_viewport(ctx, &mut self.automation_picker) {
            A::None => {}
            A::Toggle {
                track_idx,
                target,
                add,
            } => {
                if let Some(idx) = self.workspace.active_doc {
                    let now = self.apply_automation_toggle(idx, track_idx, &target, add);
                    self.automation_picker.set_existing(&target, now);
                }
            }
            A::Close => self.automation_picker.open = false,
        }
    }

    /// 敲击测速对话框：每帧渲染，用户关闭窗口后清状态。
    fn show_tap_tempo_dialog(&mut self, ctx: &egui::Context) {
        if !self.tap_tempo_dialog.open {
            return;
        }
        if crate::dialogs::tap_tempo::show_viewport(ctx, &mut self.tap_tempo_dialog) {
            self.tap_tempo_dialog.open = false;
        }
    }

    /// 选择筛选对话框：每帧渲染；应用/清除/取消后同步选择状态。
    fn show_filter_dialog(&mut self, ctx: &egui::Context) {
        if !self.filter_dialog.open {
            return;
        }
        if self.filter_dialog.just_opened {
            self.filter_dialog.just_opened = false;
            crate::chrome::dialog::raise_viewport(
                ctx,
                egui::ViewportId::from_hash_of("selection_filter_dialog"),
            );
        }
        let num_tracks = self
            .workspace
            .active_doc
            .and_then(|i| self.workspace.documents.get(i))
            .map(|d| d.data.model.tracks.len())
            .unwrap_or(0);
        use crate::dialogs::filter::FilterDialogAction as A;
        match crate::dialogs::filter::show_viewport(ctx, &mut self.filter_dialog, num_tracks) {
            A::None => {}
            A::Apply => self.apply_filter_dialog(),
            A::Confirm => {
                self.apply_filter_dialog();
                self.filter_dialog.open = false;
            }
            A::Clear => {
                self.clear_filter();
                // 弹窗输入状态同步复位（从清空后的 filter 重新初始化）
                self.open_filter_dialog();
            }
            A::Cancel => self.filter_dialog.open = false,
        }
    }

    /// 渲染音轨属性 / 工程设置浮动面板（独立视口子窗口）。
    ///
    /// 音轨属性与右侧栏 Info 内容互斥：弹窗打开时右侧栏已收起（set_float_panel
    /// 关闭），用户点 X 只关闭弹窗，点「停靠到侧栏」则把内容搬回右侧栏 Info tab。
    /// 工程设置只在浮窗显示（无停靠入口），点 X 即关闭。
    fn show_float_panels(&mut self, ctx: &egui::Context) {
        // XSynth 配置窗口（独立于 FloatPanel：按源通道打开）。
        if crate::dialogs::xsynth_config::show_viewport(self, ctx) {
            self.teardown_audio();
        }
        let Some(panel) = self.float_panel else {
            return;
        };
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let audio = self.audio_state.handle.as_ref();
        let mut open = true;
        let mut dock = false;
        let mut port_changed = false;

        use crate::right_panel::FloatPanel;
        match panel {
            FloatPanel::TrackProps { track_idx } => {
                port_changed = crate::dialogs::prop_panels::show_track_props_viewport(
                    ctx, doc, audio, &mut open, track_idx, &mut dock,
                );
            }
            FloatPanel::ProjectSettings => {
                crate::dialogs::prop_panels::show_project_settings_viewport(ctx, doc, &mut open);
            }
        }

        if dock {
            self.dock_float_panel(panel);
        } else if !open {
            // 用户点 X：只关闭弹窗，侧栏保持原状。
            self.float_panel = None;
        }
        if port_changed {
            self.teardown_audio();
        }
    }
}

/// 影响音频引擎 spawn 的设置字段快照。
///
/// 用于区分「设置真的改了」与「只是关掉了设置窗口」：`rebuild_audio_if_needed`
/// 只读取这些字段（采样率 / 缓冲大小 / 输出设备 / GPU 合成开关 / 全局音色库）
/// 作为 spawn 输入。`xsynth_layers` 由 `SetLayerCount` 在线应用，不需要重建。
struct EngineSettingsKey {
    sample_rate: u32,
    buffer_size: u32,
    output_device_name: Option<String>,
    use_gpu_synth: bool,
    sf_entries: Vec<(String, String, bool)>,
}

impl EngineSettingsKey {
    fn of(settings: &crate::audio_settings::AudioSettings) -> Self {
        Self {
            sample_rate: settings.sample_rate,
            buffer_size: settings.buffer_size,
            output_device_name: settings.output_device_name.clone(),
            use_gpu_synth: settings.use_gpu_synth,
            sf_entries: settings
                .global_sf_config
                .entries
                .iter()
                .map(|e| (e.path.clone(), e.name.clone(), e.enabled))
                .collect(),
        }
    }

    fn differs(&self, other: &Self) -> bool {
        self.sample_rate != other.sample_rate
            || self.buffer_size != other.buffer_size
            || self.output_device_name != other.output_device_name
            || self.use_gpu_synth != other.use_gpu_synth
            || self.sf_entries != other.sf_entries
    }
}

#[cfg(test)]
mod engine_settings_key_tests {
    use super::EngineSettingsKey;

    fn base() -> EngineSettingsKey {
        EngineSettingsKey {
            sample_rate: 48000,
            buffer_size: 0,
            output_device_name: Some("A".into()),
            use_gpu_synth: true,
            sf_entries: vec![("/p.sfz".into(), "Piano".into(), true)],
        }
    }

    #[test]
    fn identical_settings_do_not_diff() {
        assert!(!base().differs(&base()));
    }

    #[test]
    fn engine_relevant_changes_are_detected() {
        let mut sr = base();
        sr.sample_rate = 44100;
        assert!(base().differs(&sr));

        let mut buf = base();
        buf.buffer_size = 256;
        assert!(base().differs(&buf));

        let mut dev = base();
        dev.output_device_name = None;
        assert!(base().differs(&dev));

        let mut gpu = base();
        gpu.use_gpu_synth = false;
        assert!(base().differs(&gpu));

        let mut sf = base();
        sf.sf_entries[0].2 = false;
        assert!(base().differs(&sf));

        let mut sf_add = base();
        sf_add
            .sf_entries
            .push(("/q.sfz".into(), "Strings".into(), true));
        assert!(base().differs(&sf_add));
    }
}
