use eframe::egui;

// ── Toast 种类 ──
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    pub(crate) fn color(self) -> egui::Color32 {
        match self {
            Self::Info => crate::theme::text_secondary(),
            Self::Success => crate::theme::accent_active(),
            Self::Warning => crate::theme::warning_gold(),
            Self::Error => crate::theme::danger_text(),
        }
    }
}
