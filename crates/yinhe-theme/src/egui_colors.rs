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
    pub control_bg: Color32,
    pub control_selected_bg: Color32,
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
    pub selected_bg: Color32,
    pub contrast_fg: Color32,
    // ── 按钮 / 边框 ──
    pub btn_bg: Color32,
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
    pub measure_label: Color32,
    pub beat_label: Color32,
    pub sub_beat_label: Color32,
    pub tick_label: Color32,
    pub grid_sub_beat: Color32,
    /// 网格 1tick 最浅档（bg+3%）。
    pub grid_tick: Color32,
    /// 标尺/滚动条轨道底色（bg-20%，最初 RULER_BG 的 (20,20,24)）。
    pub track_bg: Color32,
    pub stripe_bg: Color32,
    /// 线条统一色（egui 原生控件描边、分割条、网格、滑块、八度线；bg+15%）。
    pub line_fg: Color32,
    // ── 光标 / 选框 ──
    pub marquee_fill_alpha: f32,
    pub marquee_stroke_alpha: f32,
    /// 背景偏亮时（浅色主题）为 true（egui Visuals 选 light）。
    pub dark_mode: bool,
}

impl Theme {
    /// 统一悬浮色：暗色主题向主文字混 12%（提亮）；亮色主题向黑混 6%（压暗）。
    pub fn hovered(&self, base: Color32) -> Color32 {
        if self.dark_mode {
            mix(base, self.text_primary, 0.12)
        } else {
            mix(base, Color32::BLACK, 0.06)
        }
    }

    /// 统一按下色：暗色主题向主文字混 24%；亮色主题向黑混 10%。
    pub fn pressed(&self, base: Color32) -> Color32 {
        if self.dark_mode {
            mix(base, self.text_primary, 0.24)
        } else {
            mix(base, Color32::BLACK, 0.10)
        }
    }
}

// ── 颜色工具（sRGB 空间近似，够主题派生用） ──

