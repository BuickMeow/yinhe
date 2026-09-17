//! 输出前瞻峰值限幅（lookahead peak limiter）。
//!
//! 替换原 `tanh` 饱和器：tanh 数学上不削波，但多 voice 密集叠加时输入可达
//! 6~18（实测 88 键齐奏峰值 6.4、352 voice 17.9），深度饱和把波形压成近似
//! 方波（谐波失真，听感"爆音"）。前瞻限幅改用**线性增益**把峰值压到阈值
//! 以下：稳态大信号只是被固定衰减（无谐波），只有增益变化瞬间有轻微调制。
//!
//! 结构：
//! - `delay`：`lookahead` 帧延迟线（信号延迟，给增益下调留提前量）；
//! - `peaks`：滑动窗口最大峰值的单调队列（窗口 = 延迟长度）。增益按
//!   **窗口内最大峰值**计算，保证峰值样本输出时对应的峰值仍在窗口内——
//!   这是"输出严格不超阈值"的关键（只看瞬时峰值会在峰值过后立即释放，
//!   窗口内的输出就会超）；
//! - 目标增益 `min(1, threshold/peak)`：下降立即（样本还没输出，不会削峰），
//!   上升按指数时间常数恢复（避免喘息 pumping）；
//! - `threshold` 是限幅器本质参数（峰值上限），不是特性开关；判定用连续
//!   的 `min` 表达，没有"按阈值分区间做不同事"。`peak = 0` 时
//!   `threshold / 0 = ∞`，`min(1.0, ∞) = 1.0` 自然安全。

use std::collections::VecDeque;

/// 前瞻时长（秒）：3ms 短于典型块长（512 帧 ≈ 10.7ms @48k），延迟不可闻。
const LOOKAHEAD_SECONDS: f32 = 0.003;
/// 峰值阈值（线性）：留约 0.5dB headroom。
const THRESHOLD: f32 = 0.95;
/// 增益恢复（释放）时间常数（秒）：150ms 恢复约 63%，避免喘息。
const RELEASE_SECONDS: f32 = 0.15;

/// 立体声前瞻限幅器（跨调用有状态，采样率在构造时绑定）。
pub struct VolumeLimiter {
    /// 延迟线（交错立体声，长度 = lookahead 帧 × 2）。
    delay: Vec<f32>,
    /// 环形写指针（帧）。
    write: usize,
    /// 当前线性增益。
    gain: f32,
    /// 每帧释放系数（`gain += (1 - gain) * release`）。
    release: f32,
    /// 滑动窗口最大峰值：单调队列（帧序号递增、峰值递减）。
    peaks: VecDeque<(usize, f32)>,
    /// 全局帧计数（窗口过期判定）。
    frame_index: usize,
}

impl VolumeLimiter {
    pub fn new(sample_rate: u32) -> Self {
        let frames = ((sample_rate as f32 * LOOKAHEAD_SECONDS) as usize).max(1);
        let release = 1.0 - (-1.0 / (RELEASE_SECONDS * sample_rate as f32)).exp();
        Self {
            delay: vec![0.0; frames * 2],
            write: 0,
            gain: 1.0,
            release,
            peaks: VecDeque::with_capacity(frames + 1),
            frame_index: 0,
        }
    }

    /// 前瞻帧数（延迟补偿/导出收尾 flush 用）。
    pub fn latency_frames(&self) -> usize {
        self.delay.len() / 2
    }

    /// 限幅交错立体声缓冲（原地）。
    pub fn limit(&mut self, sample: &mut [f32]) {
        let frames = self.latency_frames();
        for frame in sample.chunks_exact_mut(2) {
            let in_l = frame[0];
            let in_r = frame[1];

            // 读延迟线（最老样本）后写入当前输入；out[i] 对应 in[i - latency]
            let out_l = self.delay[self.write * 2];
            let out_r = self.delay[self.write * 2 + 1];
            self.delay[self.write * 2] = in_l;
            self.delay[self.write * 2 + 1] = in_r;
            self.write += 1;
            if self.write == frames {
                self.write = 0;
            }

            // 滑动窗口峰值：当前峰值入队前，先清掉过期与更小的旧峰值
            let peak = in_l.abs().max(in_r.abs());
            while let Some(&(idx, _)) = self.peaks.front() {
                if self.frame_index > idx + frames {
                    self.peaks.pop_front();
                } else {
                    break;
                }
            }
            while let Some(&(_, p)) = self.peaks.back() {
                if p <= peak {
                    self.peaks.pop_back();
                } else {
                    break;
                }
            }
            self.peaks.push_back((self.frame_index, peak));
            let window_peak = self.peaks.front().map_or(0.0, |&(_, p)| p);

            let target = (THRESHOLD / window_peak).min(1.0);
            if target < self.gain {
                // 攻击：立即下调（延迟保证峰值输出时增益已就位）
                self.gain = target;
            } else {
                // 释放：向目标（≤ 1.0）指数恢复
                self.gain = (self.gain + (1.0 - self.gain) * self.release).min(target);
            }

            frame[0] = out_l * self.gain;
            frame[1] = out_r * self.gain;
            self.frame_index += 1;
        }
    }

