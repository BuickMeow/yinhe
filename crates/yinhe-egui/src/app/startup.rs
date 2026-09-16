//! 启动阶段：主窗口隐藏，独立「启动页」窗口显示进度；音频引擎与插件扫描
//! 全部就绪后再显示主窗口，保证主界面一出现各项引擎都已可用。
//!
//! 实现依赖 eframe 的 deferred viewport：启动页是独立原生窗口，由主 viewport
//! 每帧声明存在（`show_viewport_deferred`），就绪后停止声明即随下一帧销毁。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

use super::App;

/// 启动页 viewport 标识（仅用于 `ViewportId::from_hash_of`）。
const SPLASH_VIEWPORT_ID: &str = "yinhe_startup_splash";

/// 启动阶段状态（`App` 字段）。
pub(crate) struct StartupState {
    /// 仍处于启动阶段（主窗口隐藏、启动页显示）。
    pub(crate) pending: bool,
    /// 启动页共享状态（deferred viewport 闭包无法访问 `App`）。
    pub(crate) shared: Arc<StartupShared>,
    /// 启动页是否已创建（首次创建时拉前台一次）。
    splash_created: bool,
}

impl StartupState {
    pub(crate) fn new() -> Self {
        Self {
            pending: true,
            shared: Arc::new(StartupShared {
                status: Mutex::new(String::new()),
                exit_requested: AtomicBool::new(false),
                skip_requested: AtomicBool::new(false),
            }),
            splash_created: false,
        }
    }
}

/// 启动页与主线程共享的状态（闭包要求 `Send + Sync`）。
pub(crate) struct StartupShared {
    /// 当前活动描述（主线程每帧写入）。
    status: Mutex<String>,
    /// 用户请求退出（Cmd+Q / 关闭请求）。
    exit_requested: AtomicBool,
    /// 用户请求跳过等待，直接进入主界面。
    skip_requested: AtomicBool,
}

/// 启动阶段就绪判定。
///
/// - 音频引擎 spawn 失败直接放行（主界面会显示错误），避免卡死在启动页；
/// - 文件加载 / 待激活文档未完成前不放行（经命令行打开的工程也在启动页里加载完）；
/// - 其余要求音频就绪且插件扫描完成。
fn startup_ready(
    audio_ready: bool,
    audio_failed: bool,
    scan_done: bool,
    file_loading: bool,
    pending_doc: bool,
) -> bool {
    if audio_failed {
        return true;
    }
    !file_loading && !pending_doc && audio_ready && scan_done
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
        let scan_done = !self.mix.scan_in_progress && self.mix.scanned.is_some();
        let skip = self
            .startup
            .shared
            .skip_requested
            .swap(false, Ordering::Relaxed);

        if skip
            || startup_ready(
                audio_ready,
                audio_failed,
                scan_done,
                self.file_loader.is_loading(),
                self.audio_state.pending_doc_activate.is_some(),
            )
        {
            // 就绪：进入主界面（不再声明启动页 → 窗口随之销毁）。
            self.startup.pending = false;
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
}

fn splash_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of(SPLASH_VIEWPORT_ID)
}

fn splash_viewport_builder() -> egui::ViewportBuilder {
    crate::chrome::dialog::viewport_builder(&rust_i18n::t!("startup.title"), [420.0, 230.0], false)
}

/// 启动页绘制（deferred viewport 闭包；不能访问 `App`）。
fn draw_splash(ui: &mut egui::Ui, shared: &StartupShared) {
    // 关闭 / Cmd+Q → 退出；Esc / 按钮 → 跳过等待。
    if ui.input(|i| i.viewport().close_requested())
        || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Q))
    {
        shared.exit_requested.store(true, Ordering::Relaxed);
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        shared.skip_requested.store(true, Ordering::Relaxed);
    }

    let status = shared
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(crate::theme::app_bg()))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.label(
                    egui::RichText::new(rust_i18n::t!("startup.title"))
                        .size(26.0)
                        .color(crate::theme::accent_active()),
                );
                ui.add_space(20.0);
                ui.add(
                    egui::Spinner::new()
                        .size(22.0)
                        .color(crate::theme::accent_active()),
                );
                ui.add_space(12.0);
                ui.label(egui::RichText::new(status).color(crate::theme::text_secondary()));
            });
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                let skip = ui.add(
                    egui::Button::new(
                        egui::RichText::new(rust_i18n::t!("startup.skip"))
                            .color(crate::theme::text_muted()),
                    )
                    .frame(false),
                );
                if skip.clicked() {
                    shared.skip_requested.store(true, Ordering::Relaxed);
                }
            });
        });

    // 启动页独立重绘：Spinner 动画 + 主线程写入的状态刷新。
    ui.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use super::startup_ready;

    #[test]
    fn startup_ready_requires_audio_and_scan() {
        assert!(startup_ready(true, false, true, false, false));
        assert!(!startup_ready(false, false, true, false, false));
        assert!(!startup_ready(true, false, false, false, false));
    }

    #[test]
    fn startup_ready_waits_for_loading_and_pending_doc() {
        assert!(!startup_ready(true, false, true, true, false));
        assert!(!startup_ready(true, false, true, false, true));
    }

    #[test]
    fn startup_ready_releases_on_audio_failure() {
        // spawn 失败必须放行，否则启动页永远等不到 audio_ready。
        assert!(startup_ready(false, true, false, false, false));
        assert!(startup_ready(false, true, false, true, true));
    }
}
