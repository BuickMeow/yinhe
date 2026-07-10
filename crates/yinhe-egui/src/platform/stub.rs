//! Stub implementations for non-macOS platforms.

use super::MenuAction;

pub(crate) struct MenuBarInner {
    _rx: std::sync::mpsc::Receiver<MenuAction>,
}

impl MenuBarInner {
    pub fn new() -> Self {
        let (_, rx) = std::sync::mpsc::channel();
        Self { _rx: rx }
    }

    pub fn poll(&mut self) -> Vec<MenuAction> {
        Vec::new()
    }
}

pub(crate) fn set_document_edited(_frame: &eframe::Frame, _edited: bool) {
    // No-op on non-macOS platforms
}

pub(crate) fn request_user_attention() {
    // No-op on non-macOS platforms
}
