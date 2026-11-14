//! 示波器环形缓冲：保留最近若干帧，并提供零交叉触发快照。

/// 固定容量的立体声环形缓冲（交错存储）。
pub struct ScopeBuffer {
    data: Vec<f32>,
    capacity_frames: usize,
    write: usize,
    filled: usize,
}

impl ScopeBuffer {
    pub fn new(capacity_frames: usize) -> Self {
        let capacity_frames = capacity_frames.max(1);
        Self {
            data: vec![0.0; capacity_frames * 2],
            capacity_frames,
            write: 0,
            filled: 0,
        }
    }

    /// 默认保留约 100ms（足够 20ms 窗口 + 触发搜索）。
    pub fn default_for(sample_rate: u32) -> Self {
        Self::new((sample_rate as usize / 10).max(1024))
    }

    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.as_chunks::<2>().0 {
            let i = self.write * 2;
            self.data[i] = frame[0];
            self.data[i + 1] = frame[1];
            self.write = (self.write + 1) % self.capacity_frames;
            self.filled = (self.filled + 1).min(self.capacity_frames);
        }
    }

    pub fn filled_frames(&self) -> usize {
        self.filled
    }

    /// 复制最近 `frames` 帧（按时间顺序，交错）。返回实际复制帧数。
    pub fn snapshot(&self, frames: usize, out: &mut Vec<f32>) -> usize {
        out.clear();
        let frames = frames.min(self.filled);
        if frames == 0 {
            return 0;
        }
        let start = (self.write + self.capacity_frames - frames) % self.capacity_frames;
        out.reserve(frames * 2);
        for k in 0..frames {
            let i = ((start + k) % self.capacity_frames) * 2;
            out.push(self.data[i]);
            out.push(self.data[i + 1]);
        }
        frames
    }

    /// 从最近窗口内找第一个上升零交叉，再从该点复制 `frames` 帧。
    /// 找不到交叉时退化为普通 `snapshot`。
    pub fn triggered_snapshot(&self, frames: usize, out: &mut Vec<f32>) -> usize {
        out.clear();
        let frames = frames.min(self.filled);
        if frames < 4 {
            return self.snapshot(frames, out);
        }
        let start = (self.write + self.capacity_frames - frames) % self.capacity_frames;
        let mono = |k: usize| {
            let i = ((start + k) % self.capacity_frames) * 2;
            (self.data[i] + self.data[i + 1]) * 0.5
        };
        let mut trigger = None;
        for k in 1..frames {
            if mono(k - 1) <= 0.0 && mono(k) > 0.0 {
                trigger = Some(k);
                break;
            }
        }
        let Some(trigger) = trigger else {
            return self.snapshot(frames, out);
        };
        // 从触发点开始取；到窗口尾部后回绕到窗口开头补齐。
        out.reserve(frames * 2);
        for k in 0..frames {
            let source = (trigger + k) % frames;
            let i = ((start + source) % self.capacity_frames) * 2;
            out.push(self.data[i]);
            out.push(self.data[i + 1]);
        }
        frames
    }

    pub fn reset(&mut self) {
        self.write = 0;
        self.filled = 0;
        self.data.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_chronological() {
        let mut b = ScopeBuffer::new(8);
        b.push(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
        let mut out = Vec::new();
        assert_eq!(b.snapshot(2, &mut out), 2);
        assert_eq!(out, vec![2.0, -2.0, 3.0, -3.0]);
    }

    #[test]
    fn wraps_around() {
        let mut b = ScopeBuffer::new(4);
        b.push(&[1.0; 2]);
        b.push(&[2.0; 2]);
        b.push(&[3.0; 2]);
        let mut out = Vec::new();
        assert_eq!(b.snapshot(3, &mut out), 3);
        assert_eq!(out, vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn trigger_finds_rising_zero_crossing() {
        let mut b = ScopeBuffer::new(16);
        // -1,-1, +1,+1, +0.5,+0.5, -0.5,-0.5
        b.push(&[-1.0, -1.0, 1.0, 1.0, 0.5, 0.5, -0.5, -0.5]);
        let mut out = Vec::new();
        assert_eq!(b.triggered_snapshot(4, &mut out), 4);
        // 触发点应落在 +1 那一帧，窗口从该处开始。
        assert_eq!(out.first().copied(), Some(1.0));
    }
}
