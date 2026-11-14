//! 峰值电平表。
//!
//! 渲染线程每块写入一次峰值（post-fader），UI 线程每帧读取并自己做视觉衰减。
//! 通信模式照搬 yinhe-audio 的 `sample_position: Arc<AtomicU64>`。

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

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

    #[test]
    fn sample_tap_preserves_stereo_interleave() {
        let (mut tap, reading) = SampleTap::new();
        tap.publish(&[0.1, 0.2], &[-0.1, -0.2]);
        let mut out = Vec::new();
        assert_eq!(reading.drain_into(&mut out), 4);
        assert_eq!(out, vec![0.1, -0.1, 0.2, -0.2]);
    }

    #[test]
    fn sample_tap_drops_whole_block_when_full() {
        let (mut tap, reading) = SampleTap::new();
        // 容量 128Ki float；塞满后下一块必须整块丢弃，不能只写左声道。
        let block = vec![1.0f32; MASTER_SAMPLE_CAPACITY / 2];
        tap.publish(&block, &block);
        assert_eq!(reading.dropped_blocks(), 0);
        tap.publish(&[2.0], &[3.0]);
        assert_eq!(reading.dropped_blocks(), 1);
        let mut out = Vec::new();
        let n = reading.drain_into(&mut out);
        assert_eq!(n, MASTER_SAMPLE_CAPACITY);
        assert!(out.chunks_exact(2).all(|p| p == [1.0, 1.0]));
    }

    #[test]
    fn sample_tap_disabled_publishes_nothing() {
        let (mut tap, reading) = SampleTap::new();
        tap.set_enabled(false);
        tap.publish(&[0.5], &[0.5]);
        let mut out = Vec::new();
        assert_eq!(reading.drain_into(&mut out), 0);
    }
}

/// Master 样本 tap 的 ring 容量（交错立体声 float）。
///
/// 128 Ki floats = 64 Ki 帧 ≈ 1.36s @48kHz。UI 被卡住超过这个深度才会丢块，
/// 普通一帧（~800 帧）远远用不完。固定容量不需要知道采样率。
const MASTER_SAMPLE_CAPACITY: usize = 1 << 17;

/// 渲染线程侧的 Master 样本发布端。
///
/// 只做一次无锁拷贝：ring 空间不够时**整块丢弃**，绝不阻塞；UI 线程消费端
/// 看到的是连续的块，不会出现左右声道错位。
pub struct SampleTap {
    producer: rtrb::Producer<f32>,
    enabled: Arc<AtomicBool>,
    dropped_blocks: Arc<AtomicU64>,
}

impl SampleTap {
    pub fn new() -> (Self, SampleReading) {
        let (producer, consumer) = rtrb::RingBuffer::new(MASTER_SAMPLE_CAPACITY);
        let enabled = Arc::new(AtomicBool::new(true));
        let dropped_blocks = Arc::new(AtomicU64::new(0));
        (
            Self {
                producer,
                enabled: Arc::clone(&enabled),
                dropped_blocks: Arc::clone(&dropped_blocks),
            },
            SampleReading {
                consumer: Arc::new(Mutex::new(consumer)),
                enabled,
                dropped_blocks,
            },
        )
    }

    /// 导出时关闭，避免离线渲染喂实时表。
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// 发布一整块立体声（交错写入 L,R,L,R...）。空间不够时整块丢弃。
    pub fn publish(&mut self, left: &[f32], right: &[f32]) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let frames = left.len().min(right.len());
        let needed = frames * 2;
        if needed == 0 {
            return;
        }
        // 必须能放下整块：否则左右声道会在 ring 里错位。
        if self.producer.slots() < needed {
            self.dropped_blocks.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for i in 0..frames {
            if self.producer.push(left[i]).is_err() {
                break;
            }
            if self.producer.push(right[i]).is_err() {
                break;
            }
        }
    }
}

/// UI 线程侧的样本读取端。内部锁只在 UI 线程争用，音频线程完全不碰。
#[derive(Clone)]
pub struct SampleReading {
    consumer: Arc<Mutex<rtrb::Consumer<f32>>>,
    enabled: Arc<AtomicBool>,
    dropped_blocks: Arc<AtomicU64>,
}

impl SampleReading {
    /// 把当前所有可用样本追加到 `out`（交错立体声）。返回读取的 float 数。
    pub fn drain_into(&self, out: &mut Vec<f32>) -> usize {
        let mut consumer = self.consumer.lock().unwrap_or_else(|e| e.into_inner());
        let mut n = 0usize;
        while let Ok(sample) = consumer.pop() {
            out.push(sample);
            n += 1;
        }
        n
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// 因 UI 太慢而整块丢弃的次数（诊断用）。
    pub fn dropped_blocks(&self) -> u64 {
        self.dropped_blocks.load(Ordering::Relaxed)
    }
}
