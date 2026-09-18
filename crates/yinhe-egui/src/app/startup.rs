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

        if startup_ready(
            audio_ready,
            audio_failed,
            self.file_loader.is_loading(),
            self.audio_state.pending_doc_activate.is_some(),
        ) {
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
    let vb = egui::ViewportBuilder::default()
        .with_title(&*rust_i18n::t!("startup.title"))
        .with_inner_size([420.0, 230.0])
        .with_resizable(false);

    // macOS：保留系统窗口框架（系统圆角 + 系统阴影 + 可拖动），只隐藏标题与
    // 交通灯——titled 窗口 + fullsize content view + 透明标题栏（Safari/Chrome
    // 同款做法）；`decorations(false)` 会丢掉圆角和阴影，不可用。
    #[cfg(target_os = "macos")]
    let vb = vb
        .with_transparent(true)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(false);

    // 其他平台：无边框窗口（无最小化-最大化-关闭按钮）。
    #[cfg(not(target_os = "macos"))]
    let vb = vb.with_decorations(false);

    vb
}

/// 启动页绘制（deferred viewport 闭包；不能访问 `App`）。
///
/// 布局：大号 "Yinhe" 作背景字放左下角，当前流程（Spinner + 文案）放右下角。
fn draw_splash(ui: &mut egui::Ui, shared: &StartupShared) {
    // 无系统按钮：关闭请求 / Cmd+Q 是退出方式。
    if ui.input(|i| i.viewport().close_requested())
        || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Q))
    {
        shared.exit_requested.store(true, Ordering::Relaxed);
    }

    let status = shared
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let painter = ui.painter().clone();
    let rect = ui.max_rect();
    // 窗口无装饰：全窗口自绘背景。
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());

    // 左右下三边同边距；文字按字形实际边界（mesh_bounds）对齐，
    // 消除字体行高下行留白带来的"下方边距偏大"。
    let inner = rect.shrink(18.0);

    // 左下角：大号 "Yinhe"（背景字）。
    let title_color = crate::theme::text_primary();
    let title = painter.layout_no_wrap(
        rust_i18n::t!("startup.title").to_string(),
        egui::FontId::proportional(72.0),
        title_color,
    );
    let title_pos = egui::pos2(
        inner.left() - title.mesh_bounds.left(),
        inner.bottom() - title.mesh_bounds.bottom(),
    );
    painter.galley(title_pos, title, title_color);

    // 右下角：Spinner + 当前流程文案。
    let text_color = crate::theme::text_secondary();
    let status = painter.layout_no_wrap(status, egui::FontId::proportional(14.0), text_color);
    let status_pos = egui::pos2(
        inner.right() - status.mesh_bounds.right(),
        inner.bottom() - status.mesh_bounds.bottom(),
    );
    let status_center_y = inner.bottom() - status.mesh_bounds.height() / 2.0;
    let status_left = status_pos.x + status.mesh_bounds.left();
    painter.galley(status_pos, status, text_color);

    let spinner_size = 16.0;
    let spinner_rect = egui::Rect::from_center_size(
        egui::pos2(status_left - 8.0 - spinner_size / 2.0, status_center_y),
        egui::vec2(spinner_size, spinner_size),
    );
    egui::Spinner::new()
        .size(spinner_size)
        .color(crate::theme::accent_active())
        .paint_at(ui, spinner_rect);

    // Spinner 动画 + 主线程状态写入的刷新。
    ui.ctx().request_repaint();
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
