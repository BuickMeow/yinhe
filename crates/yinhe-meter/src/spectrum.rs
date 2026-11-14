//! 频谱分析：Hann 窗 + real FFT，输出 dBFS 幅度。

use std::sync::Arc;

use realfft::RealFftPlanner;
use realfft::RealToComplex;

/// 支持的 FFT 尺寸（与常见表头一致）。
pub const FFT_SIZES: &[usize] = &[1024, 2048, 4096, 8192, 16384];
/// 默认 FFT 尺寸。
pub const DEFAULT_FFT_SIZE: usize = 4096;
/// 频谱显示下限。
const MIN_DB: f32 = -120.0;

pub struct SpectrumAnalyzer {
    sample_rate: u32,
    fft_size: usize,
    hop: usize,
    planner: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    mono: Vec<f32>,
    fft_in: Vec<f32>,
    fft_out: Vec<realfft::num_complex::Complex<f32>>,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    magnitudes_db: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: u32, fft_size: usize) -> Self {
        let fft_size = normalize_fft_size(fft_size);
        let mut planner = RealFftPlanner::<f32>::new();
        let plan = planner.plan_fft_forward(fft_size);
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / (fft_size as f32 - 1.0);
                0.5 - 0.5 * x.cos()
            })
            .collect();
        let window_sum: f32 = window.iter().sum();
        Self {
            sample_rate,
            fft_size,
            hop: (fft_size / 4).max(1),
            planner: plan,
            window,
            window_sum: window_sum.max(1.0),
            mono: Vec::with_capacity(fft_size * 2),
            fft_in: vec![0.0; fft_size],
            fft_out: vec![realfft::num_complex::Complex::new(0.0, 0.0); fft_size / 2 + 1],
            scratch: vec![realfft::num_complex::Complex::new(0.0, 0.0); fft_size / 2 + 1],
            magnitudes_db: vec![MIN_DB; fft_size / 2 + 1],
        }
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 喂入交错立体声；至少积累一个完整 FFT 窗口时返回 true（表示频谱已更新）。
    pub fn process(&mut self, samples: &[f32]) -> bool {
        for frame in samples.as_chunks::<2>().0 {
            self.mono.push((frame[0] + frame[1]) * 0.5);
        }
        let mut updated = false;
        while self.mono.len() >= self.fft_size {
            self.compute();
            self.mono.drain(..self.hop.min(self.mono.len()));
            updated = true;
        }
        updated
    }

    fn compute(&mut self) {
        for (dst, (&src, &w)) in self
            .fft_in
            .iter_mut()
            .zip(self.mono.iter().zip(self.window.iter()))
        {
            *dst = src * w;
        }
        if self
            .planner
            .process_with_scratch(&mut self.fft_in, &mut self.fft_out, &mut self.scratch)
            .is_err()
        {
            return;
        }
        for (db, c) in self.magnitudes_db.iter_mut().zip(self.fft_out.iter()) {
            let mag = (c.re * c.re + c.im * c.im).sqrt() * 2.0 / self.window_sum;
            *db = (20.0 * mag.max(1e-12).log10()).clamp(MIN_DB, 0.0);
        }
    }

    pub fn magnitudes_db(&self) -> &[f32] {
        &self.magnitudes_db
    }

    /// 第 `bin` 个频点的中心频率。
    pub fn bin_frequency(&self, bin: usize) -> f32 {
        bin as f32 * self.sample_rate as f32 / self.fft_size as f32
    }

    pub fn reset(&mut self) {
        self.mono.clear();
        self.magnitudes_db.fill(MIN_DB);
    }
}

pub fn normalize_fft_size(fft_size: usize) -> usize {
    FFT_SIZES
        .iter()
        .copied()
        .find(|&n| n >= fft_size)
        .unwrap_or(*FFT_SIZES.last().unwrap_or(&DEFAULT_FFT_SIZE))
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
    fn finds_sine_peak_bin() {
        let mut a = SpectrumAnalyzer::new(48_000, 4096);
        let samples = sine(48_000, 1000.0, 0.5, 0.2);
        for chunk in samples.chunks(960) {
            a.process(chunk);
        }
        let bins = a.magnitudes_db();
        let (peak_bin, peak_db) = bins
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let expected = (1000.0f32 * 4096.0 / 48_000.0).round() as usize;
        assert!(
            peak_bin.abs_diff(expected) <= 2,
            "peak_bin={peak_bin} expected={expected}"
        );
        assert!(peak_db > -12.0, "peak_db={peak_db}");
    }

    #[test]
    fn silence_stays_at_floor() {
        let mut a = SpectrumAnalyzer::new(48_000, 1024);
        a.process(&vec![0.0; 4096]);
        assert!(a.magnitudes_db().iter().all(|&db| db <= -100.0));
    }

    #[test]
    fn normalizes_fft_size() {
        assert_eq!(normalize_fft_size(1), 1024);
        assert_eq!(normalize_fft_size(3000), 4096);
        assert_eq!(normalize_fft_size(99_999), 16_384);
    }
}
