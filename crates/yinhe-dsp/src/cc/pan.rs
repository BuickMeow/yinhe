//! 通道声像模块（CC10 Pan）。
//!
//! 语义对齐 xsynth `VoiceChannel::apply_channel_effects` 的立体声 Pan：
//! `pan = CC10/128`，等功率声像
//! `left *= cos(pan * π/2)`、`right *= sin(pan * π/2)`，
//! 默认 `pan = 0.5`（中心，左右各 -3dB）。
//!
//! 注意与混音台通道条的 `StripParams.pan`（工程混音设置）区分：
//! 本模块处理乐曲内容（CC），两者串联、互不相干。

use yinhe_mixer::InsertProcessor;

use crate::cc::CC_RAMP_SECONDS;
use crate::dsp::smooth::Smoothed;

/// 等功率声像增益。
#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = pan * std::f32::consts::FRAC_PI_2;
    (angle.cos().min(1.0), angle.sin().min(1.0))
}

/// 通道声像（CC 驱动）。
pub struct ChannelPan {
    pan: Smoothed,
    ramp_samples: u32,
}

impl ChannelPan {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            pan: Smoothed::new(0.5),
            ramp_samples: (sample_rate as f32 * CC_RAMP_SECONDS).max(1.0) as u32,
        }
    }
}

impl InsertProcessor for ChannelPan {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.pan.is_ramping() {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let (gl, gr) = pan_gains(self.pan.next_value());
                *l *= gl;
                *r *= gr;
            }
            return;
        }
        let (gl, gr) = pan_gains(self.pan.current());
        for l in left.iter_mut() {
            *l *= gl;
        }
        for r in right.iter_mut() {
            *r *= gr;
        }
    }

    fn handled_ccs(&self) -> &'static [u8] {
        &[10]
    }

    fn apply_cc(&mut self, cc: u8, value: u8) {
        if cc == 10 {
            self.pan.set_target(value as f32 / 128.0, self.ramp_samples);
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
    fn center_is_equal_power() {
        let mut p = ChannelPan::new(48_000);
        let mut l = [1.0; 8];
        let mut r = [1.0; 8];
        p.process(&mut l, &mut r);
        let expect = std::f32::consts::FRAC_1_SQRT_2;
        assert!((l[0] - expect).abs() < 1e-5);
        assert!((r[0] - expect).abs() < 1e-5);
    }

    #[test]
    fn cc127_nearly_silences_left() {
        let mut p = ChannelPan::new(48_000);
        p.apply_cc(10, 127);
        let mut l = [1.0; 4096];
        let mut r = [1.0; 4096];
        p.process(&mut l, &mut r);
        // 语义对齐 xsynth：pan = 127/128（不是 1.0），左声道接近静音。
        let expect_l = (127.0 / 128.0 * std::f32::consts::FRAC_PI_2).cos();
        assert!((l[4095] - expect_l).abs() < 1e-3, "left {}", l[4095]);
        let expect_r = (127.0 / 128.0 * std::f32::consts::FRAC_PI_2).sin();
        assert!((r[4095] - expect_r).abs() < 1e-3, "right {}", r[4095]);
    }
}
