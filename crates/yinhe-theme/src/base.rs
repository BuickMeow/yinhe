//! 主题标准色（用户可调）与派生主题。
//!
//! 设计目标：用户只改 5 个标准色（`BaseColors` 的 bg/text/accent/selection/border），
//! `derive_theme` 纯函数计算全部派生色（文字灰阶 6 档 + 状态/语义色），即可得到
//! 一套完整主题（见 `egui_colors::Theme`）。danger/warning 为固定语义色
//! （`FIXED_DANGER`/`FIXED_WARNING`），不随主题可调。无 egui feature 时只提供
//! 纯数据（`Rgba`/`BaseColors`，供设置持久化）。

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
        selection: Rgba::new(40, 50, 70, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(60, 60, 60, 255),
        warning: FIXED_WARNING,
    };

    /// 亮色主题（浅底深字，中性灰）。
    pub const LIGHT: Self = Self {
        bg: Rgba::new(240, 240, 244, 255),
        text: Rgba::new(30, 30, 34, 255),
        accent: Rgba::new(30, 110, 220, 255),
        selection: Rgba::new(195, 210, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(160, 160, 166, 255),
        warning: FIXED_WARNING,
    };

    /// 亮色·冷灰（GitHub Light 风：冷灰底 + 蓝色强调）。
    pub const LIGHT_COOL: Self = Self {
        bg: Rgba::new(246, 248, 250, 255),
        text: Rgba::new(31, 35, 40, 255),
        accent: Rgba::new(9, 105, 218, 255),
        selection: Rgba::new(188, 212, 246, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(173, 180, 189, 255),
        warning: FIXED_WARNING,
    };

    /// 亮色·暖米（Solarized Light 风：米色底，长时间盯屏眼睛压力小）。
    pub const LIGHT_WARM: Self = Self {
        bg: Rgba::new(250, 244, 230, 255),
        text: Rgba::new(88, 78, 60, 255),
        accent: Rgba::new(38, 119, 210, 255),
        selection: Rgba::new(228, 214, 188, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(186, 175, 150, 255),
        warning: FIXED_WARNING,
    };

    /// Dracula — 高饱和紫粉绿，DJ/电子现场高能量，插件生态最广
    pub const DRACULA: Self = Self {
        bg: Rgba::new(40, 42, 54, 255),
        text: Rgba::new(248, 248, 242, 255),
        accent: Rgba::new(189, 147, 249, 255),
        selection: Rgba::new(68, 71, 90, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(68, 71, 90, 255),
        warning: FIXED_WARNING,
    };

    /// Nord — 北欧极简冷灰蓝，影视配乐/长时间编曲低疲劳
    pub const NORD: Self = Self {
        bg: Rgba::new(46, 52, 64, 255),
        text: Rgba::new(236, 239, 244, 255),
        accent: Rgba::new(136, 192, 208, 255),
        selection: Rgba::new(67, 76, 94, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(76, 86, 106, 255),
        warning: FIXED_WARNING,
    };

    /// Catppuccin Mocha — 柔和 pastel 暗色，开源社区 2026 最受欢迎，长时舒适
    pub const CATPPUCCIN_MOCHA: Self = Self {
        bg: Rgba::new(30, 30, 46, 255),
        text: Rgba::new(205, 214, 244, 255),
        accent: Rgba::new(137, 180, 250, 255),
        selection: Rgba::new(49, 50, 68, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(69, 71, 90, 255),
        warning: FIXED_WARNING,
    };

    /// Tokyo Night — 深蓝霓虹夜景，高对比度，截图与夜间专注
    pub const TOKYO_NIGHT: Self = Self {
        bg: Rgba::new(26, 27, 38, 255),
        text: Rgba::new(192, 202, 245, 255),
        accent: Rgba::new(122, 162, 247, 255),
        selection: Rgba::new(41, 46, 66, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(52, 59, 88, 255),
        warning: FIXED_WARNING,
    };

    /// Gruvbox Dark — 复古暖棕，低眩光，适合长时间敲代码/DJ 打谱
    pub const GRUVBOX_DARK: Self = Self {
        bg: Rgba::new(40, 40, 40, 255),
        text: Rgba::new(235, 219, 178, 255),
        accent: Rgba::new(131, 165, 152, 255),
        selection: Rgba::new(60, 56, 54, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(80, 73, 69, 255),
        warning: FIXED_WARNING,
    };

    /// Everforest Dark — 绿意森林低饱和，护眼，影视/古典编曲温润
    pub const EVERFOREST_DARK: Self = Self {
        bg: Rgba::new(45, 53, 59, 255),
        text: Rgba::new(211, 198, 170, 255),
        accent: Rgba::new(127, 187, 179, 255),
        selection: Rgba::new(61, 72, 77, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(83, 98, 85, 255),
        warning: FIXED_WARNING,
    };

    /// Rose Pine — 自然松枝暮色，Lo-Fi/独立音乐人审美
    pub const ROSE_PINE: Self = Self {
        bg: Rgba::new(25, 23, 36, 255),
        text: Rgba::new(224, 222, 244, 255),
        accent: Rgba::new(235, 111, 146, 255),
        selection: Rgba::new(38, 35, 58, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(64, 61, 82, 255),
        warning: FIXED_WARNING,
    };

    /// One Dark Pro — Atom 经典深靛蓝，均衡对比，通用开发/编曲
    pub const ONE_DARK_PRO: Self = Self {
        bg: Rgba::new(40, 44, 52, 255),
        text: Rgba::new(171, 178, 191, 255),
        accent: Rgba::new(97, 175, 239, 255),
        selection: Rgba::new(62, 68, 81, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(62, 68, 81, 255),
        warning: FIXED_WARNING,
    };

    /// Catppuccin Latte — 亮色 pastel，开源社区配套浅色，强光下可读
    pub const CATPPUCCIN_LATTE: Self = Self {
        bg: Rgba::new(239, 241, 245, 255),
        text: Rgba::new(76, 79, 105, 255),
        accent: Rgba::new(30, 102, 245, 255),
        selection: Rgba::new(204, 208, 218, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(188, 192, 204, 255),
        warning: FIXED_WARNING,
    };

    /// Gruvbox Light — 复古米黄亮色，暖调低眩光，日间/明亮工作室
    pub const GRUVBOX_LIGHT: Self = Self {
        bg: Rgba::new(251, 241, 199, 255),
        text: Rgba::new(60, 56, 54, 255),
        accent: Rgba::new(7, 102, 120, 255),
        selection: Rgba::new(235, 219, 178, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(213, 196, 161, 255),
        warning: FIXED_WARNING,
    };

    /// 内置预设（设置页下拉框）。`None` 表示"自定义"。
    pub const PRESETS: [(&'static str, Self); 14] = [
        ("dark", Self::DARK),
        ("light", Self::LIGHT),
        ("light-cool", Self::LIGHT_COOL),
        ("light-warm", Self::LIGHT_WARM),
        ("dracula", Self::DRACULA),
        ("nord", Self::NORD),
        ("catppuccin-mocha", Self::CATPPUCCIN_MOCHA),
        ("tokyo-night", Self::TOKYO_NIGHT),
        ("gruvbox-dark", Self::GRUVBOX_DARK),
        ("everforest-dark", Self::EVERFOREST_DARK),
        ("rose-pine", Self::ROSE_PINE),
        ("one-dark-pro", Self::ONE_DARK_PRO),
        ("catppuccin-latte", Self::CATPPUCCIN_LATTE),
        ("gruvbox-light", Self::GRUVBOX_LIGHT),
    ];

    pub fn preset_by_name(name: &str) -> Option<Self> {
        Self::PRESETS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
    }
}
