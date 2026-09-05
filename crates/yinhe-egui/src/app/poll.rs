use crate::app::App;
use crate::file_loader::LoadResult;
use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

impl App {
    /// Poll all async operations: file loading, save completion, export completion.
    pub(in crate::app) fn poll_async_operations(&mut self) {
        // Poll async file loading
        match self.file_loader.poll_loading() {
            LoadResult::ModelLoaded {
                path,
                archive_path,
                model,
            } => {
                let (quantize_arrange, quantize_pianoroll) = self
                    .workspace
                    .active_doc
                    .and_then(|idx| self.workspace.documents.get(idx))
                    .map(|doc| (doc.edit.quantize_arrange, doc.edit.quantize_pianoroll))
                    .unwrap_or((
                        QuantizePreset::Fraction(1, 4),
                        QuantizePreset::Fraction(1, 16),
                    ));
                match Document::from_model(
                    &path,
                    model,
                    quantize_arrange,
                    quantize_pianoroll,
                    yinhe_yin::ProjectFile::default(),
                    yinhe_yin::MappingFile::default(),
                    None,
                ) {
                    Ok(mut doc) => {
                        doc.sync_track_caches_with_conductor_color(
                            crate::theme::conductor_color_f32(),
                        );
                        doc.mark_loaded(); // Loaded from file, not a fresh empty doc
                        // 重叠开关是全局设置：打开工程沿用当前持久化值。
                        doc.edit.allow_overlapping_notes =
                            self.audio_settings.allow_overlapping_notes;
                        doc.edit.overlap_blocked_behavior =
                            self.audio_settings.overlap_blocked_behavior;
                        // 仅首次启动的 Untitled（未修改且无 file_path）被替换，
                        // 避免另开一个空标签页；用户手动 NewProject 或已修改/已保存
                        // 的工程保持不动，照常 push 新标签页。
                        if self.should_replace_initial_untitled() {
                            self.workspace.documents[0] = doc;
                            self.workspace.active_doc = Some(0);
                            self.restore_mixer_rack(0);
                            self.restore_instrument_rack(0);
                        } else {
                            let insert_idx = self.workspace.documents.len();
                            self.workspace.documents.push(doc);
                            self.workspace.active_doc = Some(insert_idx);
                            self.restore_mixer_rack(insert_idx);
                            self.restore_instrument_rack(insert_idx);
                        }
                        self.teardown_audio();
                        // 替换路径下 active_doc 不变，main_loop 的 switch 检测不到，
                        // 必须主动清空 cull，否则旧 Untitled 的空 buffer 状态会让
                        // 新工程首帧走错路径。
                        self.invalidate_cull_state();
                        // 打开成功 → 记录到「最近修改的文件」
                        // 压缩包内文件记录外层压缩包路径，避免内部 entry 名（相对路径）导致「找不到文件」
                        let recent = archive_path.as_deref().unwrap_or(&path);
                        if self.audio_settings.push_recent_file(recent) {
                            self.audio_settings.save();
                        }
                        let fname = std::path::Path::new(&path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&path);
                        if self
                            .notifications
                            .has_progress(crate::widgets::toast::LOADING_PROGRESS_ID)
                            && !self
                                .notifications
                                .is_leaving(crate::widgets::toast::LOADING_PROGRESS_ID)
                        {
                            self.notifications.complete_progress(
                                crate::widgets::toast::LOADING_PROGRESS_ID,
                                crate::widgets::toast::ToastKind::Success,
                                "MIDI加载完成",
                                fname.to_string(),
                            );
                            // 进度条已满隐藏，label 改为加载耗时
                            if let Some(d) = self.file_loader.load_elapsed() {
                                self.notifications.update_progress(
                                    crate::widgets::toast::LOADING_PROGRESS_ID,
                                    1.0,
                                    format_load_duration(d),
                                );
                            }
                        } else {
                            self.notifications
                                .prune_history(crate::widgets::toast::LOADING_PROGRESS_ID);
                            self.notifications
                                .success("MIDI加载完成", fname.to_string());
                        }
                    }
                    Err(msg) => {
                        self.load_error = Some(msg.clone());
                        if self
                            .notifications
                            .has_progress(crate::widgets::toast::LOADING_PROGRESS_ID)
                            && !self
                                .notifications
                                .is_leaving(crate::widgets::toast::LOADING_PROGRESS_ID)
                        {
                            self.notifications.fail_progress(
                                crate::widgets::toast::LOADING_PROGRESS_ID,
                                "打开失败",
                                msg.clone(),
                            );
                        } else {
                            self.notifications
                                .prune_history(crate::widgets::toast::LOADING_PROGRESS_ID);
                            self.notifications.error("打开失败", msg);
                        }
                    }
                }
            }
            LoadResult::ModelFromYin {
                path,
                model,
                file_name,
                sf,
                mapping,
                mixer,
            } => {
                let (quantize_arrange, quantize_pianoroll) = self
                    .workspace
                    .active_doc
                    .and_then(|idx| self.workspace.documents.get(idx))
                    .map(|doc| (doc.edit.quantize_arrange, doc.edit.quantize_pianoroll))
                    .unwrap_or((
                        QuantizePreset::Fraction(1, 4),
                        QuantizePreset::Fraction(1, 16),
                    ));
                let project_file = yinhe_yin::ProjectFile::from_meta_with_sf(
                    &model.meta,
                    sf.mode,
                    sf.overrides.clone(),
                );
                let result = Document::from_model(
                    &path,
                    model,
                    quantize_arrange,
                    quantize_pianoroll,
                    project_file,
                    mapping,
                    mixer,
                )
                .ok()
                .map(|mut d| {
                    d.sync_track_caches_with_conductor_color(crate::theme::conductor_color_f32());
                    // 拖出新窗口产生的 temp 工程：不绑定 file_path，视为未保存
                    // （Cmd+S 弹另存为、关窗弹确认），避免用户把编辑静默写进 /tmp。
                    if is_detached_temp_path(&path) {
                        d.file_path = None;
                    } else {
                        d.file_path = Some(path.clone());
                    }
                    d.mark_loaded(); // Loaded from file, not a fresh empty doc
                    d.edit.allow_overlapping_notes = self.audio_settings.allow_overlapping_notes;
                    d.edit.overlap_blocked_behavior = self.audio_settings.overlap_blocked_behavior;

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
                    if self.should_replace_initial_untitled() {
                        self.workspace.documents[0] = doc;
                        self.workspace.active_doc = Some(0);
                        self.restore_mixer_rack(0);
                        self.restore_instrument_rack(0);
                    } else {
                        let insert_idx = self.workspace.documents.len();
                        self.workspace.documents.push(doc);
                        self.workspace.active_doc = Some(insert_idx);
                        self.restore_mixer_rack(insert_idx);
                        self.restore_instrument_rack(insert_idx);
                    }
                    self.teardown_audio();
                    self.invalidate_cull_state();
                    // 打开成功 → 记录到「最近修改的文件」
                    // 拖出 temp 不进「最近打开」（路径在 /tmp 下，重开无意义）
                    if !is_detached_temp_path(&path) && self.audio_settings.push_recent_file(&path)
                    {
                        self.audio_settings.save();
                    }
                    if self
                        .notifications
                        .has_progress(crate::widgets::toast::LOADING_PROGRESS_ID)
                        && !self
                            .notifications
                            .is_leaving(crate::widgets::toast::LOADING_PROGRESS_ID)
                    {
                        self.notifications.complete_progress(
                            crate::widgets::toast::LOADING_PROGRESS_ID,
                            crate::widgets::toast::ToastKind::Success,
                            "MIDI加载完成",
                            file_name.clone(),
                        );
                        // 进度条已满隐藏，label 改为加载耗时
                        if let Some(d) = self.file_loader.load_elapsed() {
                            self.notifications.update_progress(
                                crate::widgets::toast::LOADING_PROGRESS_ID,
                                1.0,
                                format_load_duration(d),
                            );
                        }
                    } else {
                        self.notifications
                            .prune_history(crate::widgets::toast::LOADING_PROGRESS_ID);
                        self.notifications
                            .success("MIDI加载完成", file_name.clone());
                    }
                } else {
                    let msg = t!("file_dialog.open_failed", name = file_name).to_string();
                    self.load_error = Some(msg.clone());
                    if self
                        .notifications
                        .has_progress(crate::widgets::toast::LOADING_PROGRESS_ID)
                        && !self
                            .notifications
                            .is_leaving(crate::widgets::toast::LOADING_PROGRESS_ID)
                    {
                        self.notifications.fail_progress(
                            crate::widgets::toast::LOADING_PROGRESS_ID,
                            "打开失败",
                            msg.clone(),
                        );
                    } else {
                        self.notifications
                            .prune_history(crate::widgets::toast::LOADING_PROGRESS_ID);
                        self.notifications.error("打开失败", msg);
                    }
                }
            }
            LoadResult::ArchiveError(msg) => {
                self.load_error = Some(msg.clone());
                if self
                    .notifications
                    .has_progress(crate::widgets::toast::LOADING_PROGRESS_ID)
                    && !self
                        .notifications
                        .is_leaving(crate::widgets::toast::LOADING_PROGRESS_ID)
                {
                    self.notifications.fail_progress(
                        crate::widgets::toast::LOADING_PROGRESS_ID,
                        "打开失败",
                        msg.clone(),
                    );
                } else {
                    self.notifications
                        .prune_history(crate::widgets::toast::LOADING_PROGRESS_ID);
                    self.notifications.error("打开失败", msg);
                }
            }
            // UI 层 poll_loading 已把这两个变体转换为弹框状态（返回 NotReady），
            // 这里实际不会收到，空分支只是保证 match 穷尽。
            LoadResult::ArchivePickerNeeded { .. } | LoadResult::PasswordNeeded { .. } => {}
            LoadResult::NotReady => {}
        }

        // Poll async save completion
        if let Some(rx) = &self.save_rx
            && rx.try_recv().is_ok()
        {
            self.save_rx = None;
            self.save_progress_rx = None;
            if let Ok(mut s) = self.save_progress.lock() {
                *s = None;
            }
            // Mark the active document as saved
            let saved_name = if let Some(idx) = self.workspace.active_doc {
                self.workspace.documents[idx].mark_saved();
                // 保存成功 → 记录到「最近修改的文件」
                if let Some(path) = self.workspace.documents[idx].file_path.clone()
                    && self.audio_settings.push_recent_file(&path)
                {
                    self.audio_settings.save();
                }
                Some(self.workspace.documents[idx].file_name.clone())
            } else {
                None
            };
            let (kind, title) = (crate::widgets::toast::ToastKind::Success, "已完成");
            if self
                .notifications
                .has_progress(crate::widgets::toast::SAVE_PROGRESS_ID)
                && !self
                    .notifications
                    .is_leaving(crate::widgets::toast::SAVE_PROGRESS_ID)
            {
                if let Some(name) = saved_name.clone() {
                    self.notifications.complete_progress(
                        crate::widgets::toast::SAVE_PROGRESS_ID,
                        kind,
                        title,
                        name,
                    );
                } else {
                    self.notifications.complete_progress(
                        crate::widgets::toast::SAVE_PROGRESS_ID,
                        kind,
                        title,
                        "",
                    );
                }
            } else if let Some(name) = saved_name {
                self.notifications
                    .prune_history(crate::widgets::toast::SAVE_PROGRESS_ID);
                self.notifications.success("已保存", name);
            } else {
                self.notifications
                    .prune_history(crate::widgets::toast::SAVE_PROGRESS_ID);
                self.notifications.success("已保存", "");
            }
            // If there's a deferred action, execute it now
            if self.pending_unsaved.is_some() {
                let ctx = egui::Context::default();
                self.execute_pending_file_action(&ctx);
            }
        } else if let Some(rx) = &self.save_progress_rx {
            while let Ok(p) = rx.try_recv() {
                if let Ok(mut s) = self.save_progress.lock() {
                    *s = Some((p.stage, p.fraction));
                }
            }
        }

        // Poll async export completion
        if let Some(rx) = &self.export.rx
            && let Ok(result) = rx.try_recv()
        {
            self.export.rx = None;
            match result {
                Ok((path, elapsed, speed)) => {
                    let fname = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&path)
                        .to_string();
                    self.export.completed = Some(crate::dialogs::export::ExportCompleted {
                        file_path: path.clone(),
                        elapsed_secs: elapsed,
                        overall_speed: speed,
                    });
                    if self
                        .notifications
                        .has_progress(crate::widgets::toast::EXPORT_PROGRESS_ID)
                        && !self
                            .notifications
                            .is_leaving(crate::widgets::toast::EXPORT_PROGRESS_ID)
                    {
                        let acted = self.notifications.complete_progress(
                            crate::widgets::toast::EXPORT_PROGRESS_ID,
                            crate::widgets::toast::ToastKind::Success,
                            "已完成",
                            format!("{} ({:.1}s, {:.1}x)", fname, elapsed, speed),
                        );
                        // 可操作卡：打开文件夹（计时自动升为可操作档）
                        self.notifications.set_action(
                            acted,
                            "打开文件夹",
                            crate::widgets::toast::model::ToastActionKind::RevealInFolder(
                                std::path::PathBuf::from(&path),
                            ),
                        );
                    } else {
                        self.notifications
                            .prune_history(crate::widgets::toast::EXPORT_PROGRESS_ID);
                        let nid = self.notifications.success(
                            "导出完成",
                            format!("{} ({:.1}s, {:.1}x)", fname, elapsed, speed),
                        );
                        self.notifications.set_action(
                            nid,
                            "打开文件夹",
                            crate::widgets::toast::model::ToastActionKind::RevealInFolder(
                                std::path::PathBuf::from(&path),
                            ),
                        );
                    }
                }
                Err(e) => {
                    self.load_error = Some(e.clone());
                    if self
                        .notifications
                        .has_progress(crate::widgets::toast::EXPORT_PROGRESS_ID)
                        && !self
                            .notifications
                            .is_leaving(crate::widgets::toast::EXPORT_PROGRESS_ID)
                    {
                        self.notifications.fail_progress(
                            crate::widgets::toast::EXPORT_PROGRESS_ID,
                            "导出失败",
                            e.clone(),
                        );
                    } else {
                        self.notifications
                            .prune_history(crate::widgets::toast::EXPORT_PROGRESS_ID);
                        self.notifications.error("导出失败", e);
                    }
                }
            }
        }

        // Poll async PPQ rescale completion
        self.poll_rescale_completion();
    }

