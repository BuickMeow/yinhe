//! SoA 声部池：voice 间 SIMD 内核（一个 lane = 一个 voice）。
//!
//! 布局与对齐：
//! - 所有字段数组等长（容量），活跃 voice 连续存放在 `0..len`；
//! - 渲染按 native 宽度（4/8/16）整块处理，范围向上取整到宽度，尾部
//!   lane 为哑 voice（`env_stage = ENV_FINISHED`、幅度 0），参与计算但
//!   结果无害，因此热循环内没有 tail 分支；
//! - 状态数组用 `Vec<f32>` 而不是 `[f32; 8]`：native 宽度由运行时
//!   SIMD 等级决定（SSE2/NEON 4、AVX2 8、AVX-512 16），固定块宽会浪费
//!   一半寄存器宽度。
//!
//! 为什么是 voice 间而非帧内（xsynth 做法）：
//! - 每 lane 内部仍逐帧串行，浮点运算顺序与 AoS 实现逐位一致（parity 友好）；
//! - 包络递推、biquad 反馈、采样位置累加这些"逐帧依赖"在 lane 间互相
//!   独立，可以真正并行；xsynth 的帧内 SIMD 在这些部分仍是标量串行。
//!
//! 本模块正在分批移植（先包络与状态池骨架，再采样/滤波/生命周期），
//! 接入渲染路径后移除 `#[allow(dead_code)]`。

use fearless_simd::{Select, Simd, SimdBase};

use super::simd::powi8_neg;

/// 包络结束阶段（与 `voice.rs` 的 `ENV_FINISHED` 一致）。
pub const ENV_FINISHED: f32 = 6.0;

/// lane 对齐：AVX-512 的 f32 native 宽度为 16，容量按此对齐。
pub const LANES_ALIGN: usize = 16;

/// SoA 声部池。
pub struct VoiceSoa {
    /// 活跃 voice 数（有效 slot 为 `0..len`）。
    len: usize,

    // ── 包络每帧状态 ──
    /// 当前包络幅度。
    pub envelope: Vec<f32>,
    /// 包络阶段（0..=6；用 f32 存整数值，向量比较/选择成本最低）。
    pub env_stage: Vec<f32>,
    /// 当前阶段已推进帧数。
    pub stage_progress: Vec<f32>,
    /// Attack/Release 阶段的起点幅度。
    pub env_start: Vec<f32>,
    /// Hold→Decay 时快照的幅度。
    pub decay_start: Vec<f32>,
    // ── 包络参数（per-lane；CC72/73 会改）──
    /// Decay 目标电平。
    pub sustain_level: Vec<f32>,
    /// Sustain 的 peak（当前恒 1.0，保留字段与标量语义一致）。
    pub env_level: Vec<f32>,
    pub delay_frames: Vec<f32>,
    pub attack_frames: Vec<f32>,
    pub hold_frames: Vec<f32>,
    pub decay_frames: Vec<f32>,
    pub release_frames: Vec<f32>,
}

