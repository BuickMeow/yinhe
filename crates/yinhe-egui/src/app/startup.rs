//! 启动阶段：主窗口隐藏，独立「启动页」窗口显示进度；音频引擎与插件扫描
//! 全部就绪后再显示主窗口，保证主界面一出现各项引擎都已可用。
//!
//! 实现依赖 eframe 的 deferred viewport：启动页是独立原生窗口，由主 viewport
//! 每帧声明存在（`show_viewport_deferred`），就绪后停止声明即随下一帧销毁。
//! 启动页的绘制在 `splash.rs`，本模块只维护启动/欢迎阶段状态机。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

use super::App;
use super::splash::{draw_splash, splash_viewport_builder, splash_viewport_id};

/// 启动阶段状态（`App` 字段）。
pub(crate) struct StartupState {
    /// 仍处于启动阶段（主窗口隐藏、启动页显示）。
    pub(crate) pending: bool,
    /// 启动就绪后无工程可打开：显示独立欢迎窗口（主窗口保持隐藏），
    /// 用户完成选择（新建 / 打开 / 最近）后进入主界面。
    pub(crate) welcome: bool,
    /// 启动页共享状态（deferred viewport 闭包无法访问 `App`）。
    pub(crate) shared: Arc<StartupShared>,
    /// 启动页是否已创建（首次创建时拉前台一次）。
    splash_created: bool,
}

impl StartupState {
    pub(crate) fn new() -> Self {
        Self {
            pending: true,
            welcome: false,
            shared: Arc::new(StartupShared {
                status: Mutex::new(String::new()),
                exit_requested: AtomicBool::new(false),
            }),
            splash_created: false,
        }
    }
}

/// 启动页与主线程共享的状态（闭包要求 `Send + Sync`）。
pub(crate) struct StartupShared {
    /// 当前活动描述（主线程每帧写入；splash.rs 绘制读取）。
    pub(crate) status: Mutex<String>,
    /// 用户请求退出（Cmd+Q / 关闭请求；splash.rs 写入）。
    pub(crate) exit_requested: AtomicBool,
}

/// 启动阶段就绪判定。
///
/// - 音频引擎 spawn 失败直接放行（主界面会显示错误），避免卡死在启动页；
/// - 文件加载 / 待激活文档未完成前不放行（经命令行打开的工程也在启动页里加载完）；
/// - 其余只要求音频就绪。**插件扫描不再阻塞启动**：它只服务插件选择器，
///   后台完成后 UI 自动刷新（插件多/坏插件不再拖住主界面与启动耗时）。
fn startup_ready(
    audio_ready: bool,
    audio_failed: bool,
    file_loading: bool,
    pending_doc: bool,
) -> bool {
    if audio_failed {
        return true;
    }
    !file_loading && !pending_doc && audio_ready
}

impl App {
    /// 启动阶段每帧：隐藏主窗口、驱动必要的后台轮询、显示启动页；
    /// 就绪后恢复主窗口（下一帧起走正常主循环）。
    pub(crate) fn startup_ui(&mut self, ui: &mut egui::Ui) {
        // 启动页请求退出（关闭/Cmd+Q）：走正常退出路径，不再显示启动页。
        if self.startup.shared.exit_requested.load(Ordering::Relaxed) {
            self.should_exit = true;
        }
        if self.should_exit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        crate::theme::apply_to_ctx(ui.ctx());

        // 主窗口保持隐藏：eframe 首帧后会无条件显示一次（防白闪），这里立即
        // 隐藏；隐藏期间刷新被 eframe 节流到约 100ms，足够驱动轮询。
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Visible(false));

        // 必要后台流程（与主循环同一调用顺序；纯渲染/交互相关一概跳过）。
        self.poll_async_operations();
        self.rebuild_audio_if_needed();
        self.poll_audio_spawn();
        self.poll_audio_progress();
        self.poll_insert_returns();
        self.poll_pending_doc_activate();

        // 插件扫描：首次启动立刻发起；完成判定见 startup_ready。
        if self.mix.scanned.is_none() && !self.mix.scan_in_progress {
            crate::mix::start_plugin_scan(self);
        }
        crate::mix::poll_plugin_scan(self);

