//! Stub implementations for non-macOS platforms.

use super::MenuAction;
use yinhe_editor_core::shortcuts::Keybindings;

pub(crate) struct MenuBarInner {
    _rx: std::sync::mpsc::Receiver<MenuAction>,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (_, rx) = std::sync::mpsc::channel();
        Self { _rx: rx }
    }

    pub fn poll(&mut self, _keybindings: &Keybindings, _suspend: bool) -> Vec<MenuAction> {
        Vec::new()
    }

    pub fn poll_open_files(&mut self) -> Vec<String> {
        Vec::new()
    }
}

pub(crate) fn set_document_edited(_frame: &eframe::Frame, _edited: bool) {
    // No-op on non-macOS platforms
}

pub(crate) fn request_user_attention() {
    // No-op on non-macOS platforms
}

pub(crate) fn set_app_nap_enabled(_enabled: bool) {
    // No-op on non-macOS platforms
}
