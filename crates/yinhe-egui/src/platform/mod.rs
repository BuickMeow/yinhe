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
}

/// Set the document-edited dot in the macOS traffic light close button.
/// On non-macOS platforms this is a no-op.
pub fn set_document_edited(frame: &eframe::Frame, edited: bool) {
    set_document_edited_inner(frame, edited);
}

// Re-export the platform-specific inner type and function.
#[cfg(target_os = "macos")]
use macos::{MenuBarInner, set_document_edited as set_document_edited_inner};
#[cfg(not(target_os = "macos"))]
use stub::{MenuBarInner, set_document_edited as set_document_edited_inner};