    /// 导出收尾：输出延迟线残留（按当前增益），返回写入帧数。
    ///
    /// 实时流式播放不需要（延迟线持续被后续块填充）；导出结束时延迟线里
    /// 还剩最后 `latency_frames()` 帧未输出，不补会造成末尾缺一小段。
    pub fn flush(&mut self, out: &mut [f32]) -> usize {
        let frames = self.latency_frames().min(out.len() / 2);
        for i in 0..frames {
            let idx = (self.write + i) % self.latency_frames();
            out[i * 2] = self.delay[idx * 2] * self.gain;
            out[i * 2 + 1] = self.delay[idx * 2 + 1] * self.gain;
        }
        self.delay.fill(0.0);
        self.write = 0;
        self.peaks.clear();
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// 稳态大信号：输出峰值不超过阈值，且增益收敛到 threshold/peak。
    #[test]
    fn steady_loud_signal_is_capped() {
        let mut limiter = VolumeLimiter::new(SR);
        let frames = 4_800;
        let amp = 6.4f32;
        let mut buf = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = amp * (i as f32 * 0.1).sin();
            buf.push(s);
            buf.push(s);
        }
        limiter.limit(&mut buf);
        // 跳过前 2×lookahead（延迟线填充 + 增益就位），统计稳态
        let skip = limiter.latency_frames() * 4 * 2;
        let peak = buf[skip..].iter().fold(0f32, |m, &v| m.max(v.abs()));
        assert!(peak <= THRESHOLD + 1e-4, "峰值 {peak} 超过阈值");
        let expected = THRESHOLD / amp;
        assert!(
            (limiter.gain - expected).abs() < 1e-3,
            "增益 {} 未收敛到 {expected}",
            limiter.gain
        );
    }

    /// 小信号线性直通（无增益变化）。
    #[test]
    fn quiet_signal_is_transparent() {
        let mut limiter = VolumeLimiter::new(SR);
        let frames = 1_000;
        let amp = 0.3f32;
        let mut buf: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = amp * (i as f32 * 0.05).sin();
                [s, s]
            })
            .collect();
        let original = buf.clone();
        limiter.limit(&mut buf);
        let latency = limiter.latency_frames();
        assert_eq!(limiter.gain, 1.0, "小信号不应改变增益");
        // 延迟后逐样本一致
        for i in latency..frames {
            assert_eq!(buf[i * 2], original[(i - latency) * 2]);
            assert_eq!(buf[i * 2 + 1], original[(i - latency) * 2 + 1]);
        }
    }

    /// 峰值过后增益按时间常数恢复，而不是立刻跳回。
    ///
    /// 峰值在 lookahead 窗口内保持（窗口内不释放），窗口滑出后才开始恢复。
    #[test]
    fn gain_recovers_slowly_after_peak() {
        let mut limiter = VolumeLimiter::new(SR);
        let latency = limiter.latency_frames();
        // 1 帧尖峰 + 静音
        let mut buf = vec![0.0f32; (latency + 10) * 2];
        buf[0] = 10.0;
        buf[1] = 10.0;
        limiter.limit(&mut buf);
        let gain_after_peak = limiter.gain;
        assert!(gain_after_peak < 0.2, "尖峰后增益应大幅下调");
        // 再过 1ms 静音：窗口已滑出（latency + 10 > latency），释放时间常数
        // 150ms → 恢复很少
        let mut quiet = vec![0.0f32; (SR / 1000) as usize * 2];
        limiter.limit(&mut quiet);
        assert!(
            limiter.gain > gain_after_peak && limiter.gain < 0.3,
            "增益应缓慢恢复（当前 {}）",
            limiter.gain
        );
    }

    /// flush 输出延迟线残留的帧数。
    #[test]
    fn flush_returns_pending_frames() {
        let mut limiter = VolumeLimiter::new(SR);
        let latency = limiter.latency_frames();
        assert!(latency > 0);
        let mut buf = vec![0.5f32; latency * 2];
        limiter.limit(&mut buf);
        assert_eq!(limiter.gain, 1.0);
        let mut out = vec![0.0f32; latency * 2];
        let frames = limiter.flush(&mut out);
        assert_eq!(frames, latency);
        // 残留样本 = 输入前 latency 帧的 0.5（乘增益 1.0）
        assert!(out.iter().all(|&v| (v - 0.5).abs() < 1e-6));
        // flush 后延迟线清空
        assert_eq!(limiter.flush(&mut out), latency);
        assert!(out.iter().all(|&v| v == 0.0));
    }
}
