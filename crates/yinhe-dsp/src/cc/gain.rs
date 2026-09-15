//! 通道音量/表情模块（CC7 Volume + CC11 Expression）。
//!
//! 语义对齐 xsynth `VoiceChannel::apply_channel_effects`：
//! `out *= (volume * expression)^2`，其中 `volume = CC7/128`、
//! `expression = CC11/128`，默认均为 1.0。

use yinhe_mixer::InsertProcessor;

use crate::cc::CC_RAMP_SECONDS;
use crate::dsp::smooth::Smoothed;

/// 通道音量/表情（CC 驱动）。
pub struct ChannelGain {
    volume: f32,
    expression: f32,
    /// `(volume * expression)^2` 的平滑值。
    gain: Smoothed,
    ramp_samples: u32,
}

impl ChannelGain {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            volume: 1.0,
            expression: 1.0,
            gain: Smoothed::new(1.0),
            ramp_samples: (sample_rate as f32 * CC_RAMP_SECONDS).max(1.0) as u32,
        }
    }

    fn update_target(&mut self) {
        let target = (self.volume * self.expression).powi(2);
        self.gain.set_target(target, self.ramp_samples);
    }
}

impl InsertProcessor for ChannelGain {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.gain.is_ramping() {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let g = self.gain.next_value();
                *l *= g;
                *r *= g;
            }
            return;
        }
        let g = self.gain.current();
        if g == 1.0 {
            return; // 默认增益：零开销
        }
        for s in left.iter_mut() {
            *s *= g;
        }
        for s in right.iter_mut() {
            *s *= g;
        }
    }

    fn handled_ccs(&self) -> &'static [u8] {
        &[7, 11]
    }

    fn apply_cc(&mut self, cc: u8, value: u8) {
        match cc {
            7 => self.volume = value as f32 / 128.0,
            11 => self.expression = value as f32 / 128.0,
            _ => return,
        }
        self.update_target();
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unity() {
        let mut g = ChannelGain::new(48_000);
        let mut l = [0.5; 8];
        let mut r = [0.25; 8];
        g.process(&mut l, &mut r);
        assert!(l.iter().all(|v| (*v - 0.5).abs() < 1e-6));
        assert!(r.iter().all(|v| (*v - 0.25).abs() < 1e-6));
    }

    #[test]
    fn cc7_scales_output() {
        let mut g = ChannelGain::new(48_000);
        g.apply_cc(7, 64); // 0.5^2 = 0.25
        let mut l = [1.0; 1024];
        let mut r = [1.0; 1024];
        g.process(&mut l, &mut r);
        // 斜坡结束后应精确到目标值。
        assert!((l[1023] - 0.25).abs() < 1e-4, "got {}", l[1023]);
        assert!((r[1023] - 0.25).abs() < 1e-4);
    }

    #[test]
    fn expression_multiplies_volume() {
        let mut g = ChannelGain::new(48_000);
        g.apply_cc(7, 128.min(127)); // 127/128
        g.apply_cc(11, 64); // 0.5
        let mut l = [1.0; 4096];
        let mut r = [1.0; 4096];
        g.process(&mut l, &mut r);
        let expect = (127.0 / 128.0 * 0.5f32).powi(2);
        assert!((l[4095] - expect).abs() < 1e-4);
    }
}
