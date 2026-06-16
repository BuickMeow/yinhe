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
pub const MODE_LABEL_FONT: f32 = 9.5;
pub const TAB_H: f32 = 24.0;
pub const TRANSPORT_BTN_SIZE: f32 = 32.0;
pub const TRANSPORT_BTN_FONT: f32 = 18.0;
pub const TIMECODE_FONT: f32 = 12.0;
pub const FILE_MENU_FONT: f32 = 14.0;
pub const SETTINGS_ICON_FONT: f32 = 14.0;
pub const SETTINGS_WINDOW_SIZE: [f32; 2] = [480.0, 400.0];

// ── Interaction thresholds ──
pub const CLICK_DISTANCE_THRESHOLD: f32 = 8.0;
pub const SCROLLBAR_EDGE_WIDTH: f32 = 4.0;
pub const ZOOM_DEADZONE: f32 = 0.001;
pub const SCROLL_TO_ZOOM_THRESHOLD: f32 = 0.5;
pub const ZOOM_FACTOR_PER_TICK: f32 = 1.1;
pub const DRAG_CLICK_MAX_DISTANCE: f32 = 3.0;

// ── Layout defaults ──
pub const DEFAULT_ARR_SPLIT: f32 = 0.3;
pub const DEFAULT_TRANSPORT_WIDTH: f32 = 200.0;
pub const MIN_ARR_HEIGHT: f32 = 60.0;
pub const SPLIT_CLAMP_MIN: f32 = 0.1;
pub const SPLIT_CLAMP_MAX: f32 = 0.7;
pub const MIN_KEYBOARD_WIDTH: f32 = 30.0;
pub const MAX_KEYBOARD_RATIO: f32 = 0.4;

// ── Piano roll / arrangement ──
pub const WHEEL_SCROLL_SPEED: f32 = 5000.0;
pub const NOTE_CURSOR_THRESHOLD: f64 = 0.005;

// ── Right panel ──
pub const RIGHT_PANEL_MIN_WIDTH: f32 = 160.0;
pub const RIGHT_PANEL_DEFAULT_WIDTH: f32 = 320.0;

// ── Automation panel ──
pub const AUTO_PANEL_SPLIT_H: f32 = 4.0;
pub const AUTO_PANEL_COMBO_WIDTH_RATIO: f32 = 1.0; // combo box width equals keyboard width

// ── System monitoring ──
pub const SYS_REFRESH_INTERVAL_SECS: f64 = 0.5;
pub const MEM_POPUP_SIZE: [f32; 2] = [360.0, 260.0];
