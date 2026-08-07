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
        Self {
            key_white: (text[0] * 0.81, text[1] * 0.81, text[2] * 0.81),
            key_black: mix(0.08),
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
