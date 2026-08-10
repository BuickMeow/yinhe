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
    pub fn poll(&mut self) -> Vec<MenuAction> {
        self.inner.poll()
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
