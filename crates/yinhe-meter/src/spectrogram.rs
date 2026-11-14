//! 频谱图历史：每次频谱更新推入一列，供 UI 转成纹理。

use std::collections::VecDeque;

use crate::spectrum::{DEFAULT_FFT_SIZE, SpectrumAnalyzer};

pub struct SpectrogramAnalyzer {
    spectrum: SpectrumAnalyzer,
    max_columns: usize,
    columns: VecDeque<Vec<f32>>,
    /// 被挤出的列缓冲，复用以避免每帧分配。
    recycle: Vec<Vec<f32>>,
}

impl SpectrogramAnalyzer {
    pub fn new(sample_rate: u32, fft_size: usize, max_columns: usize) -> Self {
        Self {
            spectrum: SpectrumAnalyzer::new(sample_rate, fft_size),
            max_columns: max_columns.max(1),
            columns: VecDeque::with_capacity(max_columns.max(1)),
            recycle: Vec::new(),
        }
    }

    pub fn default_for(sample_rate: u32) -> Self {
        Self::new(sample_rate, DEFAULT_FFT_SIZE, 512)
    }

    pub fn fft_size(&self) -> usize {
        self.spectrum.fft_size()
    }

    pub fn bin_frequency(&self, bin: usize) -> f32 {
        self.spectrum.bin_frequency(bin)
    }

    /// 喂入交错立体声；有新列时返回 true。
    pub fn process(&mut self, samples: &[f32]) -> bool {
        if !self.spectrum.process(samples) {
            return false;
        }
        let bins = self.spectrum.magnitudes_db();
        let mut col = self.recycle.pop().unwrap_or_else(|| vec![0.0; bins.len()]);
        col.clear();
        col.extend_from_slice(bins);
        if self.columns.len() >= self.max_columns
            && let Some(old) = self.columns.pop_front()
        {
            self.recycle.push(old);
        }
        self.columns.push_back(col);
        true
    }

    pub fn columns(&self) -> &VecDeque<Vec<f32>> {
        &self.columns
    }

    pub fn reset(&mut self) {
        self.spectrum.reset();
        while let Some(col) = self.columns.pop_front() {
            self.recycle.push(col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_bounded_history() {
        let mut a = SpectrogramAnalyzer::new(48_000, 1024, 8);
        let mut samples = Vec::new();
        for i in 0..48_000 {
            let s = (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin() * 0.5;
            samples.push(s);
            samples.push(s);
        }
        for chunk in samples.chunks(480) {
            a.process(chunk);
        }
        assert!(!a.columns().is_empty());
        assert!(a.columns().len() <= 8);
        assert_eq!(
            a.columns().front().map(Vec::len),
            Some(a.fft_size() / 2 + 1)
        );
    }
}
