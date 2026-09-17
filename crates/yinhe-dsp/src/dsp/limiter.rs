//! 输出软限幅（无阈值 tanh 饱和）。
//!
//! 由 yinhe-audio 在最终输出（混音/预览叠加后）统一调用；
//! 合成器内部不再做任何 DSP（见 `docs/spec-yinhe-dsp.md`）。

/// 多通道软限幅器：`out = tanh(in)`。
///
/// 替代 xsynth 的 AGC 峰值限幅（`val / loudness / 2`）：后者起音瞬态需
/// ~100 样本追赶，期间输出可远超 1.0 → 设备硬削波（多 voice 密集叠加时
/// 的高频滋滋）。tanh 无阈值、无状态：`|tanh(x)| < 1` 恒成立（数学保证
/// 不削波）；`|x| < 0.3` 时误差 < 0.1dB（小信号近似线性）；大信号平滑饱和。
pub struct VolumeLimiter;

impl VolumeLimiter {
    pub fn new() -> Self {
        Self
    }

    pub fn limit(&mut self, sample: &mut [f32]) {
        for s in sample.iter_mut() {
            *s = s.tanh();
        }
    }
}

impl Default for VolumeLimiter {
    fn default() -> Self {
        Self::new()
    }
}