impl Default for VoiceSoa {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceSoa {
    pub fn new() -> Self {
        Self {
            len: 0,
            envelope: Vec::new(),
            env_stage: Vec::new(),
            stage_progress: Vec::new(),
            env_start: Vec::new(),
            decay_start: Vec::new(),
            sustain_level: Vec::new(),
            env_level: Vec::new(),
            delay_frames: Vec::new(),
            attack_frames: Vec::new(),
            hold_frames: Vec::new(),
            decay_frames: Vec::new(),
            release_frames: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 容量（`LANES_ALIGN` 的倍数）。
    pub fn capacity(&self) -> usize {
        self.env_stage.len()
    }

    /// 分配一个 slot（初始化哑状态）并返回其下标。
    pub fn push_lane(&mut self) -> usize {
        if self.len == self.capacity() {
            self.grow();
        }
        let slot = self.len;
        self.reset_lane(slot);
        self.len += 1;
        slot
    }

    /// 清空全部 voice（保留容量）。
    pub fn clear(&mut self) {
        for slot in 0..self.len {
            self.reset_lane(slot);
        }
        self.len = 0;
    }

    /// 把 slot 重置为哑 voice 状态。
    fn reset_lane(&mut self, slot: usize) {
        self.envelope[slot] = 0.0;
        self.env_stage[slot] = ENV_FINISHED;
        self.stage_progress[slot] = 0.0;
        self.env_start[slot] = 0.0;
        self.decay_start[slot] = 0.0;
        self.sustain_level[slot] = 0.0;
        self.env_level[slot] = 1.0;
        self.delay_frames[slot] = 0.0;
        self.attack_frames[slot] = 0.0;
        self.hold_frames[slot] = 0.0;
        self.decay_frames[slot] = 0.0;
        self.release_frames[slot] = 0.0;
    }

    /// 扩容（翻倍，最小 `LANES_ALIGN`），新增 slot 为哑 voice。
    fn grow(&mut self) {
        let new_cap = (self.capacity() * 2).max(LANES_ALIGN);
        let dumb_stage = ENV_FINISHED;
        let dumb_one = 1.0;
        self.envelope.resize(new_cap, 0.0);
        self.env_stage.resize(new_cap, dumb_stage);
        self.stage_progress.resize(new_cap, 0.0);
        self.env_start.resize(new_cap, 0.0);
        self.decay_start.resize(new_cap, 0.0);
        self.sustain_level.resize(new_cap, 0.0);
        self.env_level.resize(new_cap, dumb_one);
        self.delay_frames.resize(new_cap, 0.0);
        self.attack_frames.resize(new_cap, 0.0);
        self.hold_frames.resize(new_cap, 0.0);
        self.decay_frames.resize(new_cap, 0.0);
        self.release_frames.resize(new_cap, 0.0);
    }

    /// 推进所有 lane 一帧包络（与 `CpuVoice::advance_env` 逐位等价）。
    ///
    /// branchless：同时计算所有阶段的候选值，用 lane 掩码选择；每个 lane
    /// 只命中一个阶段，因此各阶段之间无顺序依赖。
    #[inline(always)]
    pub fn advance_env<S: Simd>(&mut self, simd: S) {
        let width = S::f32s::LEN;
        let active_end = self.len.next_multiple_of(width);
        let mut start = 0;
        while start < active_end {
            let range = start..start + width;
            let stage = S::f32s::from_slice(simd, &self.env_stage[range.clone()]);
            let prog = S::f32s::from_slice(simd, &self.stage_progress[range.clone()]);
            let envelope = S::f32s::from_slice(simd, &self.envelope[range.clone()]);
            let env_start = S::f32s::from_slice(simd, &self.env_start[range.clone()]);
            let decay_start = S::f32s::from_slice(simd, &self.decay_start[range.clone()]);
            let sustain_level = S::f32s::from_slice(simd, &self.sustain_level[range.clone()]);
            let peak = S::f32s::from_slice(simd, &self.env_level[range.clone()]);
            let delay_frames = S::f32s::from_slice(simd, &self.delay_frames[range.clone()]);
            let attack_frames = S::f32s::from_slice(simd, &self.attack_frames[range.clone()]);
            let hold_frames = S::f32s::from_slice(simd, &self.hold_frames[range.clone()]);
            let decay_frames = S::f32s::from_slice(simd, &self.decay_frames[range.clone()]);
            let release_frames = S::f32s::from_slice(simd, &self.release_frames[range.clone()]);

            let one = S::f32s::splat(simd, 1.0);
            let two = S::f32s::splat(simd, 2.0);
            let three = S::f32s::splat(simd, 3.0);
            let four = S::f32s::splat(simd, 4.0);
            let six = S::f32s::splat(simd, 6.0);
            let zero = S::f32s::splat(simd, 0.0);

            let prog1 = prog + one;
            let sus = sustain_level * peak;

            // 各阶段候选值（公式与标量逐字一致）
            let attack_env = env_start + (peak - env_start) * (prog1 / attack_frames);
            let decay_env = sus + (decay_start - sus) * powi8_neg(simd, prog1 / decay_frames);
            let release_env = env_start * powi8_neg(simd, prog1 / release_frames);

            // 阶段完成判定 + lane 掩码
            let m0 = stage.simd_eq(0.0);
            let m1 = stage.simd_eq(1.0);
            let m2 = stage.simd_eq(2.0);
            let m3 = stage.simd_eq(3.0);
            let m4 = stage.simd_eq(4.0);
            let m5 = stage.simd_eq(5.0);
            let finished = stage.simd_ge(ENV_FINISHED);

            let done0 = m0 & prog1.simd_ge(delay_frames);
            let done1 = m1 & prog1.simd_ge(attack_frames);
            let done2 = m2 & prog1.simd_ge(hold_frames);
            let done3 = m3 & prog1.simd_ge(decay_frames);
            let done5 = m5 & prog1.simd_ge(release_frames);

            // 默认：stage 不变、progress 推进一帧、幅度不变、decay_start 不变
            let mut new_stage = stage;
            let mut new_prog = prog1;
            let mut new_env = envelope;
            let mut new_decay_start = decay_start;

            // 0 Delay：到期进 Attack（幅度不变）
            new_stage = done0.select(one, new_stage);
            new_prog = done0.select(zero, new_prog);

            // 1 Attack：线性；到期置 peak 进 Hold
            new_env = done1.select(peak, new_env);
            new_stage = done1.select(two, new_stage);
            new_prog = done1.select(zero, new_prog);
            new_env = (m1 & !done1).select(attack_env, new_env);

            // 2 Hold：到期快照 decay_start 进 Decay（幅度不变）
            new_stage = done2.select(three, new_stage);
            new_decay_start = done2.select(envelope, new_decay_start);
            new_prog = done2.select(zero, new_prog);

            // 3 Decay：指数；到期置 sus 进 Sustain
            new_env = done3.select(sus, new_env);
            new_stage = done3.select(four, new_stage);
            new_prog = done3.select(zero, new_prog);
            new_env = (m3 & !done3).select(decay_env, new_env);

            // 4 Sustain：幅度恒为 sus，progress 不推进
            new_env = m4.select(sus, new_env);

            // 5 Release：指数；到期归零并结束
            new_env = done5.select(zero, new_env);
            new_stage = done5.select(six, new_stage);
            new_prog = done5.select(zero, new_prog);
            new_env = (m5 & !done5).select(release_env, new_env);

            // 未命中任何阶段的 lane（Sustain 与 finished）不推进 progress
            let keep_prog = m4 | finished;
            new_prog = keep_prog.select(prog, new_prog);

            new_stage.store_slice(&mut self.env_stage[range.clone()]);
            new_prog.store_slice(&mut self.stage_progress[range.clone()]);
            new_env.store_slice(&mut self.envelope[range.clone()]);
            new_decay_start.store_slice(&mut self.decay_start[range.clone()]);

            start += width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fearless_simd::{Level, dispatch};

    /// 标量参照：逐字复制 `voice.rs` 的 `advance_env`（用 `powi(8)`）。
    #[derive(Clone, Copy)]
    struct ScalarEnv {
        envelope: f32,
        stage: f32,
        progress: f32,
        env_start: f32,
        decay_start: f32,
        sustain_level: f32,
        env_level: f32,
        delay_frames: f32,
        attack_frames: f32,
        hold_frames: f32,
        decay_frames: f32,
        release_frames: f32,
    }

    impl ScalarEnv {
        fn advance(&mut self) {
            if self.stage >= ENV_FINISHED {
                return;
            }
            let peak = self.env_level;
            let sus = self.sustain_level * peak;
            match self.stage as u32 {
                0 => {
                    if self.progress + 1.0 >= self.delay_frames {
                        self.stage = 1.0;
                        self.progress = 0.0;
                    } else {
                        self.progress += 1.0;
                    }
                }
                1 => {
                    let n = self.progress + 1.0;
                    if n >= self.attack_frames {
                        self.envelope = peak;
                        self.stage = 2.0;
                        self.progress = 0.0;
                    } else {
                        self.envelope =
                            self.env_start + (peak - self.env_start) * (n / self.attack_frames);
                        self.progress = n;
                    }
                }
                2 => {
                    if self.progress + 1.0 >= self.hold_frames {
                        self.stage = 3.0;
                        self.decay_start = self.envelope;
                        self.progress = 0.0;
                    } else {
                        self.progress += 1.0;
                    }
                }
                3 => {
                    let n = self.progress + 1.0;
                    if n >= self.decay_frames {
                        self.envelope = sus;
                        self.stage = 4.0;
                        self.progress = 0.0;
                    } else {
                        let t = n / self.decay_frames;
                        self.envelope = sus + (self.decay_start - sus) * (1.0 - t).powi(8);
                        self.progress = n;
                    }
                }
                4 => {
                    self.envelope = sus;
                }
                5 => {
                    let n = self.progress + 1.0;
                    if n >= self.release_frames {
                        self.envelope = 0.0;
                        self.stage = ENV_FINISHED;
                        self.progress = 0.0;
                    } else {
                        let t = n / self.release_frames;
                        self.envelope = self.env_start * (1.0 - t).powi(8);
                        self.progress = n;
                    }
                }
                _ => {}
            }
        }
    }

    fn push(soa: &mut VoiceSoa, env: &ScalarEnv) -> usize {
        let slot = soa.push_lane();
        soa.envelope[slot] = env.envelope;
        soa.env_stage[slot] = env.stage;
        soa.stage_progress[slot] = env.progress;
        soa.env_start[slot] = env.env_start;
        soa.decay_start[slot] = env.decay_start;
        soa.sustain_level[slot] = env.sustain_level;
        soa.env_level[slot] = env.env_level;
        soa.delay_frames[slot] = env.delay_frames;
        soa.attack_frames[slot] = env.attack_frames;
        soa.hold_frames[slot] = env.hold_frames;
        soa.decay_frames[slot] = env.decay_frames;
        soa.release_frames[slot] = env.release_frames;
        slot
    }

    fn read(soa: &VoiceSoa, slot: usize) -> ScalarEnv {
        ScalarEnv {
            envelope: soa.envelope[slot],
            stage: soa.env_stage[slot],
            progress: soa.stage_progress[slot],
            env_start: soa.env_start[slot],
            decay_start: soa.decay_start[slot],
            sustain_level: soa.sustain_level[slot],
            env_level: soa.env_level[slot],
            delay_frames: soa.delay_frames[slot],
            attack_frames: soa.attack_frames[slot],
            hold_frames: soa.hold_frames[slot],
            decay_frames: soa.decay_frames[slot],
            release_frames: soa.release_frames[slot],
        }
    }

    fn base(env_start: f32) -> ScalarEnv {
        ScalarEnv {
            envelope: env_start,
            stage: 0.0,
            progress: 0.0,
            env_start,
            decay_start: env_start,
            sustain_level: 0.7,
            env_level: 1.0,
            delay_frames: 3.0,
            attack_frames: 10.0,
            hold_frames: 5.0,
            decay_frames: 20.0,
            release_frames: 30.0,
        }
    }

    /// 60 帧推进：覆盖 Delay→Attack→Hold→Decay→Sustain，混合多 lane 阶段，
    /// SoA 结果与标量参照逐位一致。
    #[test]
    fn advance_env_matches_scalar_reference() {
        let mut refs = vec![
            base(0.0),
            base(0.0),
            base(0.5),
            ScalarEnv {
                stage: 5.0,
                envelope: 0.4,
                env_start: 0.4,
                progress: 7.0,
                ..base(0.0)
            },
            ScalarEnv {
                stage: 3.0,
                envelope: 0.9,
                decay_start: 1.0,
                progress: 5.0,
                ..base(0.0)
            },
            ScalarEnv {
                stage: 1.0,
                envelope: 0.2,
                env_start: 0.2,
                progress: 4.0,
                ..base(0.0)
            },
        ];

        let mut soa = VoiceSoa::new();
        for env in &refs {
            push(&mut soa, env);
        }

        let level = Level::new();
        for _ in 0..60 {
            dispatch!(level, simd => soa.advance_env(simd));
            for env in refs.iter_mut() {
                env.advance();
            }
            for (slot, env) in refs.iter().enumerate() {
                let got = read(&soa, slot);
                assert_eq!(got.stage, env.stage, "slot={slot} stage");
                assert_eq!(got.progress, env.progress, "slot={slot} progress");
                assert_eq!(got.envelope, env.envelope, "slot={slot} envelope");
                assert_eq!(got.decay_start, env.decay_start, "slot={slot} decay_start");
            }
        }
        // 推进 60 帧后：前两个 voice 应已过 Sustain，release 的那个应已结束
        assert_eq!(refs[3].stage, ENV_FINISHED);
    }

    /// 哑 lane（len 以下的非活跃 slot）不被推进，且空池安全。
    #[test]
    fn advance_env_ignores_inactive_lanes() {
        let mut soa = VoiceSoa::new();
        let level = Level::new();
        dispatch!(level, simd => soa.advance_env(simd)); // 空池

        let slot = push(&mut soa, &base(0.0));
        let _ = slot;
        // 第二个 slot 未使用：保持哑状态
        assert_eq!(soa.env_stage[1], ENV_FINISHED);
        for _ in 0..40 {
            dispatch!(level, simd => soa.advance_env(simd));
        }
        assert_eq!(soa.env_stage[1], ENV_FINISHED);
        assert_ne!(soa.env_stage[0], ENV_FINISHED);
    }
}
