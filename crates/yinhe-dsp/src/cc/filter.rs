//! 通道低通滤波模块（CC74 Cutoff + CC71 Resonance）。
//!
//! 语义对齐 xsynth `VoiceChannel` 的通道级滤波：
//! - CC74 < 64：启用低通，截止频率按 FREQS 键频表映射
//!   （`2^((key-69)/12) × 440`，`key = CC74 + 64`；> 7000Hz 部分 ×2.36 抬升）；
//! - CC74 >= 64：旁路（`None`）；
//! - CC71 > 64：Q = `10^((CC71-64)/48) × 1/√2`（-3dB 处 2.4 步进换算）；
//! - CC71 <= 64：默认 Butterworth Q。
//!
//! 系数与 xsynth/`yinhe-synth` 的 RBJ cookbook 一致（见 `crate::dsp::biquad`）。
//! 参数在 CC 事件到达时更新，块级重算系数（CC 事件本身以块为粒度）。

use yinhe_mixer::InsertProcessor;

use crate::dsp::biquad::{BiquadState, Q_BUTTERWORTH, low_pass_coeffs};

/// CC74 → 截止频率（Hz），复刻 xsynth 的 FREQS 表。
fn cutoff_from_cc74(value: u8) -> f32 {
    let key = value as f32 + 64.0;
    let mut freq = 2.0f32.powf((key - 69.0) / 12.0) * 440.0;
    if freq > 7000.0 {
        let mult = freq / 7000.0 - 1.0;
        freq = (mult * 2.36 + 1.0) * 7000.0;
    }
    freq
}

/// CC71 → 滤波器 Q，复刻 xsynth（`10^((v-64)/48) × 1/√2`）。
fn q_from_cc71(value: u8) -> f32 {
    let db = (value as f32 - 64.0) / 2.4;
    10.0f32.powf(db / 20.0) * Q_BUTTERWORTH
}

/// 通道低通滤波（CC 驱动）。
pub struct ChannelFilter {
    sample_rate: f32,
    /// 目标截止频率；`None` = 旁路（CC74 >= 64）。
    cutoff_hz: Option<f32>,
    /// 目标 Q；`None` = Butterworth。
    q: Option<f32>,
    coeffs: (f32, f32, f32, f32, f32),
    left: BiquadState,
    right: BiquadState,
    /// 参数变化后待重算系数。
    dirty: bool,
}

impl ChannelFilter {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate as f32;
        Self {
            sample_rate,
            cutoff_hz: None,
            q: None,
            coeffs: low_pass_coeffs(1000.0, Q_BUTTERWORTH, sample_rate),
            left: BiquadState::default(),
            right: BiquadState::default(),
            dirty: false,
        }
    }

    fn refresh_coeffs(&mut self) {
        if let Some(cutoff) = self.cutoff_hz {
            self.coeffs =
                low_pass_coeffs(cutoff, self.q.unwrap_or(Q_BUTTERWORTH), self.sample_rate);
        }
        self.dirty = false;
    }
}

impl InsertProcessor for ChannelFilter {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.cutoff_hz.is_none() {
            return; // 旁路（与 xsynth 一致：滤波器状态保留，不重置）
        }
        if self.dirty {
            self.refresh_coeffs();
        }
        let coeffs = self.coeffs;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            *l = self.left.process(*l, coeffs);
            *r = self.right.process(*r, coeffs);
        }
    }

    fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    fn handled_ccs(&self) -> &'static [u8] {
        &[71, 74]
    }

    fn apply_cc(&mut self, cc: u8, value: u8) {
        match cc {
            74 => {
                self.cutoff_hz = (value < 64).then(|| cutoff_from_cc74(value));
                self.dirty = true;
            }
            71 => {
                self.q = (value > 64).then(|| q_from_cc71(value));
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bypasses() {
        let mut f = ChannelFilter::new(48_000);
        let input = [0.3f32; 64];
        let mut l = input;
        let mut r = input;
        f.process(&mut l, &mut r);
        assert_eq!(l, input, "默认应旁路");
    }

    #[test]
    fn cc74_below_64_enables_filter() {
        let mut f = ChannelFilter::new(48_000);
        f.apply_cc(74, 0); // key=64 → 约 329.6 Hz
        assert!(f.cutoff_hz.is_some());
        assert!((f.cutoff_hz.unwrap() - 329.63).abs() < 1.0);
    }

    #[test]
    fn cc74_at_or_above_64_bypasses() {
        let mut f = ChannelFilter::new(48_000);
        f.apply_cc(74, 64);
        assert!(f.cutoff_hz.is_none());
    }

    #[test]
    fn cc71_sets_q() {
        let mut f = ChannelFilter::new(48_000);
        f.apply_cc(71, 64);
        assert!(f.q.is_none());
        f.apply_cc(71, 65);
        assert!((f.q.unwrap() - Q_BUTTERWORTH * 10f32.powf(1.0 / 48.0)).abs() < 1e-4);
    }

    #[test]
    fn filtered_output_stays_finite() {
        let mut f = ChannelFilter::new(48_000);
        f.apply_cc(74, 0);
        f.apply_cc(71, 127);
        let mut l = [1.0f32; 2048];
        let mut r = [1.0f32; 2048];
        f.process(&mut l, &mut r);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
    }
}
