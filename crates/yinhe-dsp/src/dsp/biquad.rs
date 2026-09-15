//! RBJ cookbook biquad 系数与 DirectForm1 状态。
//!
//! 系数与 xsynth 的 `biquad` crate、`yinhe-synth/src/synth/filter.rs::biquad_coeffs`
//! 语义一致（同一 RBJ cookbook 公式）。
//!
//! 复用取舍（spec-yinhe-dsp §10-5）：未把 `biquad_coeffs` 提取为共享模块——
//! yinhe-synth 是 GPU 渲染器（强依赖 wgpu），yinhe-dsp 是轻量 CPU 效果器库，
//! 两边各自持有一份约 30 行的纯数学实现，避免依赖纠缠；改动时两边需同步。

/// Butterworth Q（1/√2）。
pub const Q_BUTTERWORTH: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// RBJ cookbook 低通系数，返回 `(b0, b1, b2, a1, a2)`（DirectForm1）：
/// `y = b0*x + b1*x1 + b2*x2 - a1*y1 - a2*y2`
///
/// cutoff 先 clamp 到 `[1, Nyquist-1]`：CC74 极端值会算出超过 Nyquist 的
/// 频率，未 clamp 的系数会让 DF1 数值不稳定（自激振荡/啸叫）。
pub fn low_pass_coeffs(cutoff: f32, q: f32, sample_rate: f32) -> (f32, f32, f32, f32, f32) {
    let nyquist = (sample_rate * 0.5).max(1.0);
    let cutoff = cutoff.clamp(1.0, (nyquist - 1.0).max(1.0));
    let omega = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
    let q = if q > 0.0 { q } else { Q_BUTTERWORTH };
    let omega_s = omega.sin();
    let omega_c = omega.cos();
    let alpha = omega_s / (2.0 * q);
    let b0 = (1.0 - omega_c) * 0.5;
    let a0 = 1.0 + alpha;
    (
        b0 / a0,
        2.0 * b0 / a0,
        b0 / a0,
        -2.0 * omega_c / a0,
        (1.0 - alpha) / a0,
    )
}

/// DirectForm1 单声道状态。
#[derive(Clone, Copy, Debug, Default)]
pub struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    /// 清空历史（seek/reset）。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 处理一个样本。
    #[inline]
    pub fn process(&mut self, x: f32, coeffs: (f32, f32, f32, f32, f32)) -> f32 {
        let (b0, b1, b2, a1, a2) = coeffs;
        let y = b0 * x + b1 * self.x1 + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_pass_dc_gain_is_unity() {
        // 直流增益 = (b0+b1+b2)/(1+a1+a2) = 1
        let (b0, b1, b2, a1, a2) = low_pass_coeffs(1000.0, Q_BUTTERWORTH, 48_000.0);
        let gain = (b0 + b1 + b2) / (1.0 + a1 + a2);
        assert!((gain - 1.0).abs() < 1e-3, "dc gain {gain}");
    }

    #[test]
    fn low_pass_attenuates_high_frequency() {
        let coeffs = low_pass_coeffs(1000.0, Q_BUTTERWORTH, 48_000.0);
        let mut state = BiquadState::default();
        // 8kHz 正弦：输出幅度应远小于输入。
        let mut peak_out = 0.0f32;
        let mut peak_in = 0.0f32;
        for i in 0..4800 {
            let t = i as f32 / 48_000.0;
            let x = (2.0 * std::f32::consts::PI * 8000.0 * t).sin();
            let y = state.process(x, coeffs);
            if i > 2400 {
                peak_in = peak_in.max(x.abs());
                peak_out = peak_out.max(y.abs());
            }
        }
        assert!(peak_out < peak_in * 0.2, "in {peak_in} out {peak_out}");
    }

    #[test]
    fn cutoff_clamped_stays_stable() {
        // 极端 cutoff（远超 Nyquist）不应产生 NaN/Inf。
        let coeffs = low_pass_coeffs(1e9, 20.0, 48_000.0);
        let mut state = BiquadState::default();
        for i in 0..1000 {
            let x = if i % 2 == 0 { 1.0 } else { -1.0 };
            let y = state.process(x, coeffs);
            assert!(y.is_finite(), "y={y}");
        }
    }
}
