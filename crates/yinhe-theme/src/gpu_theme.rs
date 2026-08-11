/// All GPU rendering colors for the application.
///
/// Each field corresponds to a previously hardcoded color constant.
/// `from_base` derives them from the 7 standard colors (`BaseColors`),
/// so a user theme change recolors the GPU-rendered views too.
#[derive(Clone, Debug)]
pub struct GpuTheme {
    // ── Pianoroll ──
    pub key_white: (f32, f32, f32),
    pub key_black: (f32, f32, f32),

    // ── Automation ──
    pub center_line: (f32, f32, f32, f32),
}

impl GpuTheme {
    /// 从 7 个标准色派生 GPU 渲染颜色（与 egui 侧 derive_theme 同源）。
    pub fn from_base(base: crate::base::BaseColors) -> Self {
        let bg = [
            base.bg.r as f32 / 255.0,
            base.bg.g as f32 / 255.0,
            base.bg.b as f32 / 255.0,
        ];
        let text = [
            base.text.r as f32 / 255.0,
            base.text.g as f32 / 255.0,
            base.text.b as f32 / 255.0,
        ];
        // 背景与文字按比例混合：暗主题提亮、亮主题压暗，方向自动正确
        let mix = |t: f32| {
            (
                bg[0] + (text[0] - bg[0]) * t,
                bg[1] + (text[1] - bg[1]) * t,
                bg[2] + (text[2] - bg[2]) * t,
            )
        };
        // 键盘两套 token：暗色保持柔和基线（白键=亮端 78%、黑键=暗端 8%）；
        // 亮色下白键贴近背景（亮端 97%，与周围一样亮）、黑键明显提浅
        // （暗端 20% 处，中深灰）——亮色不刺眼、白键不显脏。
        let dark = (bg[0] + bg[1] + bg[2]) / 3.0 <= 0.5;
        let dark_end = if dark { bg } else { text };
        let light_end = if dark { text } else { bg };
        let tone = |t: f32| {
            (
                dark_end[0] + (light_end[0] - dark_end[0]) * t,
                dark_end[1] + (light_end[1] - dark_end[1]) * t,
                dark_end[2] + (light_end[2] - dark_end[2]) * t,
            )
        };
        Self {
            key_white: if dark { tone(0.78) } else { tone(0.97) },
            key_black: if dark { tone(0.08) } else { tone(0.20) },
            center_line: (mix(0.28).0, mix(0.28).1, mix(0.28).2, 0.6),
        }
    }
}

impl Default for GpuTheme {
    fn default() -> Self {
        Self::from_base(crate::base::BaseColors::DARK)
    }
}

// ── 当前 GPU 主题（全局，随用户主题切换更新；wgpu 侧读取） ──

use std::sync::{OnceLock, RwLock};

static CURRENT: OnceLock<RwLock<GpuTheme>> = OnceLock::new();

/// 更新全局 GPU 主题（由 yinhe-egui 的 set_theme 在切换时同步调用）。
pub fn set_current_gpu_theme(theme: GpuTheme) {
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

/// 当前 GPU 主题（未初始化时按暗色派生，避免 panic）。
pub fn current_gpu_theme() -> GpuTheme {
    match CURRENT.get() {
        Some(lock) => lock
            .read()
            .map(|t| t.clone())
            .unwrap_or_else(|_| GpuTheme::default()),
        None => GpuTheme::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：所有预设下白键必须比黑键亮（亮色主题曾因单套派生逻辑
    /// 把白键算成深色、黑键算成浅色，黑白颠倒），且黑键不能接近纯黑
    /// （亮色下曾对调到 text×0.81 ≈ 24，纯黑刺眼）。
    #[test]
    fn key_colors_white_always_lighter_than_black() {
        let lum = |c: (f32, f32, f32)| c.0 + c.1 + c.2;
        for base in [
            crate::base::BaseColors::DARK,
            crate::base::BaseColors::LIGHT,
            crate::base::BaseColors::LIGHT_COOL,
            crate::base::BaseColors::LIGHT_WARM,
        ] {
            let t = GpuTheme::from_base(base);
            assert!(
                lum(t.key_white) > lum(t.key_black),
                "{:?} 下白键应亮于黑键: white={:?} black={:?}",
                base,
                t.key_white,
                t.key_black
            );
            // 黑键亮度 >= 0.4（三通道和）：柔和深灰而非纯黑，亮色主题不刺眼
            assert!(
                lum(t.key_black) >= 0.4,
                "{:?} 下黑键不应接近纯黑: {:?}",
                base,
                t.key_black
            );
            // 亮色下白键应与背景几乎同亮（不显脏）；暗色白键远离背景是设计如此
            let bg = (
                base.bg.r as f32 / 255.0,
                base.bg.g as f32 / 255.0,
                base.bg.b as f32 / 255.0,
            );
            let bg_lum = lum(bg);
            if bg_lum > 1.5 {
                assert!(
                    bg_lum - lum(t.key_white) < 0.3,
                    "{:?} 下亮色白键应接近背景: white={:?}",
                    base,
                    t.key_white
                );
            }
        }
    }
}
