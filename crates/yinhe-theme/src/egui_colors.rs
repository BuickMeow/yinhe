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

// ════════════════════════════════════════════════════════════════
// 主题系统（阶段 1）：标准色派生。
// 旧常量保留（调用点尚未迁移，见 yinhe-egui theme 运行时层）；
// 迁移完成后旧常量将被删除。
// ════════════════════════════════════════════════════════════════

/// 完整主题：所有派生色。由 [`derive_theme`] 从 7 个标准色计算得出，
/// 用户改标准色即可生成整套主题，无需逐个调色。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    // ── 背景 / 面板 ──
    pub app_bg: Color32,
    pub action_bar_bg: Color32,
    pub ruler_bg: Color32,
    pub scrollbar_bg: Color32,
    pub tab_inactive_bg: Color32,
    pub tab_hover_bg: Color32,
    pub tab_active_bg: Color32,
    // ── 文字灰阶（从 text 标准色派生） ──
    pub text_primary: Color32,
    pub text_bright: Color32,
    pub text_medium: Color32,
    pub text_secondary: Color32,
    pub tab_dirty_dot: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub text_label: Color32,
    pub text_dim: Color32,
    pub text_dimmer: Color32,
    pub text_hint: Color32,
    pub text_label_dim: Color32,
    pub text_disabled: Color32,
    pub mode_bar_text: Color32,
    pub text_selected: Color32,
    pub tooltip_text: Color32,
    // ── 强调 / 选中 / hover ──
    pub accent_active: Color32,
    pub row_selected_bg: Color32,
    pub hover_text: Color32,
    pub marquee_color: Color32,
    pub preview_line: Color32,
    pub row_hover_tint: Color32,
    // ── 按钮 / 边框 ──
    pub btn_bg: Color32,
    pub btn_bg_hover: Color32,
    pub border_dim: Color32,
    // ── 语义色 ──
    pub danger: Color32,
    pub danger_text: Color32,
    pub danger_text_bright: Color32,
    pub error_text: Color32,
    pub danger_hover: Color32,
    pub warning_gold: Color32,
    pub mute_active: Color32,
    pub solo_active: Color32,
    // ── 标尺 / 网格线 ──
    pub ruler_divider: Color32,
    pub measure_label: Color32,
    pub beat_label: Color32,
    pub sub_beat_label: Color32,
    pub tick_label: Color32,
    pub pr_measure_line: Color32,
    pub pr_beat_line: Color32,
    pub pr_sub_beat_line: Color32,
    pub pr_tick_line: Color32,
    pub pr_octave_line: Color32,
    pub pr_scale_outside: Color32,
    pub pr_root_note: Color32,
    pub pr_black_key_row: Color32,
    pub ar_measure_line: Color32,
    pub ar_beat_line: Color32,
    // ── 滚动条 / 分割条 ──
    pub scrollbar_rect: Color32,
    pub scrollbar_hover: Color32,
    pub scrollbar_drag: Color32,
    pub split_hover: Color32,
    pub split_default: Color32,
    pub v_split_hover: Color32,
    pub v_split_default: Color32,
    // ── 光标 / 选框 ──
    pub cursor_color: Color32,
    pub marquee_fill_alpha: f32,
    pub marquee_stroke_alpha: f32,
}

// ── 颜色工具（sRGB 空间近似，够主题派生用） ──

/// 线性插值（sRGB 编码空间近似）。
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied(
        (a.r() as f32 * (1.0 - t) + b.r() as f32 * t) as u8,
        (a.g() as f32 * (1.0 - t) + b.g() as f32 * t) as u8,
        (a.b() as f32 * (1.0 - t) + b.b() as f32 * t) as u8,
        (a.a() as f32 * (1.0 - t) + b.a() as f32 * t) as u8,
    )
}

