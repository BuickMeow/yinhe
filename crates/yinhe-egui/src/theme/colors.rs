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
    raised_bg => raised_bg,
    control_bg => control_bg,
    control_selected_bg => control_selected_bg,
    text_primary => text_primary,
    text_bright => text_bright,
    text_medium => text_medium,
    text_secondary => text_secondary,
    tab_dirty_dot => tab_dirty_dot,
    text_muted => text_muted,
    text_faint => text_faint,
    text_label => text_label,
    text_dim => text_dim,
    text_dimmer => text_dimmer,
    text_hint => text_hint,
    text_label_dim => text_label_dim,
    text_disabled => text_disabled,
    mode_bar_text => mode_bar_text,
    text_selected => text_selected,
    tooltip_text => tooltip_text,
    accent_active => accent_active,
    selected_bg => selected_bg,
    contrast_fg => contrast_fg,
    btn_bg => btn_bg,
    border_dim => border_dim,
    danger_text => danger_text,
    danger_text_bright => danger_text_bright,
    error_text => error_text,
    danger_hover => danger_hover,
    warning_gold => warning_gold,
    mute_active => mute_active,
    solo_active => solo_active,
    measure_label => measure_label,
    beat_label => beat_label,
    sub_beat_label => sub_beat_label,
    tick_label => tick_label,
    grid_measure => grid_measure,
    grid_sub_beat => grid_sub_beat,
    stripe_bg => stripe_bg,
    thumb_bg => thumb_bg,
    line_fg => line_fg,
    cursor_fg => cursor_fg,
    inset_bg => inset_bg,
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

/// 危险色（关闭按钮/窗口按钮 hover 等）。
/// macOS 无窗口按钮，getter 在该平台为死代码。
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn danger() -> egui::Color32 {
    current().danger
}
