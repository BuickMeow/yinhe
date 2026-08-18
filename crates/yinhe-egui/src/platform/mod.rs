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

/// macOS：播放时阻止 App Nap（防止系统降低定时器精度）；非 macOS 平台为空操作。
pub fn set_app_nap_enabled(enabled: bool) {
    set_app_nap_enabled_inner(enabled);
}

// Re-export the platform-specific inner type and function.
#[cfg(target_os = "macos")]
use macos::{
    MenuBarInner, request_user_attention as request_user_attention_inner,
    set_app_nap_enabled as set_app_nap_enabled_inner,
    set_document_edited as set_document_edited_inner,
};
#[cfg(not(target_os = "macos"))]
use stub::{
    MenuBarInner, request_user_attention as request_user_attention_inner,
    set_app_nap_enabled as set_app_nap_enabled_inner,
    set_document_edited as set_document_edited_inner,
};