        // 启动阶段只处理退出类菜单动作（macOS 原生菜单 Cmd+Q 在此拦截），
        // 其余动作丢弃：主界面尚未就绪，任何文件/编辑动作都无处落地。
        for action in self.menu_bar.poll(
            &self.audio_settings.keybindings,
            false,
            &self.audio_settings.recent_files,
            self.follow_mode,
            self.active_tool,
        ) {
            if matches!(action, crate::platform::MenuAction::Exit) {
                self.should_exit = true;
            }
        }
        if self.should_exit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let audio_ready = self
            .audio_state
            .handle
            .as_ref()
            .is_some_and(|a| a.handle.audio_ready());
        let audio_failed = self.audio_state.spawn_error.is_some();
        // 无工程（启动欢迎页路径）不等音频：音频按文档创建，无文档时不会 spawn，
        // 等待音频就绪会永远卡在启动页。有工程/正在加载时才等音频。
        let no_project = self.workspace.documents.is_empty()
            && !self.file_loader.is_loading()
            && self.audio_state.pending_doc_activate.is_none();
        let audio_ready = no_project || audio_ready;

        if startup_ready(
            audio_ready,
            audio_failed,
            self.file_loader.is_loading(),
            self.audio_state.pending_doc_activate.is_some(),
        ) {
            self.startup.pending = false;
            // 无工程可打开（无命令行文件 / 无待恢复残留）→ 先显示独立欢迎窗口，
            // 主窗口保持隐藏；用户选择新建/打开/最近后由 `welcome_ui` 进入主界面。
            if self.welcome_needed() {
                self.startup.welcome = true;
                ui.ctx().request_repaint();
                return;
            }
            // 就绪：进入主界面（不再声明启动页 → 窗口随之销毁）。
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            ui.ctx().request_repaint();
            return;
        }

        // 更新启动页状态文字。
        let status = self.startup_status_text();
        *self
            .startup
            .shared
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = status;

