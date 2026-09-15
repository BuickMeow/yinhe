//! 通道声像模块（Pan 参数）。
//!
//! 语义对齐 xsynth `VoiceChannel::apply_channel_effects` 的立体声 Pan：
//! `pan = Pan/127`，等功率声像
//! `left *= cos(pan * π/2)`、`right *= sin(pan * π/2)`，
//! 默认 `Pan = 64`（中心，左右各 -3dB）。
//!
//! 参数在底层伪装成 MIDI CC（Pan=CC10）存储与导出。
//!
//! 注意与混音台通道条的 `StripParams.pan`（工程混音设置）区分：
//! 本模块处理乐曲内容，两者串联、互不相干。

use yinhe_mixer::InsertProcessor;

use crate::cc::CC_RAMP_SECONDS;
use crate::dsp::smooth::Smoothed;

/// 等功率声像增益。
#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = pan * std::f32::consts::FRAC_PI_2;
    (angle.cos().min(1.0), angle.sin().min(1.0))
}

/// 通道声像。
pub struct ChannelPan {
    pan: Smoothed,
    ramp_samples: u32,
}

impl ChannelPan {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            pan: Smoothed::new(64.0 / 127.0),
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
            self.pan.set_target(value as f32 / 127.0, self.ramp_samples);
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
        let (gl, gr) = pan_gains(64.0 / 127.0);
        assert!((l[0] - gl).abs() < 1e-5);
        assert!((r[0] - gr).abs() < 1e-5);
    }

    #[test]
    fn cc127_nearly_silences_left() {
        let mut p = ChannelPan::new(48_000);
        p.apply_cc(10, 127);
        let mut l = [1.0; 4096];
        let mut r = [1.0; 4096];
        p.process(&mut l, &mut r);
        // pan = 127/127 = 1.0：完全右（左声道静音）。
        assert!(l[4095].abs() < 1e-3, "left {}", l[4095]);
        assert!((r[4095] - 1.0).abs() < 1e-3, "right {}", r[4095]);
    }
}
