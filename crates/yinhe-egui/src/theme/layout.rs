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
pub const AUTO_PANEL_COMBO_WIDTH_RATIO: f32 = 1.0;

// ── System monitoring ──
pub const SYS_REFRESH_INTERVAL_SECS: f64 = 0.5;
pub const MEM_POPUP_SIZE: [f32; 2] = [360.0, 260.0];