    /// Sync `automation_event_density` to the audio engine when it changes.
    pub(in crate::app) fn sync_automation_density(&mut self) {
        let density = self.audio_settings.automation_event_density;
        if density != self.last_automation_density {
            self.last_automation_density = density;
            if let Some(audio) = &self.audio_state.handle {
                audio
                    .handle
                    .send(yinhe_audio::AudioCommand::SetAutomationDensity { density });
            }
        }
    }

    /// Refresh system resource monitoring (CPU, memory) if enough time has elapsed.
    pub(crate) fn refresh_system_stats(&mut self) {
        self.sys_monitor.refresh_if_needed();
    }
}

/// 加载耗时文案：不足一分钟省略“分”，不足一秒省略“秒”。
/// 例：2分15秒321毫秒 / 15秒321毫秒 / 321毫秒。
pub(crate) fn format_load_duration(d: std::time::Duration) -> String {
    let total_ms = d.as_millis();
    let mins = total_ms / 60_000;
    let secs = (total_ms % 60_000) / 1_000;
    let ms = total_ms % 1_000;
    if mins > 0 {
        format!("加载时间：{}分{}秒{}毫秒", mins, secs, ms)
    } else if secs > 0 {
        format!("加载时间：{}秒{}毫秒", secs, ms)
    } else {
        format!("加载时间：{}毫秒", ms)
    }
}

/// 判断路径是否为「拖出新窗口」产生的 temp 工程
/// （命名约定见 `App::detach_tab_to_new_window`；uuid 后缀保证不会撞名）。
fn is_detached_temp_path(path: &str) -> bool {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("yinhe-detached-") && n.ends_with(".yin"))
}

#[cfg(test)]
mod tests {
    use super::format_load_duration;

    #[test]
    fn load_duration_omits_leading_units() {
        assert_eq!(
            format_load_duration(std::time::Duration::from_millis(135_321)),
            "加载时间：2分15秒321毫秒"
        );
        assert_eq!(
            format_load_duration(std::time::Duration::from_millis(15_321)),
            "加载时间：15秒321毫秒"
        );
        assert_eq!(
            format_load_duration(std::time::Duration::from_millis(321)),
            "加载时间：321毫秒"
        );
        assert_eq!(
            format_load_duration(std::time::Duration::from_millis(0)),
            "加载时间：0毫秒"
        );
    }
}
