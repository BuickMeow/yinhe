//! 峰值限幅器（自实现，逻辑与 xsynth `VolumeLimiter` 完全一致）。

struct SingleChannelLimiter {
    loudness: f32,
    attack: f32,
    falloff: f32,
    strength: f32,
    min_thresh: f32,
}

impl SingleChannelLimiter {
    fn new() -> Self {
        Self {
            loudness: 1.0,
            attack: 100.0,
            falloff: 16000.0,
            strength: 1.0,
            min_thresh: 1.0,
        }
    }

    fn limit(&mut self, val: f32) -> f32 {
        let abs = val.abs();
        if self.loudness > abs {
            self.loudness = (self.loudness * self.falloff + abs) / (self.falloff + 1.0);
        } else {
            self.loudness = (self.loudness * self.attack + abs) / (self.attack + 1.0);
        }
        if self.loudness < self.min_thresh {
            self.loudness = self.min_thresh;
        }
        val / (self.loudness * self.strength + 2.0 * (1.0 - self.strength)) / 2.0
    }
}

/// 多通道峰值限幅器，防止削波。
pub struct VolumeLimiter {
    channels: Vec<SingleChannelLimiter>,
    channel_count: usize,
}

impl VolumeLimiter {
    pub fn new(channel_count: u16) -> Self {
        Self {
            channels: (0..channel_count)
                .map(|_| SingleChannelLimiter::new())
                .collect(),
            channel_count: channel_count as usize,
        }
    }

    pub fn limit(&mut self, sample: &mut [f32]) {
        for (i, s) in sample.iter_mut().enumerate() {
            *s = self.channels[i % self.channel_count].limit(*s);
        }
    }
}
