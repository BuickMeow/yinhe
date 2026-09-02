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

    /// 霄绢 — 纸张质感+鼠尾草绿，开源护眼经典（Solarized/Everforest 纸色 #fdf6e3 + 柔和对比）
    pub const XIAO_JUAN: Self = Self {
        bg: Rgba::new(253, 246, 227, 255),
        text: Rgba::new(92, 106, 114, 255),
        accent: Rgba::new(141, 161, 1, 255),
        selection: Rgba::new(238, 232, 213, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(213, 196, 161, 255),
        warning: FIXED_WARNING,
    };

    /// 星砚 — 宣纸暖白+青绿，开源豆沙绿护眼（#f5f0e1 纸 + #76946a 绿，低蓝光）
    pub const XING_YAN: Self = Self {
        bg: Rgba::new(245, 240, 225, 255),
        text: Rgba::new(61, 72, 66, 255),
        accent: Rgba::new(118, 148, 106, 255),
        selection: Rgba::new(232, 225, 203, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(210, 200, 180, 255),
        warning: FIXED_WARNING,
    };

    /// 月渚 — 月光纸+苔绿，Gruvbox/PaperColor 柔和纸感（#f2eee7 + 低饱和绿）
    pub const YUE_ZHU: Self = Self {
        bg: Rgba::new(242, 238, 231, 255),
        text: Rgba::new(67, 64, 61, 255),
        accent: Rgba::new(122, 158, 126, 255),
        selection: Rgba::new(228, 222, 213, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(210, 200, 190, 255),
        warning: FIXED_WARNING,
    };

    /// 秋毫 — 暮晓宣纸+松绿，Rose Pine Dawn 暖调（#faf4ed + #56949f→绿调）
    pub const QIU_HAO: Self = Self {
        bg: Rgba::new(250, 244, 237, 255),
        text: Rgba::new(87, 82, 121, 255),
        accent: Rgba::new(86, 148, 122, 255),
        selection: Rgba::new(237, 232, 220, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(220, 212, 200, 255),
        warning: FIXED_WARNING,
    };

    /// Plum Rain — 梅雨，热门主题移植，花草诗意中性名
    pub const PLUM_RAIN: Self = Self {
        bg: Rgba::new(0, 43, 54, 255),
        text: Rgba::new(131, 148, 150, 255),
        accent: Rgba::new(38, 139, 210, 255),
        selection: Rgba::new(0, 43, 54, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(0, 43, 54, 255),
        warning: FIXED_WARNING,
    };

    /// Orchid Valley — 兰谷，热门主题移植，花草诗意中性名
    pub const ORCHID_VALLEY: Self = Self {
        bg: Rgba::new(253, 246, 227, 255),
        text: Rgba::new(101, 123, 131, 255),
        accent: Rgba::new(38, 139, 210, 255),
        selection: Rgba::new(253, 246, 227, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(253, 246, 227, 255),
        warning: FIXED_WARNING,
    };

    /// Bamboo Fence — 竹篱，热门主题移植，花草诗意中性名
    pub const BAMBOO_FENCE: Self = Self {
        bg: Rgba::new(39, 40, 34, 255),
        text: Rgba::new(248, 248, 242, 255),
        accent: Rgba::new(166, 226, 46, 255),
        selection: Rgba::new(39, 40, 34, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(39, 40, 34, 255),
        warning: FIXED_WARNING,
    };

    /// Chrysanthemum Dew — 菊露，热门主题移植，花草诗意中性名
    pub const CHRYSANTHEMUM_DEW: Self = Self {
        bg: Rgba::new(45, 42, 46, 255),
        text: Rgba::new(252, 252, 250, 255),
        accent: Rgba::new(255, 97, 136, 255),
        selection: Rgba::new(45, 42, 46, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(45, 42, 46, 255),
        warning: FIXED_WARNING,
    };

    /// Pine Wind — 松风，热门主题移植，花草诗意中性名
    pub const PINE_WIND: Self = Self {
        bg: Rgba::new(10, 14, 20, 255),
        text: Rgba::new(191, 186, 176, 255),
        accent: Rgba::new(230, 180, 80, 255),
        selection: Rgba::new(10, 14, 20, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(10, 14, 20, 255),
        warning: FIXED_WARNING,
    };

    /// Willow Bank — 柳岸，热门主题移植，花草诗意中性名
    pub const WILLOW_BANK: Self = Self {
        bg: Rgba::new(250, 250, 250, 255),
        text: Rgba::new(92, 103, 115, 255),
        accent: Rgba::new(134, 179, 0, 255),
        selection: Rgba::new(250, 250, 250, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(250, 250, 250, 255),
        warning: FIXED_WARNING,
    };

    /// Peach Stream — 桃溪，热门主题移植，花草诗意中性名
    pub const PEACH_STREAM: Self = Self {
        bg: Rgba::new(31, 36, 48, 255),
        text: Rgba::new(203, 204, 198, 255),
        accent: Rgba::new(255, 204, 102, 255),
        selection: Rgba::new(31, 36, 48, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(31, 36, 48, 255),
        warning: FIXED_WARNING,
    };

    /// Apricot Cove — 杏坞，热门主题移植，花草诗意中性名
    pub const APRICOT_COVE: Self = Self {
        bg: Rgba::new(1, 22, 39, 255),
        text: Rgba::new(214, 222, 235, 255),
        accent: Rgba::new(130, 170, 255, 255),
        selection: Rgba::new(1, 22, 39, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(1, 22, 39, 255),
        warning: FIXED_WARNING,
    };

    /// Cherry Rain — 樱雨，热门主题移植，花草诗意中性名
    pub const CHERRY_RAIN: Self = Self {
        bg: Rgba::new(240, 240, 240, 255),
        text: Rgba::new(64, 63, 83, 255),
        accent: Rgba::new(153, 76, 195, 255),
        selection: Rgba::new(240, 240, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(240, 240, 240, 255),
        warning: FIXED_WARNING,
    };

    /// Maple Marsh — 枫沼，热门主题移植，花草诗意中性名
    pub const MAPLE_MARSH: Self = Self {
        bg: Rgba::new(45, 43, 85, 255),
        text: Rgba::new(165, 153, 233, 255),
        accent: Rgba::new(250, 208, 0, 255),
        selection: Rgba::new(45, 43, 85, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(45, 43, 85, 255),
        warning: FIXED_WARNING,
    };

    /// Lotus Pond — 荷塘，热门主题移植，花草诗意中性名
    pub const LOTUS_POND: Self = Self {
        bg: Rgba::new(41, 45, 62, 255),
        text: Rgba::new(166, 172, 205, 255),
        accent: Rgba::new(199, 146, 234, 255),
        selection: Rgba::new(41, 45, 62, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(41, 45, 62, 255),
        warning: FIXED_WARNING,
    };

    /// Osmanthus Brew — 桂酿，热门主题移植，花草诗意中性名
    pub const OSMANTHUS_BREW: Self = Self {
        bg: Rgba::new(38, 50, 56, 255),
        text: Rgba::new(238, 255, 255, 255),
        accent: Rgba::new(130, 170, 255, 255),
        selection: Rgba::new(38, 50, 56, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(38, 50, 56, 255),
        warning: FIXED_WARNING,
    };

    /// Paulownia Court — 桐庭，热门主题移植，花草诗意中性名
    pub const PAULOWNIA_COURT: Self = Self {
        bg: Rgba::new(28, 30, 38, 255),
        text: Rgba::new(233, 86, 120, 255),
        accent: Rgba::new(250, 183, 149, 255),
        selection: Rgba::new(28, 30, 38, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(28, 30, 38, 255),
        warning: FIXED_WARNING,
    };

    /// Ginkgo — 银杏，热门主题移植，花草诗意中性名
    pub const GINKGO: Self = Self {
        bg: Rgba::new(35, 38, 46, 255),
        text: Rgba::new(213, 206, 217, 255),
        accent: Rgba::new(0, 232, 198, 255),
        selection: Rgba::new(35, 38, 46, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(35, 38, 46, 255),
        warning: FIXED_WARNING,
    };

    /// Crabapple — 棠梨，热门主题移植，花草诗意中性名
    pub const CRABAPPLE: Self = Self {
        bg: Rgba::new(38, 35, 53, 255),
        text: Rgba::new(255, 255, 255, 255),
        accent: Rgba::new(255, 126, 219, 255),
        selection: Rgba::new(38, 35, 53, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(38, 35, 53, 255),
        warning: FIXED_WARNING,
    };

    /// Vine Grass — 蔓草，热门主题移植，花草诗意中性名
    pub const VINE_GRASS: Self = Self {
        bg: Rgba::new(25, 53, 73, 255),
        text: Rgba::new(255, 255, 255, 255),
        accent: Rgba::new(255, 198, 0, 255),
        selection: Rgba::new(25, 53, 73, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(25, 53, 73, 255),
        warning: FIXED_WARNING,
    };

    /// Duckweed Islet — 萍洲，热门主题移植，花草诗意中性名
    pub const DUCKWEED_ISLET: Self = Self {
        bg: Rgba::new(13, 17, 23, 255),
        text: Rgba::new(201, 209, 217, 255),
        accent: Rgba::new(88, 166, 255, 255),
        selection: Rgba::new(13, 17, 23, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(13, 17, 23, 255),
        warning: FIXED_WARNING,
    };

    /// Algae — 藻荇，热门主题移植，花草诗意中性名
    pub const ALGAE: Self = Self {
        bg: Rgba::new(255, 255, 255, 255),
        text: Rgba::new(36, 41, 46, 255),
        accent: Rgba::new(3, 102, 214, 255),
        selection: Rgba::new(255, 255, 255, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(255, 255, 255, 255),
        warning: FIXED_WARNING,
    };

    /// Water Chestnut Song — 菱歌，热门主题移植，花草诗意中性名
    pub const WATER_CHESTNUT_SONG: Self = Self {
        bg: Rgba::new(250, 250, 250, 255),
        text: Rgba::new(56, 58, 66, 255),
        accent: Rgba::new(64, 120, 242, 255),
        selection: Rgba::new(250, 250, 250, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(250, 250, 250, 255),
        warning: FIXED_WARNING,
    };

    /// Foxnut — 芡实，热门主题移植，花草诗意中性名
    pub const FOXNUT: Self = Self {
        bg: Rgba::new(29, 31, 33, 255),
        text: Rgba::new(197, 200, 198, 255),
        accent: Rgba::new(204, 102, 102, 255),
        selection: Rgba::new(29, 31, 33, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(29, 31, 33, 255),
        warning: FIXED_WARNING,
    };

    /// Wild Rice Pond — 茭塘，热门主题移植，花草诗意中性名
    pub const WILD_RICE_POND: Self = Self {
        bg: Rgba::new(1, 22, 39, 255),
        text: Rgba::new(214, 222, 235, 255),
        accent: Rgba::new(126, 87, 194, 255),
        selection: Rgba::new(1, 22, 39, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(1, 22, 39, 255),
        warning: FIXED_WARNING,
    };

    /// Knotweed Bank — 蓼汀，热门主题移植，花草诗意中性名
    pub const KNOTWEED_BANK: Self = Self {
        bg: Rgba::new(26, 30, 42, 255),
        text: Rgba::new(241, 241, 241, 255),
        accent: Rgba::new(99, 148, 255, 255),
        selection: Rgba::new(26, 30, 42, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(26, 30, 42, 255),
        warning: FIXED_WARNING,
    };

    /// Mint — 薄荷，热门主题移植，花草诗意中性名
    pub const MINT: Self = Self {
        bg: Rgba::new(27, 30, 40, 255),
        text: Rgba::new(228, 240, 251, 255),
        accent: Rgba::new(173, 215, 255, 255),
        selection: Rgba::new(27, 30, 40, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(27, 30, 40, 255),
        warning: FIXED_WARNING,
    };

    /// Bramble Gate — 荆扉，热门主题移植，花草诗意中性名
    pub const BRAMBLE_GATE: Self = Self {
        bg: Rgba::new(21, 20, 27, 255),
        text: Rgba::new(204, 202, 194, 255),
        accent: Rgba::new(162, 119, 255, 255),
        selection: Rgba::new(21, 20, 27, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(21, 20, 27, 255),
        warning: FIXED_WARNING,
    };

    /// Vetch Wall — 薇垣，热门主题移植，花草诗意中性名
    pub const VETCH_WALL: Self = Self {
        bg: Rgba::new(22, 24, 33, 255),
        text: Rgba::new(198, 200, 209, 255),
        accent: Rgba::new(132, 160, 198, 255),
        selection: Rgba::new(22, 24, 33, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(22, 24, 33, 255),
        warning: FIXED_WARNING,
    };

    /// Calamus — 菖蒲，热门主题移植，花草诗意中性名
    pub const CALAMUS: Self = Self {
        bg: Rgba::new(0, 0, 42, 255),
        text: Rgba::new(128, 255, 234, 255),
        accent: Rgba::new(255, 0, 160, 255),
        selection: Rgba::new(0, 0, 42, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(0, 0, 42, 255),
        warning: FIXED_WARNING,
    };

    /// Knotweed Flower — 蓼花，热门主题移植，花草诗意中性名
    pub const KNOTWEED_FLOWER: Self = Self {
        bg: Rgba::new(3, 26, 22, 255),
        text: Rgba::new(129, 181, 166, 255),
        accent: Rgba::new(11, 140, 140, 255),
        selection: Rgba::new(3, 26, 22, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(3, 26, 22, 255),
        warning: FIXED_WARNING,
    };

    /// Angelica Bank — 芷汀，热门主题移植，花草诗意中性名
    pub const ANGELICA_BANK: Self = Self {
        bg: Rgba::new(247, 247, 247, 255),
        text: Rgba::new(74, 69, 67, 255),
        accent: Rgba::new(219, 45, 32, 255),
        selection: Rgba::new(247, 247, 247, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 247, 247, 255),
        warning: FIXED_WARNING,
    };

    /// Asara — 蘅皋，热门主题移植，花草诗意中性名
    pub const ASARA: Self = Self {
        bg: Rgba::new(9, 3, 0, 255),
        text: Rgba::new(165, 162, 162, 255),
        accent: Rgba::new(219, 45, 32, 255),
        selection: Rgba::new(9, 3, 0, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(9, 3, 0, 255),
        warning: FIXED_WARNING,
    };

    /// Azalea — 杜若，热门主题移植，花草诗意中性名
    pub const AZALEA: Self = Self {
        bg: Rgba::new(28, 27, 25, 255),
        text: Rgba::new(252, 232, 195, 255),
        accent: Rgba::new(239, 47, 39, 255),
        selection: Rgba::new(28, 27, 25, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(28, 27, 25, 255),
        warning: FIXED_WARNING,
    };

    /// Cuckoo Cry — 鹃啼，热门主题移植，花草诗意中性名
    pub const CUCKOO_CRY: Self = Self {
        bg: Rgba::new(26, 35, 32, 255),
        text: Rgba::new(212, 220, 219, 255),
        accent: Rgba::new(107, 197, 223, 255),
        selection: Rgba::new(26, 35, 32, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(26, 35, 32, 255),
        warning: FIXED_WARNING,
    };

    /// Violet — 堇色，热门主题移植，花草诗意中性名
    pub const VIOLET: Self = Self {
        bg: Rgba::new(247, 245, 247, 255),
        text: Rgba::new(79, 58, 75, 255),
        accent: Rgba::new(195, 34, 111, 255),
        selection: Rgba::new(247, 245, 247, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 245, 247, 255),
        warning: FIXED_WARNING,
    };

    /// Mallow — 葵藿，热门主题移植，花草诗意中性名
    pub const MALLOW: Self = Self {
        bg: Rgba::new(49, 57, 34, 255),
        text: Rgba::new(237, 238, 235, 255),
        accent: Rgba::new(129, 223, 107, 255),
        selection: Rgba::new(49, 57, 34, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(49, 57, 34, 255),
        warning: FIXED_WARNING,
    };

    /// Hibiscus Fence — 槿篱，热门主题移植，花草诗意中性名
    pub const HIBISCUS_FENCE: Self = Self {
        bg: Rgba::new(236, 237, 240, 255),
        text: Rgba::new(45, 49, 61, 255),
        accent: Rgba::new(51, 34, 195, 255),
        selection: Rgba::new(236, 237, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(236, 237, 240, 255),
        warning: FIXED_WARNING,
    };

    /// Maple Leaf — 槭叶，热门主题移植，花草诗意中性名
    pub const MAPLE_LEAF: Self = Self {
        bg: Rgba::new(73, 48, 50, 255),
        text: Rgba::new(229, 224, 224, 255),
        accent: Rgba::new(223, 153, 107, 255),
        selection: Rgba::new(73, 48, 50, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(73, 48, 50, 255),
        warning: FIXED_WARNING,
    };

    /// Camphor Court — 樟庭，热门主题移植，花草诗意中性名
    pub const CAMPHOR_COURT: Self = Self {
        bg: Rgba::new(245, 247, 245, 255),
        text: Rgba::new(58, 79, 65, 255),
        accent: Rgba::new(34, 195, 145, 255),
        selection: Rgba::new(245, 247, 245, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(245, 247, 245, 255),
        warning: FIXED_WARNING,
    };

    /// Nanmu Stream — 楠溪，热门主题移植，花草诗意中性名
    pub const NANMU_STREAM: Self = Self {
        bg: Rgba::new(38, 32, 43, 255),
        text: Rgba::new(217, 212, 220, 255),
        accent: Rgba::new(221, 107, 223, 255),
        selection: Rgba::new(38, 32, 43, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(38, 32, 43, 255),
        warning: FIXED_WARNING,
    };

    /// Oak Plain — 栎原，热门主题移植，花草诗意中性名
    pub const OAK_PLAIN: Self = Self {
        bg: Rgba::new(240, 239, 236, 255),
        text: Rgba::new(61, 60, 45, 255),
        accent: Rgba::new(151, 195, 34, 255),
        selection: Rgba::new(240, 239, 236, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(240, 239, 236, 255),
        warning: FIXED_WARNING,
    };

    /// Sandalwood Smoke — 檀烟，热门主题移植，花草诗意中性名
    pub const SANDALWOOD_SMOKE: Self = Self {
        bg: Rgba::new(40, 64, 66, 255),
        text: Rgba::new(235, 238, 238, 255),
        accent: Rgba::new(107, 158, 223, 255),
        selection: Rgba::new(40, 64, 66, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(40, 64, 66, 255),
        warning: FIXED_WARNING,
    };

    /// Zelkova Forest — 榉林，热门主题移植，花草诗意中性名
    pub const ZELKOVA_FOREST: Self = Self {
        bg: Rgba::new(247, 245, 246, 255),
        text: Rgba::new(79, 58, 68, 255),
        accent: Rgba::new(195, 34, 57, 255),
        selection: Rgba::new(247, 245, 246, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 245, 246, 255),
        warning: FIXED_WARNING,
    };

    /// Torreya — 榧子，热门主题移植，花草诗意中性名
    pub const TORREYA: Self = Self {
        bg: Rgba::new(28, 36, 24, 255),
        text: Rgba::new(225, 229, 224, 255),
        accent: Rgba::new(107, 223, 124, 255),
        selection: Rgba::new(28, 36, 24, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(28, 36, 24, 255),
        warning: FIXED_WARNING,
    };

    /// Camellia Bud — 椿芽，热门主题移植，花草诗意中性名
    pub const CAMELLIA_BUD: Self = Self {
        bg: Rgba::new(236, 236, 240, 255),
        text: Rgba::new(46, 45, 61, 255),
        accent: Rgba::new(104, 34, 195, 255),
        selection: Rgba::new(236, 236, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(236, 236, 240, 255),
        warning: FIXED_WARNING,
    };

    /// Coconut Wind — 椰风，热门主题移植，花草诗意中性名
    pub const COCONUT_WIND: Self = Self {
        bg: Rgba::new(52, 42, 39, 255),
        text: Rgba::new(220, 215, 212, 255),
        accent: Rgba::new(223, 192, 107, 255),
        selection: Rgba::new(52, 42, 39, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(52, 42, 39, 255),
        warning: FIXED_WARNING,
    };

    /// Litchi — 荔枝，热门主题移植，花草诗意中性名
    pub const LITCHI: Self = Self {
        bg: Rgba::new(245, 247, 246, 255),
        text: Rgba::new(58, 79, 72, 255),
        accent: Rgba::new(34, 191, 195, 255),
        selection: Rgba::new(245, 247, 246, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(245, 247, 246, 255),
        warning: FIXED_WARNING,
    };

    /// Grain Awn — 芒种，热门主题移植，花草诗意中性名
    pub const GRAIN_AWN: Self = Self {
        bg: Rgba::new(71, 45, 76, 255),
        text: Rgba::new(238, 235, 238, 255),
        accent: Rgba::new(223, 107, 187, 255),
        selection: Rgba::new(71, 45, 76, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(71, 45, 76, 255),
        warning: FIXED_WARNING,
    };

    /// Thatched Pavilion — 茅亭，热门主题移植，花草诗意中性名
    pub const THATCHED_PAVILION: Self = Self {
        bg: Rgba::new(239, 240, 236, 255),
        text: Rgba::new(57, 61, 45, 255),
        accent: Rgba::new(97, 195, 34, 255),
        selection: Rgba::new(239, 240, 236, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(239, 240, 236, 255),
        warning: FIXED_WARNING,
    };

    /// Silver Grass — 荻芦，热门主题移植，花草诗意中性名
    pub const SILVER_GRASS: Self = Self {
        bg: Rgba::new(30, 39, 45, 255),
        text: Rgba::new(224, 226, 229, 255),
        accent: Rgba::new(107, 119, 223, 255),
        selection: Rgba::new(30, 39, 45, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(30, 39, 45, 255),
        warning: FIXED_WARNING,
    };

    /// Duckweed — 萍蓬，热门主题移植，花草诗意中性名
    pub const DUCKWEED: Self = Self {
        bg: Rgba::new(247, 245, 245, 255),
        text: Rgba::new(79, 58, 62, 255),
        accent: Rgba::new(195, 64, 34, 255),
        selection: Rgba::new(247, 245, 245, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 245, 245, 255),
        warning: FIXED_WARNING,
    };

    /// Water Chestnut Boat — 菱舟，热门主题移植，花草诗意中性名
    pub const WATER_CHESTNUT_BOAT: Self = Self {
        bg: Rgba::new(45, 61, 45, 255),
        text: Rgba::new(212, 220, 213, 255),
        accent: Rgba::new(107, 223, 163, 255),
        selection: Rgba::new(45, 61, 45, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(45, 61, 45, 255),
        warning: FIXED_WARNING,
    };

    /// Foxnut Pond — 芡塘，热门主题移植，花草诗意中性名
    pub const FOXNUT_POND: Self = Self {
        bg: Rgba::new(237, 236, 240, 255),
        text: Rgba::new(52, 45, 61, 255),
        accent: Rgba::new(158, 34, 195, 255),
        selection: Rgba::new(237, 236, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(237, 236, 240, 255),
        warning: FIXED_WARNING,
    };

    /// Knotweed Islet — 蓼屿，热门主题移植，花草诗意中性名
    pub const KNOTWEED_ISLET: Self = Self {
        bg: Rgba::new(38, 31, 22, 255),
        text: Rgba::new(238, 237, 235, 255),
        accent: Rgba::new(216, 223, 107, 255),
        selection: Rgba::new(38, 31, 22, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(38, 31, 22, 255),
        warning: FIXED_WARNING,
    };

    /// Thin Mist — 薄雾，热门主题移植，花草诗意中性名
    pub const THIN_MIST: Self = Self {
        bg: Rgba::new(245, 247, 246, 255),
        text: Rgba::new(58, 79, 79, 255),
        accent: Rgba::new(34, 138, 195, 255),
        selection: Rgba::new(245, 247, 246, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(245, 247, 246, 255),
        warning: FIXED_WARNING,
    };

    /// Bramble Stream — 荆溪，热门主题移植，花草诗意中性名
    pub const BRAMBLE_STREAM: Self = Self {
        bg: Rgba::new(55, 36, 52, 255),
        text: Rgba::new(229, 224, 228, 255),
        accent: Rgba::new(223, 107, 148, 255),
        selection: Rgba::new(55, 36, 52, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(55, 36, 52, 255),
        warning: FIXED_WARNING,
    };

    /// Vetch Dew — 薇露，热门主题移植，花草诗意中性名
    pub const VETCH_DEW: Self = Self {
        bg: Rgba::new(238, 240, 236, 255),
        text: Rgba::new(52, 61, 45, 255),
        accent: Rgba::new(44, 195, 34, 255),
        selection: Rgba::new(238, 240, 236, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(238, 240, 236, 255),
        warning: FIXED_WARNING,
    };

    /// Calamus Stream — 菖溪，热门主题移植，花草诗意中性名
    pub const CALAMUS_STREAM: Self = Self {
        bg: Rgba::new(52, 56, 70, 255),
        text: Rgba::new(212, 213, 220, 255),
        accent: Rgba::new(134, 107, 223, 255),
        selection: Rgba::new(52, 56, 70, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(52, 56, 70, 255),
        warning: FIXED_WARNING,
    };

    /// Asara Waste — 蘅芜，热门主题移植，花草诗意中性名
    pub const ASARA_WASTE: Self = Self {
        bg: Rgba::new(247, 245, 245, 255),
        text: Rgba::new(79, 61, 58, 255),
        accent: Rgba::new(195, 118, 34, 255),
        selection: Rgba::new(247, 245, 245, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 245, 245, 255),
        warning: FIXED_WARNING,
    };

    /// Azalea Balance — 杜衡，热门主题移植，花草诗意中性名
    pub const AZALEA_BALANCE: Self = Self {
        bg: Rgba::new(28, 47, 35, 255),
        text: Rgba::new(235, 238, 236, 255),
        accent: Rgba::new(107, 223, 202, 255),
        selection: Rgba::new(28, 47, 35, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(28, 47, 35, 255),
        warning: FIXED_WARNING,
    };

    /// Cuckoo Blood — 鹃血，热门主题移植，花草诗意中性名
    pub const CUCKOO_BLOOD: Self = Self {
        bg: Rgba::new(238, 236, 240, 255),
        text: Rgba::new(57, 45, 61, 255),
        accent: Rgba::new(195, 34, 178, 255),
        selection: Rgba::new(238, 236, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(238, 236, 240, 255),
        warning: FIXED_WARNING,
    };

    /// Violet Mud — 堇泥，热门主题移植，花草诗意中性名
    pub const VIOLET_MUD: Self = Self {
        bg: Rgba::new(64, 62, 42, 255),
        text: Rgba::new(229, 229, 224, 255),
        accent: Rgba::new(177, 223, 107, 255),
        selection: Rgba::new(64, 62, 42, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(64, 62, 42, 255),
        warning: FIXED_WARNING,
    };

    /// Mallow Garden — 葵园，热门主题移植，花草诗意中性名
    pub const MALLOW_GARDEN: Self = Self {
        bg: Rgba::new(245, 246, 247, 255),
        text: Rgba::new(58, 72, 79, 255),
        accent: Rgba::new(34, 84, 195, 255),
        selection: Rgba::new(245, 246, 247, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(245, 246, 247, 255),
        warning: FIXED_WARNING,
    };

    /// Hibiscus Cottage — 槿舍，热门主题移植，花草诗意中性名
    pub const HIBISCUS_COTTAGE: Self = Self {
        bg: Rgba::new(35, 26, 30, 255),
        text: Rgba::new(220, 212, 215, 255),
        accent: Rgba::new(223, 107, 110, 255),
        selection: Rgba::new(35, 26, 30, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(35, 26, 30, 255),
        warning: FIXED_WARNING,
    };

    /// Maple Bank — 槭浦，热门主题移植，花草诗意中性名
    pub const MAPLE_BANK: Self = Self {
        bg: Rgba::new(237, 240, 236, 255),
        text: Rgba::new(46, 61, 45, 255),
        accent: Rgba::new(34, 195, 78, 255),
        selection: Rgba::new(237, 240, 236, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(237, 240, 236, 255),
        warning: FIXED_WARNING,
    };

    /// Camphor Port — 樟埠，热门主题移植，花草诗意中性名
    pub const CAMPHOR_PORT: Self = Self {
        bg: Rgba::new(36, 34, 57, 255),
        text: Rgba::new(236, 235, 238, 255),
        accent: Rgba::new(173, 107, 223, 255),
        selection: Rgba::new(36, 34, 57, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(36, 34, 57, 255),
        warning: FIXED_WARNING,
    };

    /// Nanmu Camphor — 楠樟，热门主题移植，花草诗意中性名
    pub const NANMU_CAMPHOR: Self = Self {
        bg: Rgba::new(247, 245, 245, 255),
        text: Rgba::new(79, 68, 58, 255),
        accent: Rgba::new(195, 171, 34, 255),
        selection: Rgba::new(247, 245, 245, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 245, 245, 255),
        warning: FIXED_WARNING,
    };

    /// Oak Shrine — 栎社，热门主题移植，花草诗意中性名
    pub const OAK_SHRINE: Self = Self {
        bg: Rgba::new(48, 73, 65, 255),
        text: Rgba::new(224, 229, 228, 255),
        accent: Rgba::new(107, 206, 223, 255),
        selection: Rgba::new(48, 73, 65, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(48, 73, 65, 255),
        warning: FIXED_WARNING,
    };

    /// Sandalwood — 檀栾，热门主题移植，花草诗意中性名
    pub const SANDALWOOD: Self = Self {
        bg: Rgba::new(240, 236, 240, 255),
        text: Rgba::new(61, 45, 60, 255),
        accent: Rgba::new(195, 34, 124, 255),
        selection: Rgba::new(240, 236, 240, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(240, 236, 240, 255),
        warning: FIXED_WARNING,
    };

    /// Zelkova Stream — 榉溪，热门主题移植，花草诗意中性名
    pub const ZELKOVA_STREAM: Self = Self {
        bg: Rgba::new(41, 43, 32, 255),
        text: Rgba::new(217, 220, 212, 255),
        accent: Rgba::new(139, 223, 107, 255),
        selection: Rgba::new(41, 43, 32, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(41, 43, 32, 255),
        warning: FIXED_WARNING,
    };

    /// Torreya Wind — 榧风，热门主题移植，花草诗意中性名
    pub const TORREYA_WIND: Self = Self {
        bg: Rgba::new(245, 245, 247, 255),
        text: Rgba::new(58, 65, 79, 255),
        accent: Rgba::new(37, 34, 195, 255),
        selection: Rgba::new(245, 245, 247, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(245, 245, 247, 255),
        warning: FIXED_WARNING,
    };

    /// Camellia — 椿萱，热门主题移植，花草诗意中性名
    pub const CAMELLIA: Self = Self {
        bg: Rgba::new(66, 40, 44, 255),
        text: Rgba::new(238, 235, 235, 255),
        accent: Rgba::new(223, 144, 107, 255),
        selection: Rgba::new(66, 40, 44, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(66, 40, 44, 255),
        warning: FIXED_WARNING,
    };

    /// Coconut Island — 椰岛，热门主题移植，花草诗意中性名
    pub const COCONUT_ISLAND: Self = Self {
        bg: Rgba::new(236, 240, 236, 255),
        text: Rgba::new(45, 61, 49, 255),
        accent: Rgba::new(34, 195, 131, 255),
        selection: Rgba::new(236, 240, 236, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(236, 240, 236, 255),
        warning: FIXED_WARNING,
    };

    /// Litchi Bank — 荔浦，热门主题移植，花草诗意中性名
    pub const LITCHI_BANK: Self = Self {
        bg: Rgba::new(29, 24, 36, 255),
        text: Rgba::new(227, 224, 229, 255),
        accent: Rgba::new(211, 107, 223, 255),
        selection: Rgba::new(29, 24, 36, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(29, 24, 36, 255),
        warning: FIXED_WARNING,
    };

    /// Miscanthus Islet — 芒屿，热门主题移植，花草诗意中性名
    pub const MISCANTHUS_ISLET: Self = Self {
        bg: Rgba::new(247, 246, 245, 255),
        text: Rgba::new(79, 75, 58, 255),
        accent: Rgba::new(164, 195, 34, 255),
        selection: Rgba::new(247, 246, 245, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 246, 245, 255),
        warning: FIXED_WARNING,
    };

    /// Thatched Cottage — 茅庐，热门主题移植，花草诗意中性名
    pub const THATCHED_COTTAGE: Self = Self {
        bg: Rgba::new(39, 52, 52, 255),
        text: Rgba::new(212, 219, 220, 255),
        accent: Rgba::new(107, 168, 223, 255),
        selection: Rgba::new(39, 52, 52, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(39, 52, 52, 255),
        warning: FIXED_WARNING,
    };

    /// Silver Grass Islet — 荻洲，热门主题移植，花草诗意中性名
    pub const SILVER_GRASS_ISLET: Self = Self {
        bg: Rgba::new(240, 236, 239, 255),
        text: Rgba::new(61, 45, 54, 255),
        accent: Rgba::new(195, 34, 71, 255),
        selection: Rgba::new(240, 236, 239, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(240, 236, 239, 255),
        warning: FIXED_WARNING,
    };

    /// Duckweed Marsh — 萍沼，热门主题移植，花草诗意中性名
    pub const DUCKWEED_MARSH: Self = Self {
        bg: Rgba::new(58, 76, 45, 255),
        text: Rgba::new(236, 238, 235, 255),
        accent: Rgba::new(107, 223, 115, 255),
        selection: Rgba::new(58, 76, 45, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(58, 76, 45, 255),
        warning: FIXED_WARNING,
    };

    /// Algae Pond — 藻池，热门主题移植，花草诗意中性名
    pub const ALGAE_POND: Self = Self {
        bg: Rgba::new(245, 245, 247, 255),
        text: Rgba::new(58, 58, 79, 255),
        accent: Rgba::new(91, 34, 195, 255),
        selection: Rgba::new(245, 245, 247, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(245, 245, 247, 255),
        warning: FIXED_WARNING,
    };

    /// Water Chestnut Stream — 菱溪，热门主题移植，花草诗意中性名
    pub const WATER_CHESTNUT_STREAM: Self = Self {
        bg: Rgba::new(45, 33, 30, 255),
        text: Rgba::new(229, 225, 224, 255),
        accent: Rgba::new(223, 182, 107, 255),
        selection: Rgba::new(45, 33, 30, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(45, 33, 30, 255),
        warning: FIXED_WARNING,
    };

    /// Foxnut Islet — 芡洲，热门主题移植，花草诗意中性名
    pub const FOXNUT_ISLET: Self = Self {
        bg: Rgba::new(236, 240, 238, 255),
        text: Rgba::new(45, 61, 54, 255),
        accent: Rgba::new(34, 195, 185, 255),
        selection: Rgba::new(236, 240, 238, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(236, 240, 238, 255),
        warning: FIXED_WARNING,
    };

    /// Wild Rice Islet — 茭洲，热门主题移植，花草诗意中性名
    pub const WILD_RICE_ISLET: Self = Self {
        bg: Rgba::new(57, 45, 61, 255),
        text: Rgba::new(219, 212, 220, 255),
        accent: Rgba::new(223, 107, 197, 255),
        selection: Rgba::new(57, 45, 61, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(57, 45, 61, 255),
        warning: FIXED_WARNING,
    };

    /// Knotweed Islet 2 — 蓼洲，热门主题移植，花草诗意中性名
    pub const KNOTWEED_ISLET_2: Self = Self {
        bg: Rgba::new(247, 247, 245, 255),
        text: Rgba::new(75, 79, 58, 255),
        accent: Rgba::new(111, 195, 34, 255),
        selection: Rgba::new(247, 247, 245, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(247, 247, 245, 255),
        warning: FIXED_WARNING,
    };

    /// Calamus Islet — 菖洲，热门主题移植，花草诗意中性名
    pub const CALAMUS_ISLET: Self = Self {
        bg: Rgba::new(22, 33, 38, 255),
        text: Rgba::new(235, 237, 238, 255),
        accent: Rgba::new(107, 129, 223, 255),
        selection: Rgba::new(22, 33, 38, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(22, 33, 38, 255),
        warning: FIXED_WARNING,
    };

    /// Tangerine — 橘柚，热门主题移植，花草诗意中性名
    pub const TANGERINE: Self = Self {
        bg: Rgba::new(240, 236, 237, 255),
        text: Rgba::new(61, 45, 49, 255),
        accent: Rgba::new(195, 51, 34, 255),
        selection: Rgba::new(240, 236, 237, 255),
        danger: FIXED_DANGER,
        border: Rgba::new(240, 236, 237, 255),
        warning: FIXED_WARNING,
    };

    /// 内置预设（94 套花草诗意 + 6 套神话 vivid，无日夜字样，每色可经日/月翻转）。
    pub const PRESETS: [(&'static str, Self); 100] = [
        ("ink-wash", Self::DARK),
        ("rice-paper", Self::LIGHT),
        ("reed-mist", Self::LIGHT_COOL),
        ("wheat-awn", Self::LIGHT_WARM),
        ("wisteria", Self::DRACULA),
        ("pine-frost", Self::NORD),
        ("plum-ink", Self::CATPPUCCIN_MOCHA),
        ("yinglong", Self::TOKYO_NIGHT),
        ("persimmon", Self::GRUVBOX_DARK),
        ("moss", Self::EVERFOREST_DARK),
        ("wild-rose", Self::ROSE_PINE),
        ("indigo", Self::ONE_DARK_PRO),
        ("cotton-rose", Self::CATPPUCCIN_LATTE),
        ("bamboo", Self::GRUVBOX_LIGHT),
        ("silk", Self::XIAO_JUAN),
        ("tea-bud", Self::XING_YAN),
        ("bulrush", Self::YUE_ZHU),
        ("reed-fluff", Self::QIU_HAO),
        ("plum-rain", Self::PLUM_RAIN),
        ("orchid-valley", Self::ORCHID_VALLEY),
        ("bamboo-fence", Self::BAMBOO_FENCE),
        ("chrysanthemum-dew", Self::CHRYSANTHEMUM_DEW),
        ("pine-wind", Self::PINE_WIND),
        ("willow-bank", Self::WILLOW_BANK),
        ("peach-stream", Self::PEACH_STREAM),
        ("apricot-cove", Self::APRICOT_COVE),
        ("cherry-rain", Self::CHERRY_RAIN),
        ("qilin", Self::MAPLE_MARSH),
        ("lotus-pond", Self::LOTUS_POND),
        ("osmanthus-brew", Self::OSMANTHUS_BREW),
        ("fenghuang", Self::PAULOWNIA_COURT),
        ("ginkgo", Self::GINKGO),
        ("crabapple", Self::CRABAPPLE),
        ("vine-grass", Self::VINE_GRASS),
        ("duckweed-islet", Self::DUCKWEED_ISLET),
        ("algae", Self::ALGAE),
        ("water-chestnut-song", Self::WATER_CHESTNUT_SONG),
        ("foxnut", Self::FOXNUT),
        ("wild-rice-pond", Self::WILD_RICE_POND),
        ("knotweed-bank", Self::KNOTWEED_BANK),
        ("baize", Self::MINT),
        ("bramble-gate", Self::BRAMBLE_GATE),
        ("vetch-wall", Self::VETCH_WALL),
        ("kunpeng", Self::CALAMUS),
        ("knotweed-flower", Self::KNOTWEED_FLOWER),
        ("angelica-bank", Self::ANGELICA_BANK),
        ("asara", Self::ASARA),
        ("zhurong", Self::AZALEA),
        ("cuckoo-cry", Self::CUCKOO_CRY),
        ("violet", Self::VIOLET),
        ("mallow", Self::MALLOW),
        ("hibiscus-fence", Self::HIBISCUS_FENCE),
        ("maple-leaf", Self::MAPLE_LEAF),
        ("camphor-court", Self::CAMPHOR_COURT),
        ("nanmu-stream", Self::NANMU_STREAM),
        ("oak-plain", Self::OAK_PLAIN),
        ("sandalwood-smoke", Self::SANDALWOOD_SMOKE),
        ("zelkova-forest", Self::ZELKOVA_FOREST),
        ("torreya", Self::TORREYA),
        ("camellia-bud", Self::CAMELLIA_BUD),
        ("coconut-wind", Self::COCONUT_WIND),
        ("litchi", Self::LITCHI),
        ("grain-awn", Self::GRAIN_AWN),
        ("thatched-pavilion", Self::THATCHED_PAVILION),
        ("silver-grass", Self::SILVER_GRASS),
        ("duckweed", Self::DUCKWEED),
        ("water-chestnut-boat", Self::WATER_CHESTNUT_BOAT),
        ("foxnut-pond", Self::FOXNUT_POND),
        ("knotweed-islet", Self::KNOTWEED_ISLET),
        ("thin-mist", Self::THIN_MIST),
        ("bramble-stream", Self::BRAMBLE_STREAM),
        ("vetch-dew", Self::VETCH_DEW),
        ("calamus-stream", Self::CALAMUS_STREAM),
        ("asara-waste", Self::ASARA_WASTE),
        ("azalea-balance", Self::AZALEA_BALANCE),
        ("cuckoo-blood", Self::CUCKOO_BLOOD),
        ("violet-mud", Self::VIOLET_MUD),
        ("mallow-garden", Self::MALLOW_GARDEN),
        ("hibiscus-cottage", Self::HIBISCUS_COTTAGE),
        ("maple-bank", Self::MAPLE_BANK),
        ("camphor-port", Self::CAMPHOR_PORT),
        ("nanmu-camphor", Self::NANMU_CAMPHOR),
        ("oak-shrine", Self::OAK_SHRINE),
        ("sandalwood", Self::SANDALWOOD),
        ("zelkova-stream", Self::ZELKOVA_STREAM),
        ("torreya-wind", Self::TORREYA_WIND),
        ("camellia", Self::CAMELLIA),
        ("coconut-island", Self::COCONUT_ISLAND),
        ("litchi-bank", Self::LITCHI_BANK),
        ("miscanthus-islet", Self::MISCANTHUS_ISLET),
        ("thatched-cottage", Self::THATCHED_COTTAGE),
        ("silver-grass-islet", Self::SILVER_GRASS_ISLET),
        ("duckweed-marsh", Self::DUCKWEED_MARSH),
        ("algae-pond", Self::ALGAE_POND),
        ("water-chestnut-stream", Self::WATER_CHESTNUT_STREAM),
        ("foxnut-islet", Self::FOXNUT_ISLET),
        ("wild-rice-islet", Self::WILD_RICE_ISLET),
        ("knotweed-islet-2", Self::KNOTWEED_ISLET_2),
        ("calamus-islet", Self::CALAMUS_ISLET),
        ("tangerine", Self::TANGERINE),
    ];

    pub fn preset_by_name(name: &str) -> Option<Self> {
        Self::PRESETS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
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
