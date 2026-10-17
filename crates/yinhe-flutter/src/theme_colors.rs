//! LIGHT 主题派生色（与 `yinhe-theme::base::BaseColors::LIGHT` 同步）。
//!
//! 派生规则见 `yinhe-theme/src/egui_colors.rs::derive_theme`（浅色 token）。
//! wgpu 装饰层（网格/键盘/轨道面板）与 Dart 侧 UI 使用同一套色值；
//! 后续接入主题配置时，这里应改为从 `yinhe_theme` 读取。

/// 应用背景（LIGHT：240,240,244），装饰层底矩形与无文档清屏用。
pub const BG: [f32; 4] = [240.0 / 255.0, 240.0 / 255.0, 244.0 / 255.0, 1.0];

/// 网格/分隔线基色：mix(bg, black, 0.07)。
pub const LINE_FG: [f32; 4] = [223.0 / 255.0, 223.0 / 255.0, 227.0 / 255.0, 1.0];

/// AR 交替行底色：mix(bg, black, 0.06)。
pub const STRIPE_BG: [f32; 4] = [226.0 / 255.0, 226.0 / 255.0, 229.0 / 255.0, 1.0];

/// 键盘白键（浅色 tone 0.97）。
pub const KEY_WHITE: [f32; 4] = [247.0 / 255.0, 247.0 / 255.0, 247.0 / 255.0, 1.0];
/// 键盘黑键（浅色 tone 0.20）。
pub const KEY_BLACK: [f32; 4] = [51.0 / 255.0, 51.0 / 255.0, 51.0 / 255.0, 1.0];

/// 主文字色（LIGHT：30,30,34）。
pub const TEXT_PRIMARY: [f32; 4] = [30.0 / 255.0, 30.0 / 255.0, 34.0 / 255.0, 1.0];

/// 网格四档（line_fg 的透明度变体，与桌面 grid_lines 一致）。
pub const GRID_MEASURE: [f32; 4] = LINE_FG;
pub const GRID_BEAT: [f32; 4] = [LINE_FG[0], LINE_FG[1], LINE_FG[2], 140.0 / 255.0];
pub const GRID_SUB: [f32; 4] = [LINE_FG[0], LINE_FG[1], LINE_FG[2], 110.0 / 255.0];
pub const GRID_TICK: [f32; 4] = [LINE_FG[0], LINE_FG[1], LINE_FG[2], 60.0 / 255.0];

/// 线/标签共用的最小像素间距（与桌面 grid_lines 一致）。
pub const GRID_MIN_SPACING: f32 = 38.0;
