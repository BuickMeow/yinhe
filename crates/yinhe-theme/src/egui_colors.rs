//! Egui-facing color constants.
//! Only available when the `egui` feature is enabled.

#[cfg(feature = "egui")]
use egui::Color32;

// ── App-level background color ──
pub const APP_BG: Color32 = Color32::from_rgb(25, 25, 28);

// ── Active accent color ──
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(100, 180, 255);

// ── 模式栏（mode bar）文字/图标色：讲解行与性能显示（CPU/MEM/FPS）的灰色字 ──
pub const MODE_BAR_TEXT: Color32 = Color32::GRAY;

// ── UI 文字/图标灰度（值对应各 UI 层既有用法，统一从 theme 取色）──
pub const TEXT_PRIMARY: Color32 = Color32::from_gray(220); // 标题/强调文字
pub const TEXT_BRIGHT: Color32 = Color32::from_gray(200); // 主文字/按钮文字
pub const TEXT_MEDIUM: Color32 = Color32::from_gray(190); // 箭头/图标
pub const TEXT_SECONDARY: Color32 = Color32::from_gray(180); // 次要文字
pub const TAB_DIRTY_DOT: Color32 = Color32::from_gray(200); // 标签未保存圆点（比文字深一点的灰）
pub const TEXT_MUTED: Color32 = Color32::from_gray(160); // 弱化文字/未激活图标
pub const TEXT_FAINT: Color32 = Color32::from_gray(140); // 弱化文字
pub const TEXT_DIM: Color32 = Color32::from_gray(120); // 小标签/弱文字
pub const TEXT_DIMMER: Color32 = Color32::from_gray(110); // 摘要文字
pub const TEXT_HINT: Color32 = Color32::from_gray(100); // 提示/占位文字
pub const TEXT_LABEL_DIM: Color32 = Color32::from_gray(90); // 键盘音名等极弱文字
pub const TEXT_DISABLED: Color32 = Color32::from_gray(80); // 禁用文字
pub const BTN_BG: Color32 = Color32::from_gray(45); // 内联按钮底色
pub const BTN_BG_HOVER: Color32 = Color32::from_gray(70); // 内联按钮悬停底色
pub const BORDER_DIM: Color32 = Color32::from_gray(60); // 分隔线/描边/弱背景
pub const ACTION_BAR_BG: Color32 = Color32::from_rgb(30, 30, 35); // 浮动操作条背景

// ── 语义色 ──
pub const DANGER: Color32 = Color32::from_rgb(232, 17, 35); // 关闭按钮红
pub const DANGER_TEXT: Color32 = Color32::from_rgb(232, 80, 80); // 危险文字
pub const DANGER_TEXT_BRIGHT: Color32 = Color32::from_rgb(255, 80, 80); // 危险强调文字
pub const ERROR_TEXT: Color32 = Color32::from_rgb(220, 80, 80); // 错误提示文字
pub const OK_GREEN: Color32 = Color32::from_rgb(80, 200, 80); // 成功/启用
pub const WARNING: Color32 = Color32::from_rgb(230, 160, 40); // 警告
pub const WARNING_GOLD: Color32 = Color32::from_rgb(220, 180, 90); // 金色标记

// ── Event browser 行选中底色（树形导航 / PR 根音行共用） ──
pub const ROW_SELECTED_BG: Color32 = Color32::from_rgb(40, 50, 70);

// ── Tab colors ──
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(55, 55, 60);
pub const TAB_INACTIVE_BG: Color32 = Color32::from_rgb(35, 35, 38);
pub const TAB_HOVER_BG: Color32 = Color32::from_rgb(48, 48, 52);

// ── Close / danger hover ──
pub const DANGER_HOVER: Color32 = Color32::from_rgb(200, 50, 50);

// ── Window button hover (non-macOS) ──
#[cfg(not(target_os = "macos"))]
pub const WIN_BTN_HOVER: Color32 = Color32::from_rgb(80, 80, 85);

// ── Time ruler colors ──
pub const RULER_BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x18);
pub const RULER_DIVIDER: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3F);
pub const MEASURE_LABEL: Color32 = Color32::from_rgb(0xAA, 0xAA, 0xAF);
pub const BEAT_LABEL: Color32 = Color32::from_rgb(0x77, 0x77, 0x7C);
pub const SUB_BEAT_LABEL: Color32 = Color32::from_rgb(0x55, 0x55, 0x5A);
pub const TICK_LABEL: Color32 = Color32::from_rgb(0x44, 0x44, 0x49);

// ── Grid line colors（pianoroll / automation 共用 pr_*，arrangement 用 ar_*）──
// 从 GpuTheme 迁移而来，统一用 Color32。alpha 直接编进 Color32。
pub const PR_MEASURE_LINE: Color32 = Color32::from_rgb(0x59, 0x59, 0x66); // (0.35,0.35,0.40,1.0)
pub const PR_BEAT_LINE: Color32 = Color32::from_rgb(0x38, 0x38, 0x40); // (0.22,0.22,0.25,1.0)
pub const PR_SUB_BEAT_LINE: Color32 = Color32::from_rgb(0x29, 0x29, 0x2E); // (0.16,0.16,0.18,1.0)
pub const PR_TICK_LINE: Color32 = Color32::from_rgb(0x25, 0x25, 0x2A);

// ── Piano roll 横向：八度线 / 调号背景 ──
/// 每个八度边界的横向细线（C 位置）。
pub const PR_OCTAVE_LINE: Color32 = Color32::from_rgb(0x38, 0x38, 0x40);
/// 调外音行背景（与 `PR_BLACK_KEY_ROW` 同色，保证有/无调号时视觉一致）。
pub const PR_SCALE_OUTSIDE: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1F);
/// 根音行背景（与事件列表选中高亮蓝一致：RGB(40,50,70)）。
pub const PR_ROOT_NOTE: Color32 = Color32::from_rgb(0x28, 0x32, 0x46);
/// 无调号时的黑键行背景（与 GpuTheme::pr_black_key_row 一致）。
pub const PR_BLACK_KEY_ROW: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1F);
pub const AR_MEASURE_LINE: Color32 = Color32::from_rgb(0x4D, 0x4D, 0x59); // (0.30,0.30,0.35,1.0)
pub const AR_BEAT_LINE: Color32 = Color32::from_rgb(0x33, 0x33, 0x3B); // (0.20,0.20,0.23,1.0)

// ── Scrollbar colors ──
pub const SCROLLBAR_BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x18);
pub const SCROLLBAR_RECT: Color32 = Color32::from_rgb(0x50, 0x50, 0x58);
pub const SCROLLBAR_HOVER: Color32 = Color32::from_rgb(0x70, 0x70, 0x78);
pub const SCROLLBAR_DRAG: Color32 = Color32::from_rgb(0x90, 0x90, 0x98);

// ── Split handle colors ──
pub const SPLIT_HOVER: Color32 = Color32::from_gray(100);
pub const SPLIT_DEFAULT: Color32 = Color32::from_gray(60);
pub const V_SPLIT_HOVER: Color32 = Color32::from_gray(160);
pub const V_SPLIT_DEFAULT: Color32 = Color32::from_gray(80);

// ── Track button colors ──
pub const MUTE_ACTIVE: Color32 = Color32::from_rgb(240, 200, 60);
pub const SOLO_ACTIVE: Color32 = Color32::from_rgb(220, 80, 80);

// ── Cursor / playhead ──
pub const CURSOR_COLOR: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 204);
pub const CURSOR_WIDTH: f32 = 2.0;
