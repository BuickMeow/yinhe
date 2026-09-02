use std::sync::{OnceLock, RwLock};

use yinhe_theme::base::BaseColors;
use yinhe_theme::egui_colors::Theme;

pub use yinhe_theme::egui_colors::{derive_theme, rgb_to_color32, rgba_to_color32};

/// 当前应用主题（阶段 2 运行时层）。启动/设置页通过 [`set_theme`] 更新。
static CURRENT: OnceLock<RwLock<Theme>> = OnceLock::new();

/// 切换主题：重新派生全部颜色，下一帧生效。
pub fn set_theme(base: BaseColors) {
    let theme = derive_theme(base);
    // GPU 渲染主题同步派生（钢琴卷帘/走带纹理同源换色）
    yinhe_theme::set_current_gpu_theme(yinhe_theme::GpuTheme::from_base(base));
    match CURRENT.get() {
        Some(lock) => {
            let mut guard = lock.write().unwrap_or_else(|e| e.into_inner());
            *guard = theme;
        }
        None => {
            let _ = CURRENT.set(RwLock::new(theme));
        }
    }
}

/// 当前主题（未初始化时按默认暗色派生，避免 panic）。
pub fn current() -> Theme {
    match CURRENT.get() {
        Some(lock) => lock
            .read()
            .map(|t| *t)
            .unwrap_or_else(|_| derive_theme(BaseColors::DARK)),
        None => derive_theme(BaseColors::DARK),
    }
}

/// 为每个 Theme 颜色字段生成 `pub fn field() -> Color32` getter，
/// 调用点保持 `crate::theme::xxx()` 风格，主题切换后所有读取自动跟随。
macro_rules! theme_getters {
    ($($getter:ident => $field:ident),* $(,)?) => {
        $(
            pub fn $getter() -> egui::Color32 {
                current().$field
            }
        )*
    };
}

theme_getters! {
    app_bg => app_bg,
    control_bg => control_bg,
    control_selected_bg => control_selected_bg,
    text_primary => text_primary,
    text_bright => text_bright,
    text_secondary => text_secondary,
    tab_dirty_dot => tab_dirty_dot,
    text_muted => text_muted,
    text_label => text_label,
    text_disabled => text_disabled,
    mode_bar_text => mode_bar_text,
    accent_active => accent_active,
    selected_bg => selected_bg,
    contrast_fg => contrast_fg,
    btn_bg => btn_bg,
    danger_text => danger_text,
    danger_text_bright => danger_text_bright,
    danger_hover => danger_hover,
    warning_gold => warning_gold,
    mute_active => mute_active,
    solo_active => solo_active,
    measure_label => measure_label,
    tick_label => tick_label,
    grid_sub_beat => grid_sub_beat,
    grid_tick => grid_tick,
    track_bg => track_bg,
    stripe_bg => stripe_bg,
    line_fg => line_fg,
}

pub fn marquee_fill_alpha() -> f32 {
    current().marquee_fill_alpha
}

/// 统一悬浮色：基色向主文字混入 12%（暗色主题变亮、亮色主题变暗）。
pub fn hover_color(base: egui::Color32) -> egui::Color32 {
    current().hovered(base)
}

/// 统一按下色：基色向主文字混入 24%（比悬浮更重一档）。
pub fn pressed_color(base: egui::Color32) -> egui::Color32 {
    current().pressed(base)
}

/// 当前主题是否暗基底（egui 原生控件 Visuals 选型用）。
pub fn dark_mode() -> bool {
    current().dark_mode
}

pub fn marquee_stroke_alpha() -> f32 {
    current().marquee_stroke_alpha
}

/// Conductor 曲线/轨道指示色：主文字色系（跟随主题 `text_primary`，而非固定黑白）。
pub fn conductor_color() -> egui::Color32 {
    text_primary()
}

pub fn conductor_color_f32() -> [f32; 4] {
    let c = conductor_color();
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// 危险色（关闭按钮/窗口按钮 hover 等）。
/// macOS 无窗口按钮，getter 在该平台为死代码。
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn danger() -> egui::Color32 {
    current().danger
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_theme::base::BaseColors;

    /// 回归：Conductor 必须使用主文字色系（text_primary），而非固定白/黑
    #[test]
    fn conductor_equals_text_primary() {
        for (name, base) in BaseColors::PRESETS {
            set_theme(base);
            assert_eq!(
                conductor_color(),
                text_primary(),
                "Conductor 应等于 text_primary，preset={name}"
            );
            assert_eq!(conductor_color_f32()[3], 1.0);
        }
        // 恢复默认暗色，避免污染后续测试
        set_theme(BaseColors::DARK);
    }

    /// 新增预设全部派生成功且 dark_mode 正确（100 套，含热门移植）
    #[test]
    fn all_presets_have_consistent_dark_mode() {
        for (name, base) in BaseColors::PRESETS {
            let t = derive_theme(base);
            assert_eq!(
                t.dark_mode,
                base.is_dark(),
                "preset {name} dark_mode 与 bg 亮度不一致"
            );
            // 所有主题文字与背景必须可区分
            assert_ne!(t.text_primary, t.app_bg, "preset {name}");
        }
        set_theme(BaseColors::DARK);
    }
}
