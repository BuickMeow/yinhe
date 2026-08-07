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
    action_bar_bg => action_bar_bg,
    ruler_bg => ruler_bg,
    scrollbar_bg => scrollbar_bg,
    tab_inactive_bg => tab_inactive_bg,
    tab_hover_bg => tab_hover_bg,
    tab_active_bg => tab_active_bg,
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
    row_selected_bg => row_selected_bg,
    hover_text => hover_text,
    marquee_color => marquee_color,
    preview_line => preview_line,
    row_hover_tint => row_hover_tint,
    btn_bg => btn_bg,
    btn_bg_hover => btn_bg_hover,
    border_dim => border_dim,
    danger_text => danger_text,
    danger_text_bright => danger_text_bright,
    error_text => error_text,
    danger_hover => danger_hover,
    warning_gold => warning_gold,
    mute_active => mute_active,
    solo_active => solo_active,
    ruler_divider => ruler_divider,
    measure_label => measure_label,
    beat_label => beat_label,
    sub_beat_label => sub_beat_label,
    tick_label => tick_label,
    pr_measure_line => pr_measure_line,
    pr_beat_line => pr_beat_line,
    pr_sub_beat_line => pr_sub_beat_line,
    pr_tick_line => pr_tick_line,
    pr_octave_line => pr_octave_line,
    pr_scale_outside => pr_scale_outside,
    pr_root_note => pr_root_note,
    pr_black_key_row => pr_black_key_row,
    ar_measure_line => ar_measure_line,
    ar_beat_line => ar_beat_line,
    scrollbar_rect => scrollbar_rect,
    scrollbar_hover => scrollbar_hover,
    scrollbar_drag => scrollbar_drag,
    split_hover => split_hover,
    split_default => split_default,
    v_split_hover => v_split_hover,
    v_split_default => v_split_default,
    cursor_color => cursor_color,
    timecode_bg => timecode_bg,
}

pub fn marquee_fill_alpha() -> f32 {
    current().marquee_fill_alpha
}

/// 内容层背景/条纹透明度（PR/AM 背景、PR 色块条带；1.0 = 不透明）。
pub fn content_alpha() -> f32 {
    current().content_alpha
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
