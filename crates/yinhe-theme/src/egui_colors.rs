//! Egui-facing theme: 全部派生色 + `derive_theme`（从 `BaseColors` 标准色计算）。
//! Only available when the `egui` feature is enabled.

#[cfg(feature = "egui")]
use egui::Color32;

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
    /// 时间码等"信息凹槽"底色（背景深一档，明暗主题方向自动正确）。
    pub timecode_bg: Color32,
    /// 背景偏亮时（浅色主题）为 true（egui Visuals 选 light）。
    pub dark_mode: bool,
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
    let danger = base.danger.to_color32();
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

    // 危险系：危险色与文字/对比色混合得到浅红/暗红档位（同色相）
    let danger_text = mix(danger, text, 0.28);
    let danger_text_bright = mix(danger, contrast, 0.30);
    let error_text = mix(danger, text, 0.32);
    let danger_hover = mix(danger, gray128, 0.30);
    let warning_gold = mix(warning, text, 0.25);

    // 色系设计：选中底 = 强调色混背景（暗主题深蓝、亮主题中浅蓝，同色相）；
    // 边框/分割线 = 主文字暗化（与文字同色系）
    let row_selected_bg = mix(bg, accent, 0.30);
    let border_dim = shade(text, 0.27);
    // 时间码凹槽：背景深一档（暗主题更深、亮主题浅灰，方向自动正确）
    let timecode_bg = shade(bg, 0.7);

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
        text_selected: contrast_text(row_selected_bg),
        tooltip_text: contrast,
        accent_active: accent,
        row_selected_bg,
        hover_text: contrast,
        marquee_color: contrast,
        preview_line: contrast,
        row_hover_tint: mix_row_hover,
        btn_bg: mix_btn_bg,
        btn_bg_hover: mix_btn_bg_hover,
        border_dim,
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
        pr_root_note: row_selected_bg,
        pr_black_key_row: mix_pr_scale_outside,
        ar_measure_line: mix_ar_measure,
        ar_beat_line: mix_ar_beat,
        scrollbar_rect: mix_scrollbar_rect,
        scrollbar_hover: mix_scrollbar_hover,
        scrollbar_drag: mix_scrollbar_drag,
        split_hover: text_hint,
        split_default: border_dim,
        v_split_hover: text_muted,
        v_split_default: border_dim,
        cursor_color: Color32::from_rgba_premultiplied(
            contrast.r(),
            contrast.g(),
            contrast.b(),
            cursor_a,
        ),
        marquee_fill_alpha: 0.15,
        marquee_stroke_alpha: 0.40,
        timecode_bg,
        // 与 contrast_text 同一把尺子：背景偏亮 → 亮基底（egui Visuals::light）
        dark_mode: luminance(bg) <= 0.5,
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
        // 选中底 = 背景混强调色（同色相）：25,25,28 混 (100,180,255) 30%
        assert_eq!(t.row_selected_bg, Color32::from_rgb(47, 71, 96));
        assert_eq!(t.danger, Color32::from_rgb(232, 17, 35));
        // 边框 = 主文字暗化（同色系）：220×0.27
        assert_eq!(t.border_dim, Color32::from_gray(59));
        // 时间码凹槽 = 背景深一档：25,25,28 × 0.7
        assert_eq!(t.timecode_bg, Color32::from_rgb(17, 17, 19));
        // hover/选中文字在暗底上应为白
        assert_eq!(t.hover_text, Color32::WHITE);
        assert_eq!(t.text_selected, Color32::WHITE);
        assert!(t.dark_mode);
    }

    /// 亮色主题：对比色方向、灰阶方向、网格线方向全部正确。
    #[test]
    fn derive_light_directions() {
        let t = derive_theme(crate::base::BaseColors::LIGHT);
        assert!(!t.dark_mode);
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

    /// 新增的亮色预设：亮基底、深对比字、网格线压暗方向全部正确。
    #[test]
    fn derive_light_presets_directions() {
        for base in [
            crate::base::BaseColors::LIGHT_COOL,
            crate::base::BaseColors::LIGHT_WARM,
        ] {
            let t = derive_theme(base);
            assert!(!t.dark_mode);
            assert_eq!(t.hover_text, Color32::from_gray(20));
            assert!(t.pr_measure_line.r() < t.app_bg.r());
        }
    }

    /// Rgba ↔ Color32 往返无损。
    #[test]
    fn rgba_roundtrip() {
        let c = Color32::from_rgb(12, 34, 56);
        assert_eq!(crate::base::Rgba::from_color32(c).to_color32(), c);
    }
}
