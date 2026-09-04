//! Platform-specific integrations (macOS menu bar, document-edited dot, etc.)

#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
mod stub;

/// Actions from the native menu bar.
#[derive(Clone, Debug)]
pub enum MenuAction {
    NewProject,
    Open,
    Save,
    SaveAs,
    CloseDocument,
    ExportAudio,
    ExportMidi,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Duplicate,
    Delete,
    TransposeUp,
    TransposeDown,
    DedupWithinTrack,
    DedupAcrossTracks,
    /// 播放菜单「播放/暂停」（Space）。
    TogglePlay,
    /// 播放菜单「停止」（Esc）。
    Stop,
    /// 播放菜单「录音」切换。
    ToggleRecord,
    /// 播放菜单「步进输入」切换。
    ToggleStepInput,
    /// 播放菜单「播放跟随」四档单选。
    SetFollowMode(yinhe_editor_core::follow::FollowMode),
    /// 文件菜单「最近修改的文件」子菜单选中的路径。
    OpenRecent(String),
    /// App 菜单「设置…」（⌘,），打开应用设置对话框。
    Settings,
    /// 文件菜单「工程设置…」，打开工程设置浮窗。
    ProjectSettings,
    /// App 菜单「退出」（⌘Q），走未保存检查流程。
    Exit,
    /// 系统级动作：由平台层就地执行，不经过主线程通道。
    About,
    Hide,
    HideOthers,
    ShowAll,
}

/// Handle to the native menu bar and its action receiver.
pub struct MenuBar {
    inner: MenuBarInner,
}

impl MenuBar {
    pub fn new() -> Self {
        Self {
            inner: MenuBarInner::new(),
        }
    }

    /// Poll for pending menu actions.
    /// keybindings 用于检测快捷键配置变化并同步原生菜单加速键（macOS）；
    /// suspend 为 true（设置窗口打开/快捷键录制中）时暂停原生菜单加速键，
    /// 避免系统级拦截组合键。
    /// recent_files / follow_mode 用于同步原生菜单的动态内容
    /// （最近文件子菜单、跟随档勾选），与 transport bar popup 保持一致。
    pub fn poll(
        &mut self,
        keybindings: &yinhe_editor_core::shortcuts::Keybindings,
        suspend: bool,
        recent_files: &[String],
        follow_mode: yinhe_editor_core::follow::FollowMode,
    ) -> Vec<MenuAction> {
        self.inner
            .poll(keybindings, suspend, recent_files, follow_mode)
    }

    /// Poll for file paths passed in by the OS (Finder "Open With" on macOS).
    pub fn poll_open_files(&mut self) -> Vec<String> {
        self.inner.poll_open_files()
    }
}

/// Set the document-edited dot in the macOS traffic light close button.
/// On non-macOS platforms this is a no-op.
pub fn set_document_edited(frame: &eframe::Frame, edited: bool) {
    set_document_edited_inner(frame, edited);
}

/// 让 macOS Dock 栏图标跳动，提示用户注意。
/// 非 macOS 平台为空操作。
pub fn request_user_attention() {
    request_user_attention_inner();
}

/// 在系统文件管理器中打开文件所在目录（macOS open / Windows explorer / Linux xdg-open）。
/// 路径不存在或启动失败时静默忽略（toast 场景不值得弹错）。
pub fn open_containing_folder(path: &std::path::Path) {
    let dir: &std::path::Path = if path.is_dir() {
        path
    } else {
        let Some(parent) = path.parent() else {
            return;
        };
        parent
    };
    if dir.as_os_str().is_empty() {
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// macOS：播放时阻止 App Nap（防止系统降低定时器精度）；非 macOS 平台为空操作。
pub fn set_app_nap_enabled(enabled: bool) {
    set_app_nap_enabled_inner(enabled);
}

/// 禁用系统标题栏区域的背景拖动（macOS：content view 的
/// `mouseDownCanMoveWindow` → NO）；非 macOS 平台为空操作。
/// 窗口拖动由 title_bar / transport_bar 的手动 StartDrag 追踪负责。
pub fn disable_background_window_drag(frame: &eframe::Frame) {
    disable_background_window_drag_inner(frame);
}

// Re-export the platform-specific inner type and function.
#[cfg(target_os = "macos")]
use macos::{
    MenuBarInner, disable_background_window_drag as disable_background_window_drag_inner,
    request_user_attention as request_user_attention_inner,
    set_app_nap_enabled as set_app_nap_enabled_inner,
    set_document_edited as set_document_edited_inner,
};
#[cfg(not(target_os = "macos"))]
use stub::{
    MenuBarInner, disable_background_window_drag as disable_background_window_drag_inner,
    request_user_attention as request_user_attention_inner,
    set_app_nap_enabled as set_app_nap_enabled_inner,
    set_document_edited as set_document_edited_inner,
};
