//! 经典 VU：300ms 平均 RMS（一阶平滑），指针显示在 UI 侧完成。

#[derive(Clone, Copy, Debug, Default)]
pub struct VuReading {
    pub rms_l: f32,
    pub rms_r: f32,
    pub rms_mid: f32,
}

pub struct VuAnalyzer {
    alpha: f32,
    mean_l: f32,
    mean_r: f32,
    mean_mid: f32,
}

impl VuAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        let seconds = 0.3;
        let alpha = 1.0 - (-1.0 / (seconds * sample_rate as f32)).exp();
        Self {
            alpha,
            mean_l: 0.0,
            mean_r: 0.0,
            mean_mid: 0.0,
        }
    }

    pub fn process(&mut self, samples: &[f32]) {
        let a = self.alpha;
        for frame in samples.as_chunks::<2>().0 {
            let l = frame[0];
            let r = frame[1];
            let mid = (l + r) * 0.5;
            self.mean_l += a * (l * l - self.mean_l);
            self.mean_r += a * (r * r - self.mean_r);
            self.mean_mid += a * (mid * mid - self.mean_mid);
        }
    }

    pub fn reading(&self) -> VuReading {
        VuReading {
            rms_l: self.mean_l.max(0.0).sqrt(),
            rms_r: self.mean_r.max(0.0).sqrt(),
            rms_mid: self.mean_mid.max(0.0).sqrt(),
        }
    }

    pub fn reset(&mut self) {
        self.mean_l = 0.0;
        self.mean_r = 0.0;
        self.mean_mid = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_sine_rms() {
        let mut a = VuAnalyzer::new(48_000);
        let mut samples = Vec::new();
        for i in 0..96_000 {
            let s = (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin();
            samples.push(s);
            samples.push(s);
        }
        for chunk in samples.chunks(480) {
            a.process(chunk);
        }
        let r = a.reading().rms_mid;
        assert!((r - 0.707).abs() < 0.05, "rms={r}");
    }
}
