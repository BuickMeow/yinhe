//! 峰值电平表。
//!
//! 渲染线程每块写入一次峰值（post-fader），UI 线程每帧读取并自己做视觉衰减。
//! 通信模式照搬 yinhe-audio 的 `sample_position: Arc<AtomicU64>`。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// 渲染线程侧的表头。存 f32 bits（峰值，非负）。
#[derive(Clone)]
pub struct MeterTap {
    peak_l: Arc<AtomicU32>,
    peak_r: Arc<AtomicU32>,
}

impl MeterTap {
    pub fn new() -> (Self, MeterReading) {
        let peak_l = Arc::new(AtomicU32::new(0));
        let peak_r = Arc::new(AtomicU32::new(0));
        let tap = Self {
            peak_l: Arc::clone(&peak_l),
            peak_r: Arc::clone(&peak_r),
        };
        (tap, MeterReading { peak_l, peak_r })
    }

    /// 渲染线程调用：发布一段已处理音频的峰值。
    pub(crate) fn publish(&self, left: &[f32], right: &[f32]) {
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        for (&l, &r) in left.iter().zip(right.iter()) {
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
        }
        self.peak_l.store(peak_l.to_bits(), Ordering::Relaxed);
        self.peak_r.store(peak_r.to_bits(), Ordering::Relaxed);
    }
}

/// UI 线程侧的读数端。
#[derive(Clone)]
pub struct MeterReading {
    peak_l: Arc<AtomicU32>,
    peak_r: Arc<AtomicU32>,
}

impl MeterReading {
    /// 读取最近一块的峰值（L, R），范围 [0, +∞)。
    pub fn read(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_l.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_r.load(Ordering::Relaxed)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_publishes_peak() {
        let (tap, reading) = MeterTap::new();
        tap.publish(&[-0.5, 0.25, 0.1], &[0.0, -0.75, 0.3]);
        let (l, r) = reading.read();
        assert_eq!(l, 0.5);
        assert_eq!(r, 0.75);
    }
}
