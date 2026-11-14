//! 波形历史：按列保存 min/max，供滚动包络显示。

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaveColumn {
    pub min_l: f32,
    pub max_l: f32,
    pub min_r: f32,
    pub max_r: f32,
}

impl WaveColumn {
    fn from_first(l: f32, r: f32) -> Self {
        Self {
            min_l: l,
            max_l: l,
            min_r: r,
            max_r: r,
        }
    }

    fn push(&mut self, l: f32, r: f32) {
        self.min_l = self.min_l.min(l);
        self.max_l = self.max_l.max(l);
        self.min_r = self.min_r.min(r);
        self.max_r = self.max_r.max(r);
    }
}

pub struct WaveformHistory {
    max_columns: usize,
    samples_per_column: usize,
    columns: VecDeque<WaveColumn>,
    current: Option<WaveColumn>,
    count: usize,
}

impl WaveformHistory {
    pub fn new(sample_rate: u32, window_seconds: f32, max_columns: usize) -> Self {
        let max_columns = max_columns.max(1);
        let samples_per_column =
            ((sample_rate as f32 * window_seconds) / max_columns as f32).round() as usize;
        Self {
            max_columns,
            samples_per_column: samples_per_column.max(1),
            columns: VecDeque::with_capacity(max_columns),
            current: None,
            count: 0,
        }
    }

    pub fn default_for(sample_rate: u32) -> Self {
        Self::new(sample_rate, 5.0, 512)
    }

    pub fn process(&mut self, samples: &[f32]) {
        for frame in samples.as_chunks::<2>().0 {
            let l = frame[0];
            let r = frame[1];
            match self.current.as_mut() {
                Some(col) => col.push(l, r),
                None => self.current = Some(WaveColumn::from_first(l, r)),
            }
            self.count += 1;
            if self.count >= self.samples_per_column {
                if let Some(col) = self.current.take() {
                    if self.columns.len() >= self.max_columns {
                        self.columns.pop_front();
                    }
                    self.columns.push_back(col);
                }
                self.count = 0;
            }
        }
    }

    pub fn columns(&self) -> &VecDeque<WaveColumn> {
        &self.columns
    }

    pub fn reset(&mut self) {
        self.columns.clear();
        self.current = None;
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_into_columns_and_keeps_bounds() {
        let mut h = WaveformHistory::new(48_000, 0.01, 2);
        // 每列 240 帧；喂 480 帧 → 两列。
        let mut samples = Vec::new();
        for i in 0..480 {
            let s = if i < 240 { 0.5 } else { -0.5 };
            samples.push(s);
            samples.push(s);
        }
        h.process(&samples);
        assert_eq!(h.columns().len(), 2);
        assert_eq!(h.columns()[0].max_l, 0.5);
        assert_eq!(h.columns()[1].min_l, -0.5);
    }

    #[test]
    fn old_columns_are_dropped() {
        let mut h = WaveformHistory::new(48_000, 0.01, 2);
        for _ in 0..3 {
            h.process(&vec![0.1; 480]);
        }
        assert_eq!(h.columns().len(), 2);
    }
}
