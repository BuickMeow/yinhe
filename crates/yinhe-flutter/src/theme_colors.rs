//! LIGHT 主题派生色（与 `yinhe-theme::base::BaseColors::LIGHT` 同步）。
//!
//! 派生规则见 `yinhe-theme/src/egui_colors.rs::derive_theme`（浅色 token）。
//! wgpu 侧（折叠自动化底色、无文档清屏）与 Dart 侧 UI 使用同一套色值；
//! 后续接入主题配置时，这里应改为从 `yinhe_theme` 读取。

/// 应用背景（LIGHT：240,240,244），装饰层底矩形与无文档清屏用。
pub const BG: [f32; 4] = [240.0 / 255.0, 240.0 / 255.0, 244.0 / 255.0, 1.0];

/// AR 交替行底色：mix(bg, black, 0.06)。
pub const STRIPE_BG: [f32; 4] = [226.0 / 255.0, 226.0 / 255.0, 229.0 / 255.0, 1.0];

/// 主文字色（LIGHT：30,30,34）。
pub const TEXT_PRIMARY: [f32; 4] = [30.0 / 255.0, 30.0 / 255.0, 34.0 / 255.0, 1.0];
