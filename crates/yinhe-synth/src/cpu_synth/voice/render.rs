//! voice 渲染路径：块级包络切片、常数段快路径、逐帧采样/插值/滤波/输出。
//!
//! 拆自 voice.rs（文件过长）。字段仍是 `CpuVoice` 的（父模块）私有字段，
//! 子模块可直接访问；被本模块调用的父模块方法（advance_env 等）放宽为
//! `pub(in crate::cpu_synth)`。

use super::*;

impl CpuVoice {
    /// 块级渲染：把 `[0, frames)`（区间内帧）按**包络阶段切片**渲染，
    /// 输出**累加**到交错立体声缓冲 `out`（长度 = frames × 2）。
    ///
    /// 核心优化：Delay/Hold/Sustain 是**常数包络段**——整段一次跳过，不再逐帧
    /// 调用 `advance_env`（成本分解实测：tau 峰值段 87% 成本在逐帧包络+控制流，
    /// 采样只占 10%、滤波 5%）。Attack/Decay/Release 仍逐帧推进。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(in crate::cpu_synth) fn render_block(
        &mut self,
        out: &mut [f32],
        frames: usize,
        fi_start: usize,
        sample_start: u64,
        damper: bool,
        profile_mode: u8,
    ) {
        if frames == 0 || self.env_stage >= ENV_FINISHED {
            return;
        }
        // `start_offset` 与 `fi_start` 同为"本次 render_range 调用内的帧坐标"：
        // 段终点 = fi_start + frames；voice 从 begin 起渲染（之前未开始的帧跳过，
        // 不推进包络——与逐帧语义一致）。start_offset 的跨段前移由调用方
        // `advance_block` 统一处理。
        // 创建块用 start_offset 跳过起音前帧；此后 begin 恒 0（渲染后已 started）
        let begin = if self.started {
            0
        } else {
            self.start_offset as usize
        };
        let seg_end = fi_start + frames;
        if begin >= seg_end {
            return;
        }
        self.started = true;
        // 锚点解析出块首进度（替代块末 advance_block 的逐 voice 累加）
        let time0 =
            self.anchor_time + (sample_start - self.anchor_abs) as f64 * f64::from(self.speed);
        // 到期释放的段内帧（未释放且未被踏板保持时有效；<= 段首表示段首已到期）。
        // 释放点在包络切片边界应用——取代逐帧 O(V×frames) 的到期扫描
        //（8139 voice × 512 帧 × 1723 块 = 72 亿次比较，实测主导成本）。
        // 触发后置 `usize::MAX`：局部变量不随 `self.released` 更新，不置位会
        // 每帧重复 `signal_release`（progress 反复清零 → release 曲线退化为
        // 乘法累积，与 GPU/xsynth 的逐帧 release 不一致）。
        let mut release_at: usize = if !self.released && !self.held_by_damper {
            self.end_sample.saturating_sub(sample_start) as usize
        } else {
            usize::MAX
        };
        let mut done = begin.saturating_sub(fi_start);
        if release_at <= done {
            if damper {
                self.held_by_damper = true;
            } else {
                self.signal_release(ENV_RELEASE);
            }
            release_at = usize::MAX;
        }
        while done < frames {
            let (mut sub, constant) = self.env_slice(frames - done);
            if sub == 0 {
                break;
            }
            let mut do_release = false;
            if release_at != usize::MAX && release_at <= done + sub {
                sub = release_at.saturating_sub(done).max(1);
                do_release = true;
            }
            self.render_sub(out, done, sub, fi_start, time0, constant, profile_mode);
            done += sub;
            if do_release {
                if damper {
                    self.held_by_damper = true;
                } else {
                    self.signal_release(ENV_RELEASE);
                }
                release_at = usize::MAX;
            }
        }
    }

    /// 包络推进到下一阶段边界：返回 `(子段帧数, 包络是否常数)`。
    ///
    /// 常数段（Delay/Hold/Sustain）在 `render_sub` 里不逐帧推进；非常数段
    /// （Attack/Decay/Release）逐帧推进（增益逐帧变化）。逐帧语义与
    /// `advance_env` 完全一致（子段边界 = 阶段切换的帧）。
    fn env_slice(&mut self, frames_left: usize) -> (usize, bool) {
        if self.env_stage >= ENV_FINISHED || frames_left == 0 {
            return (0, true);
        }
        match self.env_stage {
            0 => {
                // Delay：包络不变
                let remaining = (self.delay_frames - self.stage_progress).max(0.0);
                let to_boundary = remaining.ceil() as usize;
                let n = frames_left.min(to_boundary.max(1));
                self.stage_progress += n as f32;
                if self.stage_progress + 1.0 >= self.delay_frames {
                    self.env_stage = 1;
                    self.stage_progress = 0.0;
                }
                (n, true)
            }
            1 => {
                // Attack：线性，逐帧
                let remaining = (self.attack_frames - self.stage_progress).max(0.0);
                let to_boundary = remaining.ceil() as usize;
                (frames_left.min(to_boundary.max(1)), false)
            }
            2 => {
                // Hold：包络不变
                let remaining = (self.hold_frames - self.stage_progress).max(0.0);
                let to_boundary = remaining.ceil() as usize;
                let n = frames_left.min(to_boundary.max(1));
                self.stage_progress += n as f32;
                if self.stage_progress + 1.0 >= self.hold_frames {
                    self.env_stage = 3;
                    self.decay_start = self.envelope;
                    self.stage_progress = 0.0;
                }
                (n, true)
            }
            3 => {
                // Decay：指数，逐帧
                let remaining = (self.decay_frames - self.stage_progress).max(0.0);
                let to_boundary = remaining.ceil() as usize;
                (frames_left.min(to_boundary.max(1)), false)
            }
            4 => {
                // Sustain：常数，整段一次跳过（最大收益点）
                let peak = self.env_level;
                self.envelope = self.sustain_level * peak;
                (frames_left, true)
            }
            5 => {
                // Release：指数，逐帧
                let remaining = (self.release_frames - self.stage_progress).max(0.0);
                let to_boundary = remaining.ceil() as usize;
                (frames_left.min(to_boundary.max(1)), false)
            }
            _ => (0, true),
        }
    }

    /// 渲染一个子段：逐帧采样/插值/滤波/输出；非常数包络段逐帧推进包络。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    fn render_sub(
        &mut self,
        out: &mut [f32],
        offset: usize,
        n: usize,
        fi_start: usize,
        time0: f64,
        constant_env: bool,
        profile_mode: u8,
    ) {
        let gain = self.base_gain * self.dup as f32;
        // 循环不变量提出：self 是 &mut 且循环内有 advance_env 调用，
        // LLVM 无法证明这些字段不被改，逐帧字段 load 全部省掉。
        let max_idx = self.sample_length.saturating_sub(1);
        let loop_cont = self.loop_mode == 1;
        let loop_sus_mode = self.loop_mode == 2;
        let loop_avail = self.loop_end > self.loop_start;
        let loop_start = self.loop_start;
        let loop_end = self.loop_end;
        let loop_len = loop_end.saturating_sub(loop_start);
        let scale = 1 + self.is_stereo as u32;
        let sample_offset = self.sample_offset;
        let sample_length = self.sample_length;
        let is_stereo = self.is_stereo;
        let linear = self.interp == 1;
        let cutoff_on = self.cutoff > 0.0;
        let speed = f64::from(self.speed);
        let pan_l = self.pan_l;
        let pan_r = self.pan_r;
        let fi0 = fi_start + offset;
        let loop_active =
            (loop_cont || (loop_sus_mode && self.env_stage < ENV_RELEASE)) && loop_avail;
        // 常数段快路径（Delay/Hold/Sustain，profile 采样关闭）：通用循环里的
        // 包络/循环/profile 分支每帧都在判断，但条件在子段内恒定，编译器
        // 无法从通用循环里消掉——这里独立一份精简循环，把采样 slice、包络
        // 增益、滤波状态全部寄存器化（通用路径因 advance_env 的 &mut self
        // 借用无法这样借）。循环回绕是否生效由循环外的 `loop_active` 布尔
        // 控制（恒定分支，预测稳定）。
        if constant_env && profile_mode == 0 {
            let sample: &[f32] = &self.sample;
            let env = self.envelope;
            let (b0, b1, b2, a1, a2) = (
                self.flt_b0,
                self.flt_b1,
                self.flt_b2,
                self.flt_a1,
                self.flt_a2,
            );
            let (mut s1, mut s2) = (self.flt_s1, self.flt_s2);
            let (mut s1r, mut s2r) = (self.flt_s1r, self.flt_s2r);
            let end_when_over = !loop_cont;
            let mut finished = false;
            for i in 0..n {
                let fi = (fi0 + i) as u32;
                // t 以段起点为基准（time0 对齐到 sample_start；fi 为块内坐标）
                let t = time0 + (fi as usize - fi_start) as f64 * speed;
                let mut idx = t as u64;
                if loop_active && idx > loop_end as u64 {
                    idx = (idx - loop_end as u64 - 1) % loop_len as u64 + loop_start as u64;
                }
                let frac = (t - idx as f64) as f32;
                if idx < sample_length as u64 {
                    let si = (sample_offset as u64 + idx * scale as u64) as usize;
                    let mut l0 = sample.get(si).copied().unwrap_or(0.0);
                    let mut r0 = if is_stereo {
                        sample.get(si + 1).copied().unwrap_or(0.0)
                    } else {
                        l0
                    };
                    if linear && idx < max_idx as u64 {
                        let i1 = si + scale as usize;
                        let l1 = sample.get(i1).copied().unwrap_or(0.0);
                        let r1 = if is_stereo {
                            sample.get(i1 + 1).copied().unwrap_or(0.0)
                        } else {
                            l1
                        };
                        l0 += (l1 - l0) * frac;
                        r0 += (r1 - r0) * frac;
                    }
                    let mut s_l = l0 * gain * env;
                    let mut s_r = r0 * gain * env;
                    if cutoff_on {
                        let out_l = b0 * s_l + s1;
                        s1 = b1 * s_l - a1 * out_l + s2;
                        s2 = b2 * s_l - a2 * out_l;
                        s_l = out_l;
                        if is_stereo {
                            let out_r = b0 * s_r + s1r;
                            s1r = b1 * s_r - a1 * out_r + s2r;
                            s2r = b2 * s_r - a2 * out_r;
                            s_r = out_r;
                        } else {
                            s_r = s_l;
                        }
                    }
                    let oi = (offset + i) * 2;
                    out[oi] += s_l * pan_l;
                    out[oi + 1] += s_r * pan_r;
                } else if end_when_over {
                    finished = true;
                    break;
                }
            }
            if cutoff_on {
                self.flt_s1 = s1;
                self.flt_s2 = s2;
                self.flt_s1r = s1r;
                self.flt_s2r = s2r;
            }
            if finished {
                self.env_stage = ENV_FINISHED;
            }
            return;
        }
        for i in 0..n {
            let fi = (fi0 + i) as u32;

            // 成本分解：3 = 仅包络推进（无采样位置/循环控制流）
            if profile_mode == 3 {
                if !constant_env {
                    self.advance_env();
                }
                continue;
            }

            let t = time0 + (fi as usize - fi_start) as f64 * speed;
            let mut idx = t as u64;
            let frac = (t - idx as f64) as f32;

            // 循环处理（与 xsynth 一致）：1=Continuous 恒循环；2=Sustain 仅未 release 循环
            let released = self.env_stage >= ENV_RELEASE;
            let loop_sus = loop_sus_mode && !released;
            let has_loop = (loop_cont || loop_sus) && loop_avail;
            if has_loop && idx > loop_end as u64 {
                idx = (idx - loop_end as u64 - 1) % loop_len as u64 + loop_start as u64;
            }

            // 成本分解：2 = 保留采样位置/循环控制流，跳过数据读取/插值/滤波/输出
            if profile_mode == 2 {
                if !constant_env {
                    self.advance_env();
                }
                continue;
            }

            if idx < sample_length as u64 {
                let si = (sample_offset as u64 + idx * scale as u64) as usize;
                let mut l0 = self.sample.get(si).copied().unwrap_or(0.0);
                let mut r0 = if is_stereo {
                    self.sample.get(si + 1).copied().unwrap_or(0.0)
                } else {
                    l0
                };
                if linear && idx < max_idx as u64 {
                    let i1 = si + scale as usize;
                    let l1 = self.sample.get(i1).copied().unwrap_or(0.0);
                    let r1 = if is_stereo {
                        self.sample.get(i1 + 1).copied().unwrap_or(0.0)
                    } else {
                        l1
                    };
                    l0 += (l1 - l0) * frac;
                    r0 += (r1 - r0) * frac;
                }
                let mut s_l = l0 * gain * self.envelope;
                let mut s_r = r0 * gain * self.envelope;
                // 成本分解：1 = 无滤波
                if cutoff_on && profile_mode != 1 {
                    // Transposed DirectForm2 biquad：y = b0*x + s1;
                    // s1 = b1*x - a1*y + s2; s2 = b2*x - a2*y（状态 2 个/声道）
                    let out_l = self.flt_b0 * s_l + self.flt_s1;
                    self.flt_s1 = self.flt_b1 * s_l - self.flt_a1 * out_l + self.flt_s2;
                    self.flt_s2 = self.flt_b2 * s_l - self.flt_a2 * out_l;
                    s_l = out_l;
                    if is_stereo {
                        let out_r = self.flt_b0 * s_r + self.flt_s1r;
                        self.flt_s1r = self.flt_b1 * s_r - self.flt_a1 * out_r + self.flt_s2r;
                        self.flt_s2r = self.flt_b2 * s_r - self.flt_a2 * out_r;
                        s_r = out_r;
                    } else {
                        // 单声道样本只用一组滤波器，右声道复用左输出（与 xsynth mono 一致）
                        s_r = s_l;
                    }
                }
                let oi = (offset + i) * 2;
                out[oi] += s_l * pan_l;
                out[oi + 1] += s_r * pan_r;
            } else if !loop_cont {
                // 采样播完（NoLoop/OneShot/LoopSustain release 后）：结束 voice
                self.env_stage = ENV_FINISHED;
            }

            if !constant_env {
                self.advance_env();
            }
        }
    }
}
