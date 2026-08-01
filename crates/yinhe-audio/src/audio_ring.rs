use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Inner {
    data: Box<[UnsafeCell<f32>]>,
    capacity: usize,
    read: AtomicUsize,
    write: AtomicUsize,
}

unsafe impl Sync for Inner {}

/// Single-producer/single-consumer audio ring buffer.
///
/// Capacity must be a power of two. The producer and consumer indices are
/// monotonically increasing counters; wrapping is done only when indexing.
pub(crate) struct AudioRing {
    inner: Arc<Inner>,
}

pub(crate) struct AudioRingProducer {
    inner: Arc<Inner>,
}

pub(crate) struct AudioRingConsumer {
    inner: Arc<Inner>,
}

impl AudioRing {
    pub(crate) fn new(capacity: usize) -> Self {
        // 调用方（spawn.rs::RING_CAPACITY）已经在编译期用 const 断言保证 power-of-two，
        // 这里仅保留 dev 构建下的防御性检查；release 下为零开销。
        debug_assert!(capacity.is_power_of_two());
        debug_assert!(capacity > 0);
        let data = (0..capacity)
            .map(|_| UnsafeCell::new(0.0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            inner: Arc::new(Inner {
                data,
                capacity,
                read: AtomicUsize::new(0),
                write: AtomicUsize::new(0),
            }),
        }
    }

    pub(crate) fn split(self) -> (AudioRingProducer, AudioRingConsumer) {
        (
            AudioRingProducer {
                inner: Arc::clone(&self.inner),
            },
            AudioRingConsumer { inner: self.inner },
        )
    }
}

impl AudioRingProducer {
    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        self.inner.capacity
    }

    /// 当前写入计数（单调递增，不取模）。
    /// 配合 `AudioRingConsumer::discard_before` 实现竞态安全的"清空"：
    /// 以此刻写入计数为边界，之后推入的音频全部保留。
    #[inline]
    pub(crate) fn write_position(&self) -> usize {
        self.inner.write.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        let read = self.inner.read.load(Ordering::Acquire);
        let write = self.inner.write.load(Ordering::Relaxed);
        write.wrapping_sub(read)
    }

    #[inline]
    pub(crate) fn free_space(&self) -> usize {
        self.capacity().saturating_sub(self.len())
    }

    pub(crate) fn push_slice(&mut self, input: &[f32]) -> usize {
        let read = self.inner.read.load(Ordering::Acquire);
        let write = self.inner.write.load(Ordering::Relaxed);
        let available = self.inner.capacity - write.wrapping_sub(read);
        let count = input.len().min(available);
        if count == 0 {
            return 0;
        }

        unsafe {
            copy_into_ring(&self.inner, write, &input[..count]);
        }
        self.inner.write.store(write.wrapping_add(count), Ordering::Release);
        count
    }
}

impl AudioRingConsumer {
    pub(crate) fn pop_into(&mut self, output: &mut [f32]) -> usize {
        let write = self.inner.write.load(Ordering::Acquire);
        let read = self.inner.read.load(Ordering::Relaxed);
        let available = write.wrapping_sub(read);
        let count = output.len().min(available);
        if count == 0 {
            return 0;
        }

        unsafe {
            copy_from_ring(&self.inner, read, &mut output[..count]);
        }
        self.inner.read.store(read.wrapping_add(count), Ordering::Release);
        count
    }

    /// 丢弃 `write_at_clear` 之前的所有缓冲内容，保留之后推入的音频。
    ///
    /// `write_at_clear` 取自已清空瞬间的 `AudioRingProducer::write_position()`。
    /// seek/play 存在竞态：渲染器可能在 cpal 回调 ack 之前就把新音频推入 ring，
    /// 此时整体 clear 会把新播放位置的开头一起丢掉（第二次播放开头缺失的根因）。
    /// 改为只丢弃边界前的内容，边界后的新音频原样保留。
    pub(crate) fn discard_before(&mut self, write_at_clear: usize) {
        let read = self.inner.read.load(Ordering::Relaxed);
        let stale = write_at_clear.wrapping_sub(read);
        self.inner.read.store(read.wrapping_add(stale), Ordering::Release);
    }
}

unsafe fn copy_into_ring(inner: &Inner, start: usize, input: &[f32]) {
    let mask = inner.capacity - 1;
    for (offset, &sample) in input.iter().enumerate() {
        let index = (start + offset) & mask;
        unsafe {
            *inner.data[index].get() = sample;
        }
    }
}

unsafe fn copy_from_ring(inner: &Inner, start: usize, output: &mut [f32]) {
    let mask = inner.capacity - 1;
    for (offset, sample) in output.iter_mut().enumerate() {
        let index = (start + offset) & mask;
        unsafe {
            *sample = *inner.data[index].get();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_pop_preserves_order() {
        let (mut producer, mut consumer) = AudioRing::new(8).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0]), 3);

        let mut out = [0.0; 3];
        assert_eq!(consumer.pop_into(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn wraps_around() {
        let (mut producer, mut consumer) = AudioRing::new(4).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0, 4.0]), 4);

        let mut out = [0.0; 3];
        assert_eq!(consumer.pop_into(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);

        assert_eq!(producer.push_slice(&[5.0, 6.0, 7.0]), 3);
        let mut rest = [0.0; 4];
        assert_eq!(consumer.pop_into(&mut rest), 4);
        assert_eq!(rest, [4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn does_not_overwrite_unread_samples() {
        let (mut producer, mut consumer) = AudioRing::new(4).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0, 4.0, 5.0]), 4);
        assert_eq!(producer.push_slice(&[6.0]), 0);

        let mut out = [0.0; 4];
        assert_eq!(consumer.pop_into(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn discard_before_keeps_audio_pushed_after_marker() {
        // 模拟 seek/play 竞态：清空标记之后渲染器已推入新音频，
        // discard_before 必须保留新音频、只丢弃旧内容。
        let (mut producer, mut consumer) = AudioRing::new(8).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0]), 3); // 旧音频（seek 前）
        let marker = producer.write_position(); // 清空瞬间
        assert_eq!(producer.push_slice(&[4.0, 5.0]), 2); // 竞态窗口内推入的新音频

        consumer.discard_before(marker);

        let mut out = [0.0; 4];
        assert_eq!(consumer.pop_into(&mut out), 2);
        assert_eq!(&out[..2], &[4.0, 5.0]);
    }

    #[test]
    fn discard_before_without_new_audio_empties_ring() {
        // 清空后还没来得及推入新音频（模型加载慢的路径）：全部丢弃。
        let (mut producer, mut consumer) = AudioRing::new(8).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0]), 3);
        let marker = producer.write_position();

        consumer.discard_before(marker);

        let mut out = [0.0; 3];
        assert_eq!(consumer.pop_into(&mut out), 0);
        assert_eq!(producer.push_slice(&[6.0]), 1);
        assert_eq!(consumer.pop_into(&mut out[..1]), 1);
        assert_eq!(out[0], 6.0);
    }

    #[test]
    fn discard_before_handles_wrap_around() {
        let (mut producer, mut consumer) = AudioRing::new(4).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0, 4.0]), 4);
        let mut out = [0.0; 3];
        assert_eq!(consumer.pop_into(&mut out), 3); // read=3，剩 [4.0]
        let marker = producer.write_position(); // write=4
        assert_eq!(producer.push_slice(&[5.0, 6.0, 7.0]), 3); // write=7，绕回

        consumer.discard_before(marker);

        assert_eq!(consumer.pop_into(&mut out), 3);
        assert_eq!(&out, &[5.0, 6.0, 7.0]);
    }
}
