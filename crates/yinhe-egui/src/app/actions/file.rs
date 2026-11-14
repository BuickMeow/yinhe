//! 文件菜单、最近文件、未保存确认与退出队列。

use eframe::egui;
use rust_i18n::t;

use crate::app::{App, PendingFileAction};
use crate::chrome::transport_bar;

/// 退出保存队列的推进结果。
enum ExitSaveStep {
    /// 已发起保存（或等待上一个保存完成），保持 pending 等回调推进。
    Started,
    /// 所有脏文档都已保存，可以退出。
    Done,
    /// 无路径文档的另存为对话框被取消：取消退出。
    Cancelled,
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
            | transport_bar::FileAction::ExportMidi7z
            | transport_bar::FileAction::ExportAudio
            | transport_bar::FileAction::Settings
            | transport_bar::FileAction::ProjectSettings
            | transport_bar::FileAction::ImportAudio
            | transport_bar::FileAction::ImportMidi
            | transport_bar::FileAction::Welcome
            | transport_bar::FileAction::About
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

    /// 打开「最近修改的文件」（transport bar 子菜单 / 标题栏 popup / macOS 菜单栏共用）。
    /// 文件已被移动/删除时从列表移除并报错。
    ///
    /// 打开是**追加新文档**（不取代/关闭现有文档），因此不检查未保存修改：
    /// 触发保存确认只在会丢失当前文档的场景（关闭文档 / 新建 / 退出）。
    pub(crate) fn open_recent_file(&mut self, path: &str) {
        if self.prune_missing_recent_file(path) {
            return;
        }
        // 打开意图是"进入编辑"，关闭可能显示中的欢迎页面。
        self.show_welcome_page = false;
        self.file_loader
            .load_path(path.to_string(), self.audio_settings.midi_import_encoding);
    }

    /// 最近文件已被移动/删除：移除记录、保存并报错，返回 true（调用方直接返回）。
    /// 「打开为文档」与「新窗口打开」两个入口共用。
    pub(crate) fn prune_missing_recent_file(&mut self, path: &str) -> bool {
        if std::path::Path::new(path).exists() {
            return false;
        }
        self.audio_settings.remove_recent_file(path);
        self.audio_settings.save();
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        self.show_error(
            t!("toast.file_missing"),
            t!("file_dialog.not_found", name = name),
        );
        true
    }

    /// 执行欢迎页动作（独立欢迎窗口 / 主界面无工程占位共用入口）。
    ///
    /// 映射到现有文件动作与最近文件逻辑，保证与菜单/走带栏入口行为一致。
    pub(crate) fn handle_welcome_action(
        &mut self,
        action: crate::welcome::WelcomeAction,
        ctx: &egui::Context,
    ) {
        use crate::welcome::WelcomeAction;
        match action {
            // 新建 / 打开 / 最近：目标动作内部会关闭欢迎页面（进入编辑语义）。
            WelcomeAction::NewProject => {
                self.handle_file_action(transport_bar::FileAction::NewProject, ctx);
            }
            WelcomeAction::OpenProject => {
                self.handle_file_action(transport_bar::FileAction::Open, ctx);
            }
            WelcomeAction::OpenRecent(path) => self.open_recent_file(&path),
            WelcomeAction::OpenSettings => {
                self.handle_file_action(transport_bar::FileAction::Settings, ctx);
            }
        }
    }

    /// Execute a file action immediately without checking for unsaved changes.
    fn execute_file_action(&mut self, action: transport_bar::FileAction, ctx: &egui::Context) {
        match action {
            transport_bar::FileAction::NewProject => {
                self.new_project();
            }
            transport_bar::FileAction::Open => {
                // 打开意图是"进入编辑"，关闭可能显示中的欢迎页面。
                self.show_welcome_page = false;
                self.file_loader
                    .pick_file(self.audio_settings.midi_import_encoding);
            }
            transport_bar::FileAction::ImportAudio => {
                self.import_audio_dialog();
            }
            transport_bar::FileAction::ImportMidi => {
                self.import_midi_dialog(ctx);
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
            transport_bar::FileAction::ExportMidi7z => {
                self.export_midi_archive_dialog();
            }
            transport_bar::FileAction::ExportAudio => {
                self.export_audio_dialog(ctx);
            }
            transport_bar::FileAction::Exit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            transport_bar::FileAction::Settings => {
                self.audio_settings.ui_session.show_settings = true;
                crate::chrome::dialog::raise_viewport(
                    ctx,
                    egui::ViewportId::from_hash_of("settings_dialog"),
                );
            }
            transport_bar::FileAction::Welcome => {
                // 打开独立欢迎窗口（与启动欢迎窗口同款）；已打开时拉到前台。
                self.show_welcome_page = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::welcome::viewport_id());
            }
            transport_bar::FileAction::About => {
                self.show_about = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::dialogs::about::viewport_id());
            }
            transport_bar::FileAction::ProjectSettings => {
                self.set_float_panel(ctx, Some(crate::right_panel::FloatPanel::ProjectSettings));
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
            PendingFileAction::CloseDocument(idx) => {
                self.close_document(idx);
            }
            PendingFileAction::Exit => {
                // 退出 = 保存所有脏文档，而不是只保存当前活动文档（此前其余
                // 脏标签会被直接丢弃）。逐个保存，每次保存完成后由 poll 再次
                // 进入这里推进队列。
                match self.save_next_dirty_for_exit() {
                    ExitSaveStep::Started => {
                        self.pending_unsaved = Some(PendingFileAction::Exit);
                    }
                    ExitSaveStep::Done => {
                        self.should_exit = true;
                    }
                    ExitSaveStep::Cancelled => {}
                }
            }
        }
    }

    /// 用户明确选择“不保存”：直接执行目标动作，不做任何保存。
    pub(crate) fn discard_pending_file_action(&mut self) {
        let Some(pending) = self.pending_unsaved.take() else {
            return;
        };
        match pending {
            PendingFileAction::NewProject => self.new_project(),
            PendingFileAction::CloseDocument(idx) => self.close_document(idx),
            PendingFileAction::Exit => self.should_exit = true,
        }
    }

    /// 退出流程：保存下一个脏文档。全部保存完返回 `Done`；无路径文档的
    /// “另存为”被取消时返回 `Cancelled`（取消退出，避免死循环弹窗）。
    fn save_next_dirty_for_exit(&mut self) -> ExitSaveStep {
        // 上一个保存任务还没结束：保持 pending，等回调推进。
        if self.jobs.is_saving() {
            return ExitSaveStep::Started;
        }
        let Some(idx) = self.workspace.documents.iter().position(|d| d.is_dirty()) else {
            return ExitSaveStep::Done;
        };
        match self.workspace.documents[idx].file_path.clone() {
            Some(path) => {
                self.save_project_async(idx, path);
                ExitSaveStep::Started
            }
            None => {
                if self.save_as_dialog_for(idx) {
                    ExitSaveStep::Started
                } else {
                    ExitSaveStep::Cancelled
                }
            }
        }
    }
}
