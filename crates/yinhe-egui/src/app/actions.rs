use std::sync::mpsc;

use eframe::egui;
use rust_i18n::t;

use crate::app::{App, PendingFileAction};
use crate::chrome::transport_bar;
use crate::chrome::transport_bar::FileAction;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::shortcuts;

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
    /// 工具切换快捷键触发的目标工具（None = 本帧未触发）。
    pub tool_to_activate: Option<crate::widgets::tools_panel::Tool>,
    /// 文件菜单动作（非 macOS 平台由键盘触发；macOS 走原生菜单栏）。
    pub file_action: Option<FileAction>,
}

impl App {
    /// Handle keyboard shortcuts.
    /// Returns a `KeyboardActions` struct describing which actions were triggered.
    pub(crate) fn handle_keyboard_shortcuts(&self, ui: &egui::Ui) -> KeyboardActions {
        let mut actions = KeyboardActions::default();

        // 文本输入焦点（TextEdit/DragValue 等）优先：全局快捷键让位给输入框，
        // 与成熟 DAW 一致（Backspace/Delete/Cmd+C/V/Z 等作用于文本而非选区）。
        // 设置窗口打开或快捷键录制期间同样让位：设置页里不允许任何快捷键触发动作
        // （Esc 例外：由设置页录制器消费用于取消录制）。
        if ui.ctx().egui_wants_keyboard_input()
            || self.audio_settings.show_settings
            || self.audio_settings.shortcut_recording
        {
            return actions;
        }

        let is_playing_any = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);

