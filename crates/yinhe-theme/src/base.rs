//! 主题标准色（用户可调）与派生主题。
//!
//! 设计目标：用户只改 5 个标准色（`BaseColors` 的 bg/text/accent/selection/border），
//! `derive_theme` 纯函数计算全部派生色（文字灰阶 6 档 + 状态/语义色），即可得到
//! 一套完整主题（见 `egui_colors::Theme`）。danger/warning 为固定语义色
//! （`FIXED_DANGER`/`FIXED_WARNING`），不随主题可调。无 egui feature 时只提供
//! 纯数据（`Rgba`/`BaseColors`，供设置持久化）。

mod presets;

use serde::{Deserialize, Serialize};

/// 8-bit RGBA 颜色（非预乘）。不依赖 egui，可跨 crate 持久化。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// 固定语义色（不随主题可调）：危险红 / 警告金。
pub const FIXED_DANGER: Rgba = Rgba::new(232, 17, 35, 255);
pub const FIXED_WARNING: Rgba = Rgba::new(240, 200, 60, 255);

fn default_danger() -> Rgba {
    FIXED_DANGER
}
fn default_warning() -> Rgba {
    FIXED_WARNING
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// sRGB 线性插值（与 egui_colors::mix 同步，纯数据层可用）。
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: (self.r as f32 * (1.0 - t) + other.r as f32 * t) as u8,
            g: (self.g as f32 * (1.0 - t) + other.g as f32 * t) as u8,
            b: (self.b as f32 * (1.0 - t) + other.b as f32 * t) as u8,
            a: (self.a as f32 * (1.0 - t) + other.a as f32 * t) as u8,
        }
    }

    /// 相对亮度（Rec.601），与 egui 侧 derive_theme 同一把尺子。
    pub fn luminance(self) -> f32 {
        (0.299 * self.r as f32 + 0.587 * self.g as f32 + 0.114 * self.b as f32) / 255.0
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

/// 用户可调的标准色 + 固定语义色。
/// 可调：bg / text / accent / selection / border 共 5 个；
/// danger / warning 为固定语义色（`FIXED_DANGER`/`FIXED_WARNING`），
/// 保留在结构中仅为兼容旧配置反序列化，派生时不再读取。
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct BaseColors {
    /// 应用背景（最深的底色）。
    pub bg: Rgba,
    /// 主文字（灰阶从这里派生：乘以亮度系数得到从亮到暗的各级文字）。
    pub text: Rgba,
    /// 强调色（激活/选中高亮/插入线等）。
    pub accent: Rgba,
    /// 选中底色（列表行选中、根音行）。
    pub selection: Rgba,
    /// 危险色（已固定为 `FIXED_DANGER`，仅兼容旧配置）。
    #[serde(default = "default_danger")]
    pub danger: Rgba,
    /// 边框/分隔线。
    pub border: Rgba,
    /// 警告/标记金（已固定为 `FIXED_WARNING`，仅兼容旧配置）。
    #[serde(default = "default_warning")]
    pub warning: Rgba,
}

impl BaseColors {
    /// 当前背景是否为暗色（与 `derive_theme` 的 `dark_mode` 同尺子：bg 亮度 ≤0.5）。
    pub fn is_dark(&self) -> bool {
        self.bg.luminance() <= 0.5
    }

    /// 互换背景与文字以得到对向明暗方案（每家族双 accent：强调色随目标明暗微调）。
    ///
    /// - bg ↔ text 互换，selection/border 保留（派生侧已忽略，旧配置兼容）
    /// - accent 保持色相但按目标明暗各调一档：浅底时向黑混 18%（压暗保证对比），
    ///   深底时向白混 14%（提亮），与 `derive_theme` 的 light/dark 两套 token 同向
    pub fn inverted(&self) -> Self {
        let new_bg = self.text;
        let new_text = self.bg;
        let new_is_dark = new_bg.luminance() <= 0.5;
        let new_accent = if new_is_dark {
            // 深底：强调色提亮 14%（暗色主题中浅色 accent 更通透）
            self.accent.mix(Rgba::new(255, 255, 255, 255), 0.14)
        } else {
            // 浅底：强调色压暗 18%（浅色主题中深色 accent 对比更强）
            self.accent.mix(Rgba::new(0, 0, 0, 255), 0.18)
        };
        Self {
            bg: new_bg,
            text: new_text,
            accent: new_accent,
            selection: self.selection,
            danger: self.danger,
            border: self.border,
            warning: self.warning,
        }
    }

    pub fn preset_by_name(name: &str) -> Option<Self> {
        Self::PRESETS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
    }
}

impl Default for BaseColors {
    fn default() -> Self {
        Self::DARK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverted_swaps_bg_text_and_flips_dark() {
        for (_, base) in BaseColors::PRESETS {
            let inv = base.inverted();
            assert_eq!(inv.bg, base.text, "bg should swap");
            assert_eq!(inv.text, base.bg, "text should swap");
            assert_ne!(
                inv.is_dark(),
                base.is_dark(),
                "dark should flip for {base:?}"
            );
            // 强调色保持色相但明暗各调一档：新旧 accent 不等但同系
            assert_ne!(inv.accent, base.accent);
            // 二次互换回到原背景/文字（强调色因 14%/18% 非对称会有漂移，允许）
            let inv2 = inv.inverted();
            assert_eq!(inv2.bg, base.bg);
            assert_eq!(inv2.text, base.text);
        }
    }

    #[test]
    fn inverted_accent_contrast_direction() {
        // 暗底→浅底：强调色应压暗；浅底→暗底：强调色应提亮
        let dark = BaseColors::DARK;
        let inv_light = dark.inverted();
        assert!(!inv_light.is_dark());
        assert!(inv_light.accent.luminance() < dark.accent.luminance());
        let light = BaseColors::LIGHT;
        let inv_dark = light.inverted();
        assert!(inv_dark.is_dark());
        assert!(inv_dark.accent.luminance() > light.accent.luminance());
    }
}
