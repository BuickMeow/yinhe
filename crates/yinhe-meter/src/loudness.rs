//! ITU-R BS.1770 / EBU R128 响度分析。
//!
//! K-weighting、门限和积分响度交给维护良好的 `ebur128` crate；本类型只负责
//! 把交错立体声样本喂进去，并补充 UI 需要的块峰值/块 RMS。

use ebur128::{EbuR128, Mode};

/// 单帧响度读数。
///
/// 峰值/RMS 是**本次 `process` 输入块**的值（线性），UI 侧自己做弹道/平滑。
/// LUFS 为 `f32::NEG_INFINITY` 表示静音或尚未积累到足够窗口。
#[derive(Clone, Copy, Debug)]
pub struct LoudnessReading {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
    pub momentary: f32,
    pub short_term: f32,
    pub integrated: f32,
}

impl Default for LoudnessReading {
    fn default() -> Self {
        Self {
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
            momentary: f32::NEG_INFINITY,
            short_term: f32::NEG_INFINITY,
            integrated: f32::NEG_INFINITY,
        }
    }
}

pub struct LoudnessAnalyzer {
    sample_rate: u32,
    ebu: Option<EbuR128>,
    reading: LoudnessReading,
}

impl LoudnessAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            ebu: Self::new_ebu(sample_rate),
            reading: LoudnessReading::default(),
        }
    }

    fn new_ebu(sample_rate: u32) -> Option<EbuR128> {
        EbuR128::new(2, sample_rate, Mode::M | Mode::S | Mode::I).ok()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 喂入一段交错立体声样本。奇数尾样本忽略。
    pub fn process(&mut self, samples: &[f32]) {
        if samples.len() < 2 {
            self.reading.peak_l = 0.0;
            self.reading.peak_r = 0.0;
            self.reading.rms_l = 0.0;
            self.reading.rms_r = 0.0;
            return;
        }

        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        let mut sum_l = 0.0f64;
        let mut sum_r = 0.0f64;
        let mut frames = 0u32;
        for frame in samples.as_chunks::<2>().0 {
            let l = frame[0];
            let r = frame[1];
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
            sum_l += f64::from(l) * f64::from(l);
            sum_r += f64::from(r) * f64::from(r);
            frames += 1;
        }

        self.reading.peak_l = peak_l;
        self.reading.peak_r = peak_r;
        self.reading.rms_l = (sum_l / f64::from(frames.max(1))).sqrt() as f32;
        self.reading.rms_r = (sum_r / f64::from(frames.max(1))).sqrt() as f32;

        if let Some(ebu) = self.ebu.as_mut() {
            let _ = ebu.add_frames_f32(samples);
            self.reading.momentary = ebu
                .loudness_momentary()
                .map(|v| v as f32)
                .unwrap_or(f32::NEG_INFINITY);
            self.reading.short_term = ebu
                .loudness_shortterm()
                .map(|v| v as f32)
                .unwrap_or(f32::NEG_INFINITY);
            self.reading.integrated = ebu
                .loudness_global()
                .map(|v| v as f32)
                .unwrap_or(f32::NEG_INFINITY);
        }
    }

    pub fn reading(&self) -> LoudnessReading {
        self.reading
    }

    /// 清空积分响度与滤波器状态（切片/重新播放时调用）。
    pub fn reset(&mut self) {
        self.ebu = Self::new_ebu(self.sample_rate);
        self.reading = LoudnessReading::default();
    }
}

/// 改变采样率时重建（K-weighting 系数依赖采样率）。
pub fn recreate(analyzer: &mut LoudnessAnalyzer, sample_rate: u32) {
    *analyzer = LoudnessAnalyzer::new(sample_rate);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(sample_rate: u32, freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let frames = (sample_rate as f32 * seconds) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin() * amp;
            out.push(s);
            out.push(s);
        }
        out
    }

    #[test]
    fn reads_peak_and_rms() {
        let mut a = LoudnessAnalyzer::new(48_000);
        a.process(&[0.5, -0.25, -0.5, 0.25]);
        let r = a.reading();
        assert!((r.peak_l - 0.5).abs() < 1e-6);
        assert!((r.peak_r - 0.25).abs() < 1e-6);
        assert!((r.rms_l - 0.5).abs() < 1e-6);
        assert!((r.rms_r - 0.25).abs() < 1e-6);
    }

    #[test]
    fn measures_minus_23_lufs_sine() {
        let mut a = LoudnessAnalyzer::new(48_000);
        // -23 dBFS 的 1kHz 正弦，持续 5 秒，足以填满 M/S/I 窗口。
        let amp = 10f32.powf(-23.0 / 20.0);
        let samples = sine(48_000, 1000.0, amp, 5.0);
        for chunk in samples.chunks(960) {
            a.process(chunk);
        }
        let r = a.reading();
        assert!(
            (r.integrated - -23.0).abs() < 1.0,
            "integrated={}",
            r.integrated
        );
        assert!(
            (r.momentary - -23.0).abs() < 1.0,
            "momentary={}",
            r.momentary
        );
        assert!(
            (r.short_term - -23.0).abs() < 1.0,
            "short_term={}",
            r.short_term
        );
    }

    #[test]
    fn silence_is_negative_infinity() {
        let mut a = LoudnessAnalyzer::new(48_000);
        a.process(&vec![0.0; 4800]);
        assert!(a.reading().integrated.is_infinite());
    }
}