        // 本帧唯一一次主键按下（排除纯修饰键）。
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if crate::shortcuts::is_recordable_key(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });

        let kb = &self.audio_settings.keybindings;
        // 一个动作可绑定多个快捷键，任一匹配即触发。
        // macOS：第一个快捷键若带修饰键（⌘/⇧/⌥），由原生菜单加速键在系统层面
        // 处理（AppKit 拦截，egui 收不到那个键），这里跳过它避免双触发；
        // 无修饰键的快捷键（如 Space）AppKit 不会拦截菜单加速键，必须由 egui
        // 处理，否则按键会失效。其余快捷键（第二个及以后）同样由 egui 处理。
        let matches = |id: &str, key: egui::Key, modifiers: egui::Modifiers| {
            kb.get(id).iter().enumerate().any(|(i, c)| {
                #[cfg(target_os = "macos")]
                {
                    if i == 0 && crate::shortcuts::native_menu_handles(c) {
                        return false;
                    }
                }
                crate::shortcuts::matches_combo(c, modifiers, key)
            })
        };

        if let Some((key, modifiers)) = pressed {
            // ── 文件动作 ──
            // macOS：第一个快捷键走原生菜单；其余快捷键在这里分发。
            for action in FileAction::ALL {
                if matches(action.action_id(), key, modifiers) {
                    actions.file_action = Some(action);
                    break;
                }
            }

            // ── 播放/停止 ──
            if matches(shortcuts::ACTION_TOGGLE_PLAY, key, modifiers) {
                if is_playing_any {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            if matches(shortcuts::ACTION_STOP, key, modifiers) {
                actions.stop_play = true;
            }

            // ── 编辑 ──
            if matches(shortcuts::ACTION_DELETE, key, modifiers) {
                actions.delete_selected = true;
            }
            if matches(shortcuts::ACTION_DUPLICATE, key, modifiers) {
                actions.duplicate_selected = true;
            }
            if matches(shortcuts::ACTION_TRANSPOSE_UP, key, modifiers) {
                actions.transpose_up = true;
            }
            if matches(shortcuts::ACTION_TRANSPOSE_DOWN, key, modifiers) {
                actions.transpose_down = true;
            }
            if matches(shortcuts::ACTION_UNDO, key, modifiers) {
                actions.undo = true;
            }
            if matches(shortcuts::ACTION_REDO, key, modifiers) {
                actions.redo = true;
            }
            if matches(shortcuts::ACTION_CUT, key, modifiers) {
                actions.cut = true;
            }
            if matches(shortcuts::ACTION_COPY, key, modifiers) {
                actions.copy = true;
            }
            if matches(shortcuts::ACTION_PASTE, key, modifiers) {
                actions.paste = true;
            }
            if matches(shortcuts::ACTION_SELECT_ALL, key, modifiers) {
                actions.select_all = true;
            }

            // ── 工具切换 ──
            // 工具动作不在 macOS 原生菜单中，egui 一定能收到按键，
            // 因此这里不跳过第一个快捷键（与文件/编辑动作的 macOS 处理不同）。
            for tool in crate::widgets::tools_panel::ALL_TOOLS {
                if kb
                    .get(tool.action_id())
                    .iter()
                    .any(|c| crate::shortcuts::matches_combo(c, modifiers, key))
                {
                    actions.tool_to_activate = Some(tool);
                    break;
                }
            }
        }

        // ── 兼容别名（配置表之外的历史默认）──
        ui.input(|i| {
            // Backspace 等同 Delete
            if i.key_pressed(egui::Key::Backspace) {
                actions.delete_selected = true;
            }
            // Cmd/Ctrl+Y 也触发重做
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::Y) {
                actions.redo = true;
            }
        });

        actions
    }

    /// Delete all selected notes from the active document.
    pub(crate) fn delete_selected_notes(&mut self) {
        self.with_undo(t!("undo.delete_notes").as_ref(), |doc| {
            doc.delete_selected()
        });
    }

    /// Duplicate all selected notes (Ctrl+D / Cmd+D).
    /// New notes are placed after the original selection, offset by the selection duration.
    pub(crate) fn duplicate_selected_notes(&mut self) {
        self.with_undo(t!("undo.duplicate_notes").as_ref(), |doc| {
            doc.duplicate_selected()
        });
    }

    /// Transpose selected notes by `semitones` (e.g. +12 for up an octave, -12 for down).
    pub(crate) fn transpose_selected_notes(&mut self, semitones: i8) {
        let label = if semitones >= 0 {
            t!("undo.transpose_up")
        } else {
            t!("undo.transpose_down")
        };
        self.with_undo(label.as_ref(), |doc| doc.transpose_selected(semitones));
    }

    /// Flip selected notes horizontally (tick) or vertically (key).
    pub(crate) fn flip_selected_notes(&mut self, axis: yinhe_editor_core::FlipAxis) {
        let label = match axis {
            yinhe_editor_core::FlipAxis::Horizontal => t!("undo.flip_horizontal"),
            yinhe_editor_core::FlipAxis::Vertical => t!("undo.flip_vertical"),
        };
        self.with_undo(label.as_ref(), |doc| doc.flip_selected_notes(axis));
    }

    /// 一键为整首歌去重重叠音符（黑乐谱叠音清理）。
    pub(crate) fn dedup_overlapping_notes(&mut self, cross_track: bool) {
        let label = if cross_track {
            t!("undo.dedup_across_tracks")
        } else {
            t!("undo.dedup_within_track")
        };
        self.with_undo(label.as_ref(), |doc| {
            doc.dedup_overlapping_notes(cross_track)
        });
    }

    // ── Copy / Cut / Paste / Select All ──

    /// Copy selection rects to clipboard (no note data, just rects).
    /// Resets cut_past_len since a new copy invalidates the cut undo bridge.
    pub(crate) fn copy_selection(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        self.clipboard = self.workspace.documents[idx].edit.selected.clone();
        self.cut_past_len = None;
    }

    /// Cut: copy rects to clipboard, then delete selected notes.
    /// Stores the current undo stack length so paste can locate the
    /// correct undo entry (undo bridge) even if intervening edits occur.
    pub(crate) fn cut_selection(&mut self) {
        self.copy_selection();
        // cut_past_len is reset by copy_selection; set it before delete pushes.
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        self.cut_past_len = Some(self.workspace.documents[idx].history.past_len());
        self.delete_selected_notes();
    }

    /// Paste notes from clipboard at cursor position.
    pub(crate) fn paste_clipboard(&mut self) {
        let clipboard = self.clipboard.clone();
        let cut_past_len = self.cut_past_len;
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let cursor_tick = self.workspace.documents[idx]
            .edit
            .cursor_tick
            .unwrap_or(0.0);
        let track_selected = self.workspace.documents[idx].edit.track_selected.clone();
        self.with_undo(t!("undo.paste").as_ref(), |doc| {
            doc.paste_from_selection(&clipboard, cursor_tick, cut_past_len, &track_selected)
        });
    }

    /// Select all notes — PR or AR depending on current view mode.
    pub(crate) fn select_all(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let is_pr = self.view_mode == crate::chrome::mode_bar::ViewMode::Edit;
        if is_pr {
            self.workspace.documents[idx].select_all_pr();
        } else {
            // select_all_ar 内部会同步设置 doc.edit.arr_sel_rect（AR 选框）。
            self.workspace.documents[idx].select_all_ar();
        }
        self.workspace.documents[idx].data.bump_revision();
        self.pianoroll_view.base.dirty = true;
        self.arrange_view.base.dirty = true;
    }

    /// Add a single note to the given track and record an undo entry.
    pub(crate) fn add_note_with_undo(&mut self, track_idx: u16, note: yinhe_core::NoteEvent) {
        self.with_undo(t!("undo.add_note").as_ref(), |doc| {
            doc.add_note(track_idx, note)
        });
    }

    /// Run an edit closure, recording an undo entry from the returned action
    /// and notifying audio afterwards.
    ///
    /// The closure receives `&mut Document` and should return
    /// `Some(UndoAction)` if it actually changed anything; on `None` no
    /// undo entry is pushed and audio is not notified.
    pub(crate) fn with_undo<F>(&mut self, label: &str, f: F)
    where
        F: FnOnce(&mut Document) -> Option<yinhe_editor_core::history::UndoAction>,
    {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let before = self.workspace.documents[idx].capture_snapshot();
        let action = f(&mut self.workspace.documents[idx]);
        let Some(action) = action else { return };
        let doc = &mut self.workspace.documents[idx];
        doc.push_undo(action, label, before);
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
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc: &mut Document = &mut self.workspace.documents[idx];
        let changed = doc.undo();
        if changed {
            doc.data.bump_revision();
            self.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
        }
    }

    /// Re-apply the most recently undone state on the active document.
    pub(crate) fn redo(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc: &mut Document = &mut self.workspace.documents[idx];
        let changed = doc.redo();
        if changed {
            doc.data.bump_revision();
            self.pianoroll_view.base.dirty = true;
            self.notify_audio_model_changed();
        }
    }
}

