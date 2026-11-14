//! 立体声场：相关度 + L/R/MS 电平 + Lissajous 点。

use std::collections::VecDeque;

/// 单帧立体声读数。
#[derive(Clone, Copy, Debug, Default)]
pub struct StereoReading {
    /// 相关系数 -1..1（静音时为 0）。
    pub correlation: f32,
    /// 左右能量平衡 -1（全左）..1（全右）。
    pub balance: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

pub struct StereoAnalyzer {
    window_frames: usize,
    frames: VecDeque<(f32, f32)>,
    sum_lr: f64,
    sum_l2: f64,
    sum_r2: f64,
    reading: StereoReading,
}

impl StereoAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            window_frames: ((sample_rate as usize * 3) / 10).max(1),
            frames: VecDeque::with_capacity(((sample_rate as usize * 3) / 10).max(1)),
            sum_lr: 0.0,
            sum_l2: 0.0,
            sum_r2: 0.0,
            reading: StereoReading::default(),
        }
    }

    pub fn process(&mut self, samples: &[f32]) {
        for frame in samples.as_chunks::<2>().0 {
            let l = frame[0];
            let r = frame[1];
            if self.frames.len() >= self.window_frames
                && let Some((old_l, old_r)) = self.frames.pop_front()
            {
                self.sum_lr -= f64::from(old_l) * f64::from(old_r);
                self.sum_l2 -= f64::from(old_l) * f64::from(old_l);
                self.sum_r2 -= f64::from(old_r) * f64::from(old_r);
            }
            self.frames.push_back((l, r));
            self.sum_lr += f64::from(l) * f64::from(r);
            self.sum_l2 += f64::from(l) * f64::from(l);
            self.sum_r2 += f64::from(r) * f64::from(r);
        }
        let denom = (self.sum_l2.max(0.0) * self.sum_r2.max(0.0)).sqrt();
        let correlation = if denom > 1e-12 {
            (self.sum_lr / denom).clamp(-1.0, 1.0) as f32
        } else {
            0.0
        };
        let energy = self.sum_l2 + self.sum_r2;
        let balance = if energy > 1e-12 {
            ((self.sum_r2 - self.sum_l2) / energy) as f32
        } else {
            0.0
        };
        self.reading = StereoReading {
            correlation,
            balance,
            rms_l: (self.sum_l2 / self.frames.len().max(1) as f64).sqrt() as f32,
            rms_r: (self.sum_r2 / self.frames.len().max(1) as f64).sqrt() as f32,
        };
    }

    pub fn reading(&self) -> StereoReading {
        self.reading
    }

    /// 最近的 Lissajous 点 `(mid, side)`，最多 `max_points` 个。
    pub fn points(&self, max_points: usize, out: &mut Vec<(f32, f32)>) {
        out.clear();
        if self.frames.is_empty() || max_points == 0 {
            return;
        }
        let stride = (self.frames.len() / max_points).max(1);
        out.reserve(max_points);
        for (i, &(l, r)) in self.frames.iter().enumerate() {
            if i % stride == 0 {
                out.push(((l + r) * 0.5, (l - r) * 0.5));
                if out.len() >= max_points {
                    break;
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.frames.clear();
        self.sum_lr = 0.0;
        self.sum_l2 = 0.0;
        self.sum_r2 = 0.0;
        self.reading = StereoReading::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_channels_are_fully_correlated() {
        let mut a = StereoAnalyzer::new(48_000);
        a.process(&[0.5, 0.5, 0.25, 0.25]);
        assert!((a.reading().correlation - 1.0).abs() < 1e-5);
        assert!(a.reading().balance.abs() < 1e-5);
    }

    #[test]
    fn inverted_channels_are_negative_correlated() {
        let mut a = StereoAnalyzer::new(48_000);
        a.process(&[0.5, -0.5, 0.25, -0.25]);
        assert!((a.reading().correlation + 1.0).abs() < 1e-5);
    }

    #[test]
    fn silence_has_zero_correlation() {
        let mut a = StereoAnalyzer::new(48_000);
        a.process(&[0.0; 64]);
        assert_eq!(a.reading().correlation, 0.0);
    }
}
