//! 当前 GPU 主题派生色（跟随 `yinhe_theme::current_gpu_theme`）。
//!
//! wgpu 侧（折叠自动化底色、无文档清屏）与 Dart 侧 UI 需要同一套色值；
//! 以前这里写死 LIGHT，切深色后 Conductor 轨道色带仍然是浅色文字色。

use yinhe_theme::base::{BaseColors, Rgba};

fn base() -> BaseColors {
    if yinhe_theme::current_gpu_theme().dark_mode {
        BaseColors::DARK
    } else {
        BaseColors::LIGHT
    }
}

fn rgba(c: Rgba) -> [f32; 4] {
    [
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    ]
}

/// 应用背景（装饰层底矩形与无文档清屏用）。
pub fn bg() -> [f32; 4] {
    rgba(base().bg)
}

/// AR 交替行底色：亮色向黑混 6%，暗色向白混 5%。
pub fn stripe_bg() -> [f32; 4] {
    let b = base();
    let c = if b.is_dark() {
        b.bg.mix(Rgba::new(255, 255, 255, 255), 0.05)
    } else {
        b.bg.mix(Rgba::new(0, 0, 0, 255), 0.06)
    };
    rgba(c)
}

/// 主文字色：Conductor（000）轨道色带与文本色一致。
pub fn text_primary() -> [f32; 4] {
    rgba(base().text)
}
