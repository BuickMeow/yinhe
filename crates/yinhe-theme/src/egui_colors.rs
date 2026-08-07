//! Egui-facing color constants.
//! Only available when the `egui` feature is enabled.

#[cfg(feature = "egui")]
use egui::Color32;

// ── App-level background color ──
pub const APP_BG: Color32 = Color32::from_rgb(25, 25, 28);

// ── Active accent color ──
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(100, 180, 255);

// ── 模式栏（mode bar）文字/图标色：讲解行与性能显示（CPU/MEM/FPS）的灰色字 ──
// 与弱标签文字 TEXT_LABEL 同色（gray128）。
pub const MODE_BAR_TEXT: Color32 = TEXT_LABEL;

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
/// 弱标签文字/未激活图标（原散落各处的 `Color32::GRAY`，gray128）。
pub const TEXT_LABEL: Color32 = Color32::from_gray(128);
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
pub const WARNING_GOLD: Color32 = Color32::from_rgb(220, 180, 90); // 金色标记

// ── Event browser 行选中底色（树形导航 / PR 根音行共用） ──
pub const ROW_SELECTED_BG: Color32 = Color32::from_rgb(40, 50, 70);

// ── Tab colors ──
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(55, 55, 60);
pub const TAB_INACTIVE_BG: Color32 = Color32::from_rgb(35, 35, 38);
pub const TAB_HOVER_BG: Color32 = Color32::from_rgb(48, 48, 52);

// ── Close / danger hover ──
pub const DANGER_HOVER: Color32 = Color32::from_rgb(200, 50, 50);

// ── Time ruler colors（背景与文字灰阶同源，为主题系统收敛色源） ──
pub const RULER_BG: Color32 = APP_BG;
pub const RULER_DIVIDER: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3F);
pub const MEASURE_LABEL: Color32 = Color32::from_rgb(0xAA, 0xAA, 0xAF);
pub const BEAT_LABEL: Color32 = TEXT_DIM; // (119,119,124)≈gray120
pub const SUB_BEAT_LABEL: Color32 = TEXT_LABEL_DIM; // (85,85,90)≈gray90
pub const TICK_LABEL: Color32 = Color32::from_rgb(0x44, 0x44, 0x49);

// ── Grid line colors（pianoroll / automation 共用 pr_*，arrangement 用 ar_*）──
// 从 GpuTheme 迁移而来，统一用 Color32。alpha 直接编进 Color32。
pub const PR_MEASURE_LINE: Color32 = Color32::from_rgb(0x59, 0x59, 0x66); // (0.35,0.35,0.40,1.0)
pub const PR_BEAT_LINE: Color32 = Color32::from_rgb(0x38, 0x38, 0x40); // (0.22,0.22,0.25,1.0)
pub const PR_SUB_BEAT_LINE: Color32 = Color32::from_rgb(0x29, 0x29, 0x2E); // (0.16,0.16,0.18,1.0)
pub const PR_TICK_LINE: Color32 = PR_SUB_BEAT_LINE; // (37,37,42)≈次拍线

// ── Piano roll 横向：八度线 / 调号背景 ──
/// 每个八度边界的横向细线（C 位置）。与 `PR_BEAT_LINE` 同值。
pub const PR_OCTAVE_LINE: Color32 = PR_BEAT_LINE;
/// 调外音行背景（与 `PR_BLACK_KEY_ROW` 同色，保证有/无调号时视觉一致）。
pub const PR_SCALE_OUTSIDE: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1F);
/// 根音行背景（与事件列表选中高亮蓝一致：RGB(40,50,70)）。
pub const PR_ROOT_NOTE: Color32 = ROW_SELECTED_BG;
/// 无调号时的黑键行背景（与 GpuTheme::pr_black_key_row 一致）。
pub const PR_BLACK_KEY_ROW: Color32 = PR_SCALE_OUTSIDE;
pub const AR_MEASURE_LINE: Color32 = Color32::from_rgb(0x4D, 0x4D, 0x59); // (0.30,0.30,0.35,1.0)
pub const AR_BEAT_LINE: Color32 = Color32::from_rgb(0x33, 0x33, 0x3B); // (0.20,0.20,0.23,1.0)

// ── Scrollbar colors（背景与标尺同色） ──
pub const SCROLLBAR_BG: Color32 = RULER_BG;
pub const SCROLLBAR_RECT: Color32 = Color32::from_rgb(0x50, 0x50, 0x58);
pub const SCROLLBAR_HOVER: Color32 = Color32::from_rgb(0x70, 0x70, 0x78);
pub const SCROLLBAR_DRAG: Color32 = Color32::from_rgb(0x90, 0x90, 0x98);

// ── Split handle colors（与文字灰阶同源） ──
pub const SPLIT_HOVER: Color32 = TEXT_HINT;
pub const SPLIT_DEFAULT: Color32 = BORDER_DIM;
pub const V_SPLIT_HOVER: Color32 = TEXT_MUTED;
pub const V_SPLIT_DEFAULT: Color32 = TEXT_DISABLED;

// ── Track button colors ──
pub const MUTE_ACTIVE: Color32 = Color32::from_rgb(240, 200, 60);
pub const SOLO_ACTIVE: Color32 = Color32::from_rgb(220, 80, 80);

// ── Cursor / playhead ──
pub const CURSOR_COLOR: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 204);
pub const CURSOR_WIDTH: f32 = 2.0;

// ── Marquee（选框）透明度：三视图（PR/AR/AM）共用 ──
pub const MARQUEE_FILL_ALPHA: f32 = 0.15;
pub const MARQUEE_STROKE_ALPHA: f32 = 0.40;

// ── Hover / 选框 / 行提亮基色（原散落的 WHITE 字面量，为主题系统收编） ──
/// hover 高亮文字/图标色（全项目 hover 变白基准）。
pub const HOVER_TEXT: Color32 = Color32::WHITE;
/// 选框基色（PR/AR/AM 白框，透明度见 MARQUEE_*_ALPHA）。
pub const MARQUEE_COLOR: Color32 = Color32::WHITE;

/// 列表行 hover 提亮（3% 白叠加，音轨面板/音色库/archive 行共用）。
/// gamma_multiply 非 const fn，故用函数而非常量。
pub fn row_hover_tint() -> Color32 {
    Color32::WHITE.gamma_multiply(0.03)
}

/// 选中行文字色（ROW_SELECTED_BG 上的白字）。
pub const TEXT_SELECTED: Color32 = Color32::WHITE;
/// 拖拽 tooltip 文字色（深色浮层上的白字，对比度优先）。
pub const TOOLTIP_TEXT: Color32 = Color32::WHITE;
/// 编辑预览顶部高亮线（如 velocity 笔划预览的新高度线）。
pub const PREVIEW_LINE: Color32 = Color32::WHITE;

/// [f32;3] 元组（0..1，如 GpuTheme 字段）→ Color32（alpha=255）。
/// 替代散落各处的 `Color32::from_rgb((r*255.0) as u8, ...)` 手写转换。
pub fn rgb_to_color32(c: (f32, f32, f32)) -> Color32 {
    Color32::from_rgb(
        (c.0 * 255.0) as u8,
        (c.1 * 255.0) as u8,
        (c.2 * 255.0) as u8,
    )
}

/// [f32;4] 元组（0..1，非预乘 alpha，如 GpuTheme 字段）→ Color32。
pub fn rgba_to_color32(c: (f32, f32, f32, f32)) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c.0 * 255.0) as u8,
        (c.1 * 255.0) as u8,
        (c.2 * 255.0) as u8,
        (c.3 * 255.0) as u8,
    )
}
