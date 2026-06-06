use eframe::egui;

// ── App-level background color (used across all panels) ──
pub const APP_BG: egui::Color32 = egui::Color32::from_rgb(25, 25, 28);

// ── Active accent color ──
pub const ACCENT_ACTIVE: egui::Color32 = egui::Color32::from_rgb(100, 180, 255);

// ── Tab colors ──
pub const TAB_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(55, 55, 60);
pub const TAB_INACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(35, 35, 38);

// ── Close / danger hover ──
pub const DANGER_HOVER: egui::Color32 = egui::Color32::from_rgb(200, 50, 50);

// ── Window button hover (non-macOS) ──
#[cfg(not(target_os = "macos"))]
pub const WIN_BTN_HOVER: egui::Color32 = egui::Color32::from_rgb(80, 80, 85);

// ── Time ruler colors ──
pub const RULER_BG: egui::Color32 = egui::Color32::from_rgb(0x14, 0x14, 0x18);
pub const RULER_DIVIDER: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3F);
pub const MEASURE_LABEL: egui::Color32 = egui::Color32::from_rgb(0xAA, 0xAA, 0xAF);
pub const BEAT_LABEL: egui::Color32 = egui::Color32::from_rgb(0x77, 0x77, 0x7C);
pub const SUB_BEAT_LABEL: egui::Color32 = egui::Color32::from_rgb(0x55, 0x55, 0x5A);
pub const TICK_LABEL: egui::Color32 = egui::Color32::from_rgb(0x44, 0x44, 0x49);

// ── Scrollbar colors ──
pub const SCROLLBAR_BG: egui::Color32 = egui::Color32::from_rgb(0x14, 0x14, 0x18);
pub const SCROLLBAR_RECT: egui::Color32 = egui::Color32::from_rgb(0x50, 0x50, 0x58);
pub const SCROLLBAR_HOVER: egui::Color32 = egui::Color32::from_rgb(0x70, 0x70, 0x78);
pub const SCROLLBAR_DRAG: egui::Color32 = egui::Color32::from_rgb(0x90, 0x90, 0x98);

// ── Split handle colors ──
pub const SPLIT_HOVER: egui::Color32 = egui::Color32::from_gray(100);
pub const SPLIT_DEFAULT: egui::Color32 = egui::Color32::from_gray(60);
pub const V_SPLIT_HOVER: egui::Color32 = egui::Color32::from_gray(160);
pub const V_SPLIT_DEFAULT: egui::Color32 = egui::Color32::from_gray(80);

// ── Layout constants ──
pub const TITLE_BAR_H: f32 = 32.0;
pub const RULER_H: f32 = 24.0;
pub const SCROLLBAR_H: f32 = 24.0;
pub const SPLIT_GAP: f32 = 4.0;