impl App {
    /// 处理编辑动作（transport bar 编辑 popup / 图钉与 macOS 菜单共用）。
    /// 复制/粘贴/复制/删除在自动化锚点选中时作用于锚点（与键盘快捷键一致）。
    pub(crate) fn handle_edit_action(&mut self, action: transport_bar::EditAction) {
        use transport_bar::EditAction as A;
        let route_to_automation = self.has_selected_automation_anchors();
        match action {
            A::Undo => self.undo(),
            A::Redo => self.redo(),
            A::Cut => self.cut_selection(),
            A::Copy => {
                if route_to_automation {
                    self.copy_automation_anchors();
                } else {
                    self.copy_selection();
                }
            }
            A::Paste => {
                if route_to_automation {
                    self.paste_automation_anchors();
                } else {
                    self.paste_clipboard();
                }
            }
            A::SelectAll => self.select_all(),
            A::Duplicate => {
                if route_to_automation {
                    self.duplicate_automation_anchors();
                } else {
                    self.duplicate_selected_notes();
                }
            }
            A::Delete => {
                if route_to_automation {
                    self.delete_automation_anchors();
                } else {
                    self.delete_selected_notes();
                }
            }
            A::TransposeUp => self.transpose_selected_notes(12),
            A::TransposeDown => self.transpose_selected_notes(-12),
            A::DedupWithinTrack => self.dedup_overlapping_notes(false),
            A::DedupAcrossTracks => self.dedup_overlapping_notes(true),
        }
    }

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
            | transport_bar::FileAction::ProjectSettings
            | transport_bar::FileAction::Open => {
                self.execute_file_action(action, ctx);
                return;
            }
            _ => {}
        }

        // Check for unsaved changes
        if let Some(idx) = self.workspace.active_doc
            && self.workspace.documents[idx].is_dirty()
        {
            let pending = match action {
                transport_bar::FileAction::NewProject => PendingFileAction::NewProject,
                transport_bar::FileAction::Open => PendingFileAction::Open,
                transport_bar::FileAction::CloseDocument => PendingFileAction::CloseDocument(idx),
                transport_bar::FileAction::Exit => PendingFileAction::Exit,
                _ => unreachable!(), // filtered above
            };
            self.pending_unsaved = Some(pending);
            // 用户通过菜单/快捷键主动触发需要决策的操作，立刻把 unsaved 弹窗
            // 拉到主窗口前台（防止之前取消后弹窗被遮挡在主窗口后方）
            crate::chrome::dialog::raise_viewport(
                ctx,
                egui::ViewportId::from_hash_of("unsaved_dialog"),
            );
            return;
        }

        self.execute_file_action(action, ctx);
    }

    /// 打开「最近修改的文件」（transport bar 子菜单 / macOS 菜单栏共用）。
    /// 文件已被移动/删除时从列表移除并报错；有未保存修改时先走确认流程。
    pub(crate) fn open_recent_file(&mut self, path: &str, ctx: &egui::Context) {
        if !std::path::Path::new(path).exists() {
            self.audio_settings.remove_recent_file(path);
            self.audio_settings.save();
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            self.load_error = Some(t!("file_dialog.open_failed", name = name).to_string());
            return;
        }
        if let Some(idx) = self.workspace.active_doc
            && self.workspace.documents[idx].is_dirty()
        {
            self.pending_unsaved = Some(PendingFileAction::OpenRecent(path.to_string()));
            // 与 handle_file_action 一致：主动触发的操作立刻把 unsaved 弹窗拉到前台
            crate::chrome::dialog::raise_viewport(
                ctx,
                egui::ViewportId::from_hash_of("unsaved_dialog"),
            );
            return;
        }
        self.file_loader
            .load_path(path.to_string(), self.audio_settings.midi_import_encoding);
    }

    /// Execute a file action immediately without checking for unsaved changes.
    fn execute_file_action(&mut self, action: transport_bar::FileAction, ctx: &egui::Context) {
        match action {
            transport_bar::FileAction::NewProject => {
                self.new_project();
            }
            transport_bar::FileAction::Open => {
                self.file_loader
                    .pick_file(self.audio_settings.midi_import_encoding);
            }
            transport_bar::FileAction::Save => {
                if let Some(idx) = self.workspace.active_doc {
                    let path = self.workspace.documents[idx].file_path.clone();
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
                if let Some(idx) = self.workspace.active_doc {
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
            transport_bar::FileAction::ProjectSettings => {
                self.set_float_panel(Some(crate::right_panel::FloatPanel::ProjectSettings));
            }
        }
    }

    /// Execute the deferred pending action (called after save completes or on discard).
    pub(crate) fn execute_pending_file_action(&mut self, _ctx: &egui::Context) {
        let Some(pending) = self.pending_unsaved.take() else {
            return;
        };
        match pending {
            PendingFileAction::NewProject => {
                self.new_project();
            }
            PendingFileAction::Open => {
                self.file_loader
                    .pick_file(self.audio_settings.midi_import_encoding);
            }
            PendingFileAction::OpenRecent(path) => {
                self.file_loader
                    .load_path(path, self.audio_settings.midi_import_encoding);
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
        let doc = &mut self.workspace.documents[idx];
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

        // 混音台插件：保存前把实例的 CLAP state 写回 InsertRef（旁通标志同步）。
        if let Some(rack) = self.mixer_racks.get_mut(idx) {
            rack.sync_states_to(&mut self.workspace.documents[idx].mixer);
        }
        // 乐器插件：同样把实例 state 写回 mixer.instruments[channel]。
        if let Some(irack) = self.instrument_racks.get_mut(idx) {
            irack.sync_states_to(&mut self.workspace.documents[idx].mixer);
        }
        let doc = &self.workspace.documents[idx];
        let model = doc.data.model.clone();
        let project_file = doc.data.project_file.clone();
        let mapping_file = doc.data.mapping_file.clone();
        let mixer = doc.mixer.clone();
        let path_for_thread = path.clone();

        let (tx, rx) = mpsc::channel();
        let (progress_tx, progress_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = yinhe_yin::save_yin_with_files_progress(
                &model,
                &path_for_thread,
                &project_file,
                &mapping_file,
                Some(&mixer),
                |p| {
                    let _ = progress_tx.send(p);
                },
            );
            if let Err(e) = result {
                tracing::error!("Failed to save project: {}", e);
            }
            let _ = tx.send(());
        });

        if let Some(doc) = self.workspace.documents.get_mut(idx) {
            doc.file_path = Some(path);
        }
        self.save_rx = Some(rx);
        self.save_progress_rx = Some(progress_rx);
    }

    pub(crate) fn save_as_dialog(&mut self) {
        let default_name = if let Some(idx) = self.workspace.active_doc {
            format!("{}.yin", self.workspace.documents[idx].file_name)
        } else {
            t!("file_dialog.untitled").to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(t!("file_dialog.yinhe_project").as_ref(), &["yin"])
            .set_file_name(&default_name)
            .save_file()
        {
            let mut path_str = path.to_string_lossy().to_string();
            // Ensure .yin extension
            if !path_str.ends_with(".yin") {
                path_str.push_str(".yin");
            }
            if let Some(idx) = self.workspace.active_doc {
                let path2 = path_str.clone();
                self.save_project_async(idx, path2);
                // Update file_name
                if let Some(doc) = self.workspace.documents.get_mut(idx) {
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
        let default_name = if let Some(idx) = self.workspace.active_doc {
            format!("{}.mid", self.workspace.documents[idx].file_name)
        } else {
            t!("file_dialog.export_mid").to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .set_file_name(&default_name)
            .save_file()
        {
            let path_str = path.to_string_lossy().to_string();
            if let Some(idx) = self.workspace.active_doc {
                let doc = &self.workspace.documents[idx];
                let opts = yinhe_midi::MidiExportOptions {
                    encoding: self.audio_settings.midi_export_encoding,
                    rpn_full: self.audio_settings.midi_export_rpn_full,
                    curve_interpolate: self.audio_settings.midi_export_curve_interpolate,
                    curve_density: self.audio_settings.midi_export_curve_density,
                    strip_empty_tracks: self.audio_settings.midi_export_strip_empty_tracks,
                    dedup_overlaps: self.audio_settings.midi_export_dedup_overlaps,
                };
                match yinhe_midi::write_with_options(&doc.data.model, &opts) {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&path_str, &bytes) {
                            tracing::error!("Failed to export MIDI: {}", e);
                            self.notifications.error("导出MIDI失败", e.to_string());
                        } else {
                            let fname = std::path::Path::new(&path_str)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&path_str)
                                .to_string();
                            self.notifications.success("导出MIDI完成", fname);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to export MIDI: {}", e);
                        self.notifications.error("导出MIDI失败", e.to_string());
                    }
                }
            }
        }
    }

    fn export_audio_dialog(&mut self, ctx: &egui::Context) {
        if self.workspace.active_doc.is_none() {
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
        let idx = match self.workspace.active_doc {
            Some(idx) => idx,
            None => return,
        };

        // 真实计时起点：点击「开始导出」按钮的瞬间（包含后续文件对话框等待），
        // 避免之前 reset 在文件对话框之后导致 elapsed 不包含对话框等待、看起来
        // 像“点击之前就已经开始渲染”。
        let button_time = std::time::Instant::now();

        let doc = &self.workspace.documents[idx];
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
        // 记下输出路径：中止卡“打开文件夹”按钮用
        self.export.last_output_path = Some(path_str.clone());
        // 混音台 strip 参数随导出（insert 效果器不导出，见 export_wav 文档）。
        let mixer = doc.mixer.clone();
        let use_gpu_synth = self.audio_settings.use_gpu_synth;
        cancel_flag.store(false, std::sync::atomic::Ordering::Relaxed);

        // Reset progress state（计时起点为按钮点击时刻，保证壁钟时间真实）
        {
            let mut p = export_progress.lock().unwrap();
            p.reset();
            p.started_at = Some(button_time);
        }

        let (tx, rx) = mpsc::channel();

        // Try GPU export first — use the app's existing wgpu Device/Queue.
        #[cfg(feature = "gpu")]
        let gpu_device = std::sync::Arc::new(self.render_ctx.device().clone());
        #[cfg(feature = "gpu")]
        let gpu_queue = std::sync::Arc::new(self.render_ctx.queue().clone());
        // Extract SFZ paths per port for GPU export.
        #[cfg(feature = "gpu")]
        let gpu_port_sf = port_sf.clone();
        #[cfg(feature = "gpu")]
        eprintln!("[export] gpu port_sf = {:?}", gpu_port_sf);
        #[cfg(not(feature = "gpu"))]
        eprintln!("[export] GPU feature NOT enabled");

        std::thread::spawn(move || {
            eprintln!("[export] Thread started");
            // 根据设置选择导出引擎：GPU 还是 CPU
            #[cfg(feature = "gpu")]
            let result = if use_gpu_synth {
                if !gpu_port_sf.is_empty() {
                    eprintln!("[export] Using GPU path (GpuSynth)");
                    yinhe_audio::export::export_wav_gpu(
                        model,
                        sr,
                        &gpu_port_sf,
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
                        Some(export_progress.clone()),
                        Some(cancel_flag.clone()),
                        None,
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
                        None,
                        Some(&mixer),
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
                    None,
                    Some(&mixer),
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
                None,
                Some(&mixer),
            );
            // Capture final stats before hiding the progress window.
            let (elapsed, speed) = {
                let p = export_progress.lock().unwrap();
                let elapsed = p
                    .started_at
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0);
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