        let shared = Arc::clone(&self.startup.shared);
        ui.ctx().show_viewport_deferred(
            splash_viewport_id(),
            splash_viewport_builder(),
            move |ui, _class| draw_splash(ui, &shared),
        );
        if !self.startup.splash_created {
            // 首次创建：拉前台（主窗口隐藏，启动页是当前唯一可见窗口）。
            self.startup.splash_created = true;
            ui.ctx()
                .send_viewport_cmd_to(splash_viewport_id(), egui::ViewportCommand::Focus);
        }
        // 主窗口隐藏时刷新被节流；持续请求以驱动轮询与启动页状态刷新。
        ui.ctx().request_repaint();
    }

    /// 启动页显示的当前活动描述。
    fn startup_status_text(&self) -> String {
        let active_stage = self
            .jobs
            .load_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stages
            .iter()
            .find(|s| s.status == yinhe_editor_core::progress::StageStatus::Active)
            .map(|s| {
                if s.detail.is_empty() {
                    s.label.clone()
                } else {
                    format!("{} {}", s.label, s.detail)
                }
            });
        if let Some(text) = active_stage {
            return text;
        }
        if self.mix.scan_in_progress {
            return rust_i18n::t!("mix.scanning").to_string();
        }
        rust_i18n::t!("startup.preparing").to_string()
    }

    /// 启动就绪后是否需要先显示欢迎窗口：没有任何工程可打开时才弹
    /// （命令行文件、自动恢复残留都算「有工程可打开」，直接进主界面）。
    ///
    /// 启动不再预置空白工程（`Workspace::empty`），所以只需判断文档为空。
    fn welcome_needed(&self) -> bool {
        self.workspace.documents.is_empty() && self.jobs.autosave.recovery.is_none()
    }

    /// 欢迎窗口阶段每帧：主窗口保持隐藏、独立欢迎窗口显示；
    /// 用户完成选择（新建 / 打开 / 最近）后进入主界面。
    ///
    /// 与启动页同一原则：只驱动必要后台流程（音频 / 文件加载 / 插件扫描），
    /// 反馈（加载 toast 等）统一由主界面承担。
    pub(crate) fn welcome_ui(&mut self, ui: &mut egui::Ui) {
        if self.should_exit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        crate::theme::apply_to_ctx(ui.ctx());

        // 主窗口保持隐藏（与启动页同理；隐藏期间刷新被 eframe 节流到约 100ms）。
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Visible(false));

        // 必要后台流程（与启动阶段同一调用集合）。
        self.poll_async_operations();
        self.rebuild_audio_if_needed();
        self.poll_audio_spawn();
        self.poll_audio_progress();
        self.poll_insert_returns();
        self.poll_pending_doc_activate();
        if self.mix.scanned.is_none() && !self.mix.scan_in_progress {
            crate::mix::start_plugin_scan(self);
        }
        crate::mix::poll_plugin_scan(self);

        // macOS 原生菜单：与主界面同一套入口（新建/打开/设置/最近/退出）；
        // 设置窗口打开或快捷键录制时暂停加速键（与主循环一致）。
        let suspend_menu_accels = self.audio_settings.ui_session.show_settings
            || self.audio_settings.ui_session.shortcut_recording;
        // 用户是否已做出选择（决定何时进入主界面）。
        let mut chosen = false;
        for action in self.menu_bar.poll(
            &self.audio_settings.keybindings,
            suspend_menu_accels,
            &self.audio_settings.recent_files,
            self.follow_mode,
            self.active_tool,
        ) {
            use crate::platform::MenuAction;
            match action {
                MenuAction::Exit => self.should_exit = true,
                MenuAction::NewProject => {
                    self.handle_file_action(
                        crate::chrome::transport_bar::FileAction::NewProject,
                        ui.ctx(),
                    );
                    chosen = true;
                }
                MenuAction::Open => {
                    self.handle_file_action(
                        crate::chrome::transport_bar::FileAction::Open,
                        ui.ctx(),
                    );
                    chosen = self.file_loader.is_loading();
                }
                MenuAction::Settings => self.handle_file_action(
                    crate::chrome::transport_bar::FileAction::Settings,
                    ui.ctx(),
                ),
                MenuAction::About => self
                    .handle_file_action(crate::chrome::transport_bar::FileAction::About, ui.ctx()),
                MenuAction::OpenRecent(path) => {
                    self.open_recent_file(&path);
                    chosen = self.file_loader.is_loading();
                }
                // 其余动作无处落地（无文档），丢弃。
                _ => {}
            }
        }

        // 独立欢迎窗口（同一帧返回动作）。
        let (closed, action) = crate::welcome::show_window(
            ui.ctx(),
            &self.audio_settings.recent_files,
            &self.audio_settings.recent_file_times,
        );
        if closed {
            // 无工程则退出（启动向导语义：关闭欢迎窗口 = 放弃启动）。
            self.should_exit = true;
        }
        if let Some(action) = action {
            if matches!(action, crate::welcome::WelcomeAction::NewProject) {
                chosen = true;
            }
            self.handle_welcome_action(action, ui.ctx());
            // 打开/最近：文件已开始加载才算选择成功（文件选择器取消则留在欢迎窗口）。
            chosen |= self.file_loader.is_loading();
        }

        // 设置窗口：欢迎页「设置」按钮打开的窗口也在此渲染
        // （与主循环 dialog_dispatch 的同逻辑：关闭时让设置落地）。
        let (settings_closed, settings_changed) = crate::dialogs::settings::show_viewport(
            ui.ctx(),
            &mut self.audio_settings,
            &self.audio_state.handle,
        );
        if settings_closed {
            self.apply_settings_on_close();
        } else if settings_changed {
            self.sync_soundfonts_online();
        }

        // 关于窗口（菜单「关于 Yinhe」打开的窗口也在此渲染）。
        crate::dialogs::about::show_viewport(ui.ctx(), &mut self.show_about);

        if self.should_exit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // 用户已做出选择（新建完成 / 打开或最近文件已开始加载）→ 进入主界面；
        // 加载进度、失败提示等反馈都由主界面的 toast 承担。
        if chosen {
            self.startup.welcome = false;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            ui.ctx().request_repaint();
            return;
        }

        // 主窗口隐藏时刷新被节流；持续请求以驱动欢迎窗口交互与后台轮询。
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::startup_ready;

    /// 插件扫描不再阻塞启动：未扫描完成也应放行（旧行为会卡在启动页，
    /// 坏插件子进程挂死时永不进入主界面）。
    #[test]
    fn startup_ready_requires_audio_only() {
        assert!(startup_ready(true, false, false, false));
        assert!(!startup_ready(false, false, false, false));
    }

    #[test]
    fn startup_ready_waits_for_loading_and_pending_doc() {
        assert!(!startup_ready(true, false, true, false));
        assert!(!startup_ready(true, false, false, true));
    }

    #[test]
    fn startup_ready_releases_on_audio_failure() {
        // spawn 失败必须放行，否则启动页永远等不到 audio_ready。
        assert!(startup_ready(false, true, false, false));
        assert!(startup_ready(false, true, true, true));
    }
}
