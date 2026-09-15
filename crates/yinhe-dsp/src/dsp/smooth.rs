//! 线性斜坡参数。
//!
//! 语义与 xsynth 的 `ValueLerp` 一致：目标变化时按固定样本数线性推进；
//! 块级参数变化用 10ms 斜坡避免 zipper 噪声。

/// 线性斜坡参数。
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Smoothed {
    /// 以 `value` 同时作为当前值与目标值（无斜坡）。
    pub const fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    /// 设定新目标值，按 `ramp_samples` 个样本线性逼近。
    /// 目标与当前目标一致时不打断进行中的斜坡。
    pub fn set_target(&mut self, target: f32, ramp_samples: u32) {
        if (target - self.target).abs() < f32::EPSILON {
            return;
        }
        self.target = target;
        if ramp_samples == 0 || self.current == target {
            self.current = target;
            self.step = 0.0;
            self.remaining = 0;
            return;
        }
        self.step = (target - self.current) / ramp_samples as f32;
        self.remaining = ramp_samples;
    }

    /// 直接跳变到 `value`（初始化/chase 回填用，不产生斜坡）。
    pub fn reset_to(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.step = 0.0;
        self.remaining = 0;
    }

    /// 当前值。
    pub fn current(&self) -> f32 {
        self.current
    }

    /// 目标值。
    pub fn target(&self) -> f32 {
        self.target
    }

    /// 是否正处于斜坡中。
    pub fn is_ramping(&self) -> bool {
        self.remaining > 0
    }

    /// 推进一个样本并返回平滑值。
    pub fn next_value(&mut self) -> f32 {
        if self.remaining > 0 {
            self.current += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            }
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_reaches_target_exactly() {
        let mut s = Smoothed::new(1.0);
        s.set_target(0.0, 10);
        assert!(s.is_ramping());
        for _ in 0..9 {
            s.next_value();
        }
        assert!(s.current() > 0.0);
        assert_eq!(s.next_value(), 0.0);
        assert!(!s.is_ramping());
    }

    #[test]
    fn same_target_keeps_ramp() {
        let mut s = Smoothed::new(1.0);
        s.set_target(0.0, 10);
        s.next_value();
        let mid = s.current();
        s.set_target(0.0, 10);
        assert_eq!(s.current(), mid);
        assert!(s.is_ramping());
    }

    #[test]
    fn reset_to_jumps() {
        let mut s = Smoothed::new(1.0);
        s.reset_to(0.25);
        assert_eq!(s.current(), 0.25);
        assert!(!s.is_ramping());
    }
}