/// 相对亮度（Rec.601 近似），用于"对比文字/线条"方向判断。
fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// 与背景对比的"高对比"颜色：暗底白字、亮底深字（对比度惯例）。
fn contrast_text(bg: Color32) -> Color32 {
    if luminance(bg) > 0.5 {
        Color32::from_gray(20)
    } else {
        Color32::WHITE
    }
}

/// 保持 alpha 的 RGB 缩放（gamma_multiply 会把 alpha 一起乘，不适合文字灰阶）。
fn shade(c: Color32, f: f32) -> Color32 {
    Color32::from_rgba_premultiplied(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
        c.a(),
    )
}

/// 从 7 个标准色计算完整主题。纯函数：相同输入 → 相同输出。
pub fn derive_theme(base: crate::base::BaseColors) -> Theme {
    let bg = base.bg.to_color32();
    let text = base.text.to_color32();
    let accent = base.accent.to_color32();
    let selection = base.selection.to_color32();
    let danger = base.danger.to_color32();
    let border = base.border.to_color32();
    let warning = base.warning.to_color32();
    let contrast = contrast_text(bg);
    let gray128 = shade(text, 0.58); // text_label

    // 文字灰阶：主文字 × 亮度系数（保 alpha）
    let text_primary = text;
    let text_bright = shade(text, 0.90);
    let text_medium = shade(text, 0.86);
    let text_secondary = shade(text, 0.82);
    let text_muted = shade(text, 0.73);
    let text_faint = shade(text, 0.64);
    let text_label = gray128;
    let text_dim = shade(text, 0.55);
    let text_dimmer = shade(text, 0.50);
    let text_hint = shade(text, 0.45);
    let text_label_dim = shade(text, 0.41);
    let text_disabled = shade(text, 0.36);

    // 面板底色：背景与文字按比例混合（暗主题提亮、亮主题压暗，方向自动正确）
    let mix_tab_inactive = mix(bg, text, 0.05);
    let mix_tab_hover = mix(bg, text, 0.12);
    let mix_tab_active = mix(bg, text, 0.15);
    let mix_btn_bg = mix(bg, text, 0.105);
    let mix_btn_bg_hover = mix(bg, text, 0.23);
    let mix_action_bar = mix(bg, text, 0.03);
    let mix_ruler_divider = mix(bg, text, 0.18);
    let mix_tick_label = mix(bg, text, 0.22);
    let mix_pr_measure = mix(bg, text, 0.33);
    let mix_pr_beat = mix(bg, text, 0.16);
    let mix_pr_sub_beat = mix(bg, text, 0.08);
    let mix_pr_scale_outside = mix(bg, text, 0.015);
    let mix_ar_measure = mix(bg, text, 0.27);
    let mix_ar_beat = mix(bg, text, 0.13);
    let mix_scrollbar_rect = mix(bg, text, 0.28);
    let mix_scrollbar_hover = mix(bg, text, 0.45);
    let mix_scrollbar_drag = mix(bg, text, 0.61);
    let mix_measure_label = shade(text, 0.77);
    let mix_row_hover = mix(bg, text, 0.03);

    // 危险系：危险色与文字/对比色混合得到浅红/暗红档位
    let danger_text = mix(danger, text, 0.28);
    let danger_text_bright = mix(danger, contrast, 0.30);
    let error_text = mix(danger, text, 0.32);
    let danger_hover = mix(danger, gray128, 0.30);
    let warning_gold = mix(warning, text, 0.25);

    let cursor_a = (contrast.a() as f32 * 0.80) as u8;

    Theme {
        app_bg: bg,
        action_bar_bg: mix_action_bar,
        ruler_bg: bg,
        scrollbar_bg: bg,
        tab_inactive_bg: mix_tab_inactive,
        tab_hover_bg: mix_tab_hover,
        tab_active_bg: mix_tab_active,
        text_primary,
        text_bright,
        text_medium,
        text_secondary,
        tab_dirty_dot: text_bright,
        text_muted,
        text_faint,
        text_label,
        text_dim,
        text_dimmer,
        text_hint,
        text_label_dim,
        text_disabled,
        mode_bar_text: text_label,
        text_selected: contrast_text(selection),
        tooltip_text: contrast,
        accent_active: accent,
        row_selected_bg: selection,
        hover_text: contrast,
        marquee_color: contrast,
        preview_line: contrast,
        row_hover_tint: mix_row_hover,
        btn_bg: mix_btn_bg,
        btn_bg_hover: mix_btn_bg_hover,
        border_dim: border,
        danger,
        danger_text,
        danger_text_bright,
        error_text,
        danger_hover,
        warning_gold,
        mute_active: warning,
        solo_active: danger_text,
        ruler_divider: mix_ruler_divider,
        measure_label: mix_measure_label,
        beat_label: text_dim,
        sub_beat_label: text_label_dim,
        tick_label: mix_tick_label,
        pr_measure_line: mix_pr_measure,
        pr_beat_line: mix_pr_beat,
        pr_sub_beat_line: mix_pr_sub_beat,
        pr_tick_line: mix_pr_sub_beat,
        pr_octave_line: mix_pr_beat,
        pr_scale_outside: mix_pr_scale_outside,
        pr_root_note: selection,
        pr_black_key_row: mix_pr_scale_outside,
        ar_measure_line: mix_ar_measure,
        ar_beat_line: mix_ar_beat,
        scrollbar_rect: mix_scrollbar_rect,
        scrollbar_hover: mix_scrollbar_hover,
        scrollbar_drag: mix_scrollbar_drag,
        split_hover: text_hint,
        split_default: border,
        v_split_hover: text_muted,
        v_split_default: text_disabled,
        cursor_color: Color32::from_rgba_premultiplied(
            contrast.r(),
            contrast.g(),
            contrast.b(),
            cursor_a,
        ),
        marquee_fill_alpha: 0.15,
        marquee_stroke_alpha: 0.40,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 派生是纯函数：固定输入 → 固定输出（黄金值，防回归）。
    #[test]
    fn derive_dark_golden_values() {
        let t = derive_theme(crate::base::BaseColors::DARK);
        assert_eq!(t.app_bg, Color32::from_rgb(25, 25, 28));
        assert_eq!(t.text_primary, Color32::from_gray(220));
        assert_eq!(t.text_disabled, Color32::from_gray(79));
        assert_eq!(t.accent_active, Color32::from_rgb(100, 180, 255));
        assert_eq!(t.row_selected_bg, Color32::from_rgb(40, 50, 70));
        assert_eq!(t.danger, Color32::from_rgb(232, 17, 35));
        assert_eq!(t.border_dim, Color32::from_gray(60));
        // hover/选中文字在暗底上应为白
        assert_eq!(t.hover_text, Color32::WHITE);
        assert_eq!(t.text_selected, Color32::WHITE);
    }

    /// 亮色主题：对比色方向、灰阶方向、网格线方向全部正确。
    #[test]
    fn derive_light_directions() {
        let t = derive_theme(crate::base::BaseColors::LIGHT);
        // 亮底上的对比文字应为深色
        assert_eq!(t.hover_text, Color32::from_gray(20));
        assert_eq!(t.text_selected, Color32::from_gray(20));
        // 灰阶比主文字暗
        assert!(t.text_dim.r() < t.text_primary.r());
        // 网格线比背景暗（暗主题是比背景亮）
        assert!(t.pr_measure_line.r() < t.app_bg.r());
        let dark = derive_theme(crate::base::BaseColors::DARK);
        assert!(dark.pr_measure_line.r() > dark.app_bg.r());
    }

    /// Rgba ↔ Color32 往返无损。
    #[test]
    fn rgba_roundtrip() {
        let c = Color32::from_rgb(12, 34, 56);
        assert_eq!(crate::base::Rgba::from_color32(c).to_color32(), c);
    }
}
