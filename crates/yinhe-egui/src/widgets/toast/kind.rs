use eframe::egui;
use egui_material_icons::icons::*;

// ── Toast 种类 ──
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    pub(crate) fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            Self::Info => ICON_INFO,
            Self::Success => ICON_CHECK_CIRCLE,
            Self::Warning => ICON_WARNING,
            Self::Error => ICON_ERROR,
        }
    }

    pub(crate) fn color(self) -> egui::Color32 {
        match self {
            Self::Info => crate::theme::text_secondary(),
            Self::Success => crate::theme::accent_active(),
            Self::Warning => crate::theme::warning_gold(),
            Self::Error => crate::theme::danger_text(),
        }
    }
}
