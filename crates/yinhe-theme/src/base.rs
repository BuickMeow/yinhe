//! 主题标准色（用户可调）与派生主题。
//!
//! 设计目标：用户只改 7 个标准色（`BaseColors`），`derive_theme` 纯函数
//! 计算全部 ~60 个派生色，即可得到一套完整主题（见 `egui_colors::Theme`）。
//! 无 egui feature 时只提供纯数据（`Rgba`/`BaseColors`，供设置持久化）。

use serde::{Deserialize, Serialize};

/// 8-bit RGBA 颜色（非预乘）。不依赖 egui，可跨 crate 持久化。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[cfg(feature = "egui")]
    pub fn to_color32(self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }

    #[cfg(feature = "egui")]
    pub fn from_color32(c: egui::Color32) -> Self {
        // from_rgba_unmultiplied 会把 premultiplied 转回非预乘
        let c = c.to_srgba_unmultiplied();
        Self {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        }
    }
}

/// 用户可调的标准色：一套主题仅由这 5 个颜色决定。
/// 色系设计：每个标准色派生一族同色相颜色——
/// accent → 选中底/激活；text → 文字灰阶/边框/分割线；
/// danger/warning → 各自语义深浅档。
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct BaseColors {
    /// 应用背景（最深的底色）。
    pub bg: Rgba,
    /// 主文字（灰阶、边框、分割线从这里派生）。
    pub text: Rgba,
    /// 强调色（激活/选中底/插入线等，选中底 = 强调色混背景）。
    pub accent: Rgba,
    /// 危险色（关闭按钮/错误/破坏性操作/橡皮擦）。
    pub danger: Rgba,
    /// 警告/标记金（Mute 激活等）。
    pub warning: Rgba,
}

impl Default for BaseColors {
    fn default() -> Self {
        Self::DARK
    }
}

impl BaseColors {
    /// 默认暗色主题（与主题系统落地前的原始配色一致）。
    pub const DARK: Self = Self {
        bg: Rgba::new(25, 25, 28, 255),
        text: Rgba::new(220, 220, 220, 255),
        accent: Rgba::new(100, 180, 255, 255),
        danger: Rgba::new(232, 17, 35, 255),
        warning: Rgba::new(240, 200, 60, 255),
    };

    /// 亮色主题（浅底深字）。
    pub const LIGHT: Self = Self {
        bg: Rgba::new(240, 240, 244, 255),
        text: Rgba::new(30, 30, 34, 255),
        accent: Rgba::new(30, 110, 220, 255),
        danger: Rgba::new(200, 30, 40, 255),
        warning: Rgba::new(200, 150, 20, 255),
    };

    /// 亮色主题·冷色（浅蓝白底）。
    pub const LIGHT_COOL: Self = Self {
        bg: Rgba::new(232, 240, 250, 255),
        text: Rgba::new(30, 40, 55, 255),
        accent: Rgba::new(25, 100, 200, 255),
        danger: Rgba::new(205, 35, 45, 255),
        warning: Rgba::new(195, 150, 25, 255),
    };

    /// 亮色主题·暖色（浅米白底）。
    pub const LIGHT_WARM: Self = Self {
        bg: Rgba::new(250, 244, 234, 255),
        text: Rgba::new(55, 45, 35, 255),
        accent: Rgba::new(180, 90, 30, 255),
        danger: Rgba::new(205, 50, 40, 255),
        warning: Rgba::new(180, 140, 30, 255),
    };

    /// 内置预设（设置页下拉框）。`None` 表示"自定义"。
    pub const PRESETS: [(&'static str, Self); 4] = [
        ("dark", Self::DARK),
        ("light", Self::LIGHT),
        ("light-cool", Self::LIGHT_COOL),
        ("light-warm", Self::LIGHT_WARM),
    ];

    pub fn preset_by_name(name: &str) -> Option<Self> {
        Self::PRESETS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
    }
}
