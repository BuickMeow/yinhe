//! DPI 与字体大小的独立缩放
//!
//! - DPI (`ui_scale`) 通过 `zoom_factor` 控制整体布局与像素密度，`set_zoom_factor` 会自动
//!   结合 `native_pixels_per_point` 计算 `pixels_per_point`，因此 Retina 高分屏原生 2.0 不会被覆盖。
//! - 字体大小 (`font_scale`) 通过 `Style::text_styles` 独立缩放，仅影响文字，不影响布局。

use eframe::egui;

/// 应用 DPI 缩放（整体 UI），基于 `zoom_factor`。
pub fn apply_ui_scale(ctx: &egui::Context, scale: f32) {
    let s = scale.clamp(0.75, 2.0);
    ctx.set_zoom_factor(s);
}

/// 应用字体缩放，仅缩放 `text_styles`，保留视觉主题。
pub fn apply_font_scale(ctx: &egui::Context, scale: f32) {
    let s = scale.clamp(0.75, 2.0);
    let default = egui::Style::default();
    ctx.all_styles_mut(|style| {
        for (k, v) in style.text_styles.iter_mut() {
            if let Some(base) = default.text_styles.get(k) {
                v.size = base.size * s;
            }
        }
        if let Some(oid) = style.override_font_id.as_mut()
            && let Some(base_body) = default.text_styles.get(&egui::TextStyle::Body)
        {
            oid.size = base_body.size * s;
        }
    });
}