/// 线性插值（sRGB 编码空间近似）。
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
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
    let dark = luminance(bg) <= 0.5;

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

    // ── 表面层级：两套 token（暗色提亮、亮色压暗，幅度各自感知校准）──
    // 暗色主题在深底上用"提亮"表达层级；亮色主题在浅底上用"压暗"表达层级。
    // 亮色压暗幅度刻意小于暗色提亮幅度：人眼感知绝对色差，亮底上的大色差
    // 比暗底上更刺眼（成熟主题系统两套 token 的做法，方向各自独立）。
    let mix_surface = |t: f32| {
        if dark {
            mix(bg, text, t)
        } else {
            mix(bg, Color32::BLACK, t)
        }
    };
    let mix_darken = |t: f32| mix(bg, Color32::BLACK, t);
    // (control, control_selected, btn, line, sub_beat, tick, track, stripe)
    let (t_control, t_ctl_sel, t_btn, t_line, t_sub, t_tick, t_track, t_stripe) = if dark {
        (0.05, 0.15, 0.10, 0.15, 0.08, 0.03, 0.20, 0.15)
    } else {
        (0.03, 0.08, 0.05, 0.07, 0.04, 0.02, 0.12, 0.06)
    };
    let mix_control = mix_surface(t_control);
    let mix_control_selected = mix_surface(t_ctl_sel);
    let mix_btn_bg = mix_surface(t_btn);
    let mix_line = mix_surface(t_line);
    let mix_grid_sub_beat = mix_surface(t_sub);
    let mix_grid_tick = mix_surface(t_tick);
    // 条纹/轨道恒为"比背景更黑"（亮暗都压暗，幅度分表）
    let mix_track = mix_darken(t_track);
    let mix_stripe = mix_darken(t_stripe);
    let mix_tick_label = mix(bg, text, 0.22);
    let mix_measure_label = shade(text, 0.77);

    // 危险系：危险色与文字/对比色混合得到浅红/暗红档位（同色相）
    let danger_text = mix(danger, text, 0.28);
    let danger_text_bright = mix(danger, contrast, 0.30);
    let error_text = mix(danger, text, 0.32);
    let danger_hover = mix(danger, gray128, 0.30);
    let warning_gold = mix(warning, text, 0.25);

    // 色系设计：选中底 = 强调色混背景（暗主题深蓝、亮主题中浅蓝，同色相）
    let selected_bg = mix(bg, accent, 0.30);

    Theme {
        app_bg: bg,
        control_bg: mix_control,
        control_selected_bg: mix_control_selected,
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
        text_selected: contrast_text(selected_bg),
        tooltip_text: contrast,
        accent_active: accent,
        selected_bg,
        contrast_fg: contrast,
        btn_bg: mix_btn_bg,
        danger,
        danger_text,
        danger_text_bright,
        error_text,
        danger_hover,
        warning_gold,
        mute_active: warning,
        solo_active: danger_text,
        measure_label: mix_measure_label,
        beat_label: text_dim,
        sub_beat_label: text_label_dim,
        tick_label: mix_tick_label,
        grid_sub_beat: mix_grid_sub_beat,
        grid_tick: mix_grid_tick,
        track_bg: mix_track,
        stripe_bg: mix_stripe,
        line_fg: mix_line,
        marquee_fill_alpha: 0.15,
        marquee_stroke_alpha: 0.40,
        // 与 contrast_text 同一把尺子：背景偏亮 → 亮基底（egui Visuals::light）
        dark_mode: dark,
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
        assert_eq!(t.selected_bg, Color32::from_rgb(47, 71, 96));
        assert_eq!(t.danger, Color32::from_rgb(232, 17, 35));
        // 线条统一色 = 背景混主文字 15%：25 + (220-25)×0.15 ≈ 54
        assert_eq!(t.line_fg, Color32::from_rgb(54, 54, 56));
        // hover/选中文字在暗底上应为白
        assert_eq!(t.contrast_fg, Color32::WHITE);
        assert_eq!(t.text_selected, Color32::WHITE);
        assert!(t.dark_mode);
    }

    /// 亮色主题：对比色方向、灰阶方向、网格线方向全部正确。
    #[test]
    fn derive_light_directions() {
        let t = derive_theme(crate::base::BaseColors::LIGHT);
        assert!(!t.dark_mode);
        // 亮底上的对比文字应为深色
        assert_eq!(t.contrast_fg, Color32::from_gray(20));
        assert_eq!(t.text_selected, Color32::from_gray(20));
        // 灰阶比主文字暗
        assert!(t.text_dim.r() < t.text_primary.r());
        // 线条色比背景亮/暗方向随主题（亮色主题线条比背景暗）
        assert!(t.line_fg.r() < t.app_bg.r());
        let dark = derive_theme(crate::base::BaseColors::DARK);
        assert!(dark.line_fg.r() > dark.app_bg.r());
    }

    /// 两套 token：亮/暗各自的表面层级阶梯单调（无交叉、无反转）。
    #[test]
    fn derive_surface_ladder_monotonic() {
        let lum = |c: Color32| c.r() as i32; // 无彩色系下 r 即亮度
        let dark = derive_theme(crate::base::BaseColors::DARK);
        // 暗色：压暗两档 < 基底 < 提亮梯度（3/5/8/10/15% 单调）
        let d = [
            dark.track_bg,
            dark.stripe_bg,
            dark.app_bg,
            dark.grid_tick,
            dark.control_bg,
            dark.grid_sub_beat,
            dark.btn_bg,
            dark.line_fg,
        ];
        assert!(d.windows(2).all(|w| lum(w[0]) <= lum(w[1])));
        assert_eq!(dark.control_selected_bg, dark.line_fg);

        let light = derive_theme(crate::base::BaseColors::LIGHT);
        // 亮色：全部压暗，幅度阶梯单调（2/3/4/5/6/7/8/12% 单调）
        let l = [
            light.track_bg,
            light.control_selected_bg,
            light.line_fg,
            light.stripe_bg,
            light.btn_bg,
            light.grid_sub_beat,
            light.control_bg,
            light.grid_tick,
            light.app_bg,
        ];
        assert!(l.windows(2).all(|w| lum(w[0]) <= lum(w[1])));
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
            assert_eq!(t.contrast_fg, Color32::from_gray(20));
            assert!(t.line_fg.r() < t.app_bg.r());
        }
    }

    /// Rgba ↔ Color32 往返无损。
    #[test]
    fn rgba_roundtrip() {
        let c = Color32::from_rgb(12, 34, 56);
        assert_eq!(crate::base::Rgba::from_color32(c).to_color32(), c);
    }
}
