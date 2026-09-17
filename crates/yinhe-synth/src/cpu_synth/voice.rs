//! CPU voice：采样播放（loop/插值）+ 7 阶段包络 + per-voice biquad。
//!
//! 逐帧语义与 WGSL shader（`shaders/voice_render.wgsl`）逐行对齐：
//! - 采样位置用**解析式** `t = time + (fi - start_offset) * speed`，
//!   段边界换速时按 WGSL 公式校正 `time` 保持位置连续；
//! - 循环回绕 `idx > loop_end → (idx - end - 1) % len + start`（与 xsynth 一致）；
//! - 包络 7 阶段：Attack 线性、Decay/Release 指数 `(1-t)^8`；
//! - biquad DirectForm1，单声道样本右声道复用左声道输出；
//! - 块末 `time += speed * act_frames` 并回绕（避免长曲 f32 精度漂移）。
//!
//! 与 GPU 的差异（有意）：`time` 用 f64（对齐 xsynth 的 `position: f64`，
//! 无长曲漂移），采样长度按**帧**而非元素计算（修正 GPU 立体声样本的越界）。

use std::sync::Arc;

use crate::channel_state::{ChannelState, env_curve_frames};
use crate::sfz_parser::KeyInfo;

/// 包络结束阶段。
pub(super) const ENV_FINISHED: u32 = 6;
/// release 阶段（shader 的 mode 5）。
pub(super) const ENV_RELEASE: u32 = 5;

/// 单声道/立体声采样帧数（交错立体声按帧折算）。
fn frame_count(info: &KeyInfo) -> u32 {
    (info.sample_data.len() / (1 + info.is_stereo as usize)) as u32
}

pub(super) struct CpuVoice {
    pub(super) channel: u8,
    pub(super) key: u8,
    /// NoteOn 力度（per-key layer 超限时按 xsynth 语义杀最弱 voice）。
    pub(super) velocity: u8,
    /// 音符结束时间（绝对 sample；到期自释，NoteOn 携带）。
    pub(super) end_sample: u64,
    pub(super) held_by_damper: bool,
    /// 已发 release（防止重复触发；damper 保持的 voice 不置位）。
    pub(super) released: bool,

    // 采样
    sample: Arc<[f32]>,
    is_stereo: bool,
    interp: u32,
    sample_length: u32,
    sample_offset: u32,
    speed: f32,
    base_speed: f32,
    base_gain: f32,
    time: f64,
    /// 块内起始帧（NoteOn 所在帧；块末清零）。
    start_offset: u32,

    // 包络（字段与 WGSL VoiceState 对应）
    envelope: f32,
    pub(super) env_stage: u32,
    stage_progress: f32,
    env_level: f32,
    sustain_level: f32,
    env_start: f32,
    decay_start: f32,
    delay_frames: f32,
    attack_frames: f32,
    hold_frames: f32,
    decay_frames: f32,
    release_frames: f32,
    orig_attack_frames: f32,
    orig_release_frames: f32,

    // 声像（音色库基础声像；通道音量/声像在 yinhe-dsp）
    pan_l: f32,
    pan_r: f32,

    // per-voice biquad（cutoff > 0 启用）
    cutoff: f32,
    flt_b0: f32,
    flt_b1: f32,
    flt_b2: f32,
    flt_a1: f32,
    flt_a2: f32,
    flt_x1: f32,
    flt_x2: f32,
    flt_y1: f32,
    flt_y2: f32,
    flt_x1r: f32,
    flt_x2r: f32,
    flt_y1r: f32,
    flt_y2r: f32,

    // 循环
    loop_mode: u32,
    loop_start: u32,
    loop_end: u32,
}

impl CpuVoice {
    /// 由 key map 快照（`note_on` 时零公式计算）构造 voice。
    /// `end_sample`：音符结束的绝对 sample（到期自释）。
    pub(super) fn new(
        info: &KeyInfo,
        channel: u8,
        key: u8,
        velocity: u8,
        end_sample: u64,
        start_offset: u32,
        sample_rate: u32,
        ch: &ChannelState,
    ) -> Self {
        // 音色库声像：等功率法则（与 GPU/xsynth stereo spawner 一致，左右各 1.42 补偿）
        let angle = info.pan * std::f32::consts::FRAC_PI_2;
        let (pan_l, pan_r) = ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0));

        let sr = sample_rate as f32;
        let orig_attack_frames = info.ampeg_attack * sr;
        let orig_release_frames = info.ampeg_release * sr;
        let attack_frames = match ch.env_attack {
            Some(cc) => env_curve_frames(cc, orig_attack_frames, sample_rate, false),
            None => orig_attack_frames,
        };
        let release_frames = match ch.env_release {
            Some(cc) => env_curve_frames(cc, orig_release_frames, sample_rate, true),
            None => orig_release_frames,
        };

        let mut voice = Self {
            channel,
            key,
            velocity,
            end_sample,
            held_by_damper: false,
            released: false,
            sample: Arc::clone(&info.sample_data),
            is_stereo: info.is_stereo,
            interp: info.interp,
            sample_length: 0,
            sample_offset: info.offset,
            speed: info.speed_mult * ch.pitch_multiplier(),
            base_speed: info.speed_mult,
            base_gain: info.volume,
            time: 0.0,
            start_offset,
            envelope: info.ampeg_start,
            env_stage: 0,
            stage_progress: 0.0,
            env_level: 1.0,
            sustain_level: info.ampeg_sustain,
            env_start: info.ampeg_start,
            decay_start: info.ampeg_start,
            delay_frames: info.ampeg_delay * sr,
            attack_frames,
            hold_frames: info.ampeg_hold * sr,
            decay_frames: info.ampeg_decay * sr,
            release_frames,
            orig_attack_frames,
            orig_release_frames,
            pan_l,
            pan_r,
            cutoff: info.cutoff,
            flt_b0: 0.0,
            flt_b1: 0.0,
            flt_b2: 0.0,
            flt_a1: 0.0,
            flt_a2: 0.0,
            flt_x1: 0.0,
            flt_x2: 0.0,
            flt_y1: 0.0,
            flt_y2: 0.0,
            flt_x1r: 0.0,
            flt_x2r: 0.0,
            flt_y1r: 0.0,
            flt_y2r: 0.0,
            loop_mode: info.loop_mode as u32,
            loop_start: info.loop_start,
            loop_end: info.loop_end,
        };
        // 播放长度（帧）：min(采样帧数, stop) - offset。
        let total = frame_count(info);
        voice.sample_length = match info.stop {
            Some(stop) => stop
                .saturating_sub(info.offset)
                .min(total.saturating_sub(info.offset)),
            None => total.saturating_sub(info.offset),
        };
        // per-voice biquad 系数（RBJ cookbook，与 GPU/xsynth 一致）；cutoff=0 时无滤波器
        if info.cutoff > 0.0 {
            let (b0, b1, b2, a1, a2) = crate::synth::biquad_coeffs(
                crate::sfz_parser::filter_type_code(info.filter_type),
                info.cutoff,
                info.resonance,
                sample_rate as f32,
            );
            voice.flt_b0 = b0;
            voice.flt_b1 = b1;
            voice.flt_b2 = b2;
            voice.flt_a1 = a1;
            voice.flt_a2 = a2;
        }
        voice
    }

    pub(super) fn finished(&self) -> bool {
        self.env_stage >= ENV_FINISHED
    }

    /// 段边界换速（弯音/调音/音色切换）：复刻 WGSL 的 time 校正，
    /// 保持"上一帧末 + 新速度"的位置连续。
    pub(super) fn set_speed(&mut self, multiplier: f32, block_frame: u32) {
        let new_speed = self.base_speed * multiplier;
        if new_speed == self.speed {
            return;
        }
        let old_speed = self.speed;
        self.speed = new_speed;
        let n = block_frame.saturating_sub(self.start_offset) as f64;
        self.time += (n - 1.0) * (old_speed - new_speed) as f64;
    }

    /// release/kill：复刻 WGSL release 指令（env_start = 当前 amp，从当前阶段重走）。
    pub(super) fn signal_release(&mut self, stage: u32) {
        self.env_start = self.envelope;
        self.env_stage = stage;
        self.stage_progress = 0.0;
        if stage >= ENV_RELEASE {
            self.released = true;
        }
    }

    /// CC72/73：重算 attack/release 时长并从当前 amp 重走当前阶段（WGSL env_cmds 语义）。
    pub(super) fn apply_env_update(&mut self, ch: &ChannelState, sample_rate: u32) {
        // 与 WGSL/GPU 一致：None 时保持 region 原始值，不重算
        self.attack_frames = match ch.env_attack {
            Some(cc) => env_curve_frames(cc, self.orig_attack_frames, sample_rate, false),
            None => self.orig_attack_frames,
        };
        self.release_frames = match ch.env_release {
            Some(cc) => env_curve_frames(cc, self.orig_release_frames, sample_rate, true),
            None => self.orig_release_frames,
        };
        match self.env_stage {
            0 => self.stage_progress = 0.0,
            1 => {
                self.env_start = self.envelope;
                self.stage_progress = 0.0;
            }
            2 => self.stage_progress = 0.0,
            3 => {
                self.decay_start = self.envelope;
                self.stage_progress = 0.0;
            }
            5 => {
                self.env_start = self.envelope;
                self.stage_progress = 0.0;
            }
            _ => {}
        }
    }

    /// 渲染一帧（`fi` = 块内帧索引），返回 (left, right)。
    /// 复刻 WGSL `vs_main` 的帧循环体（采样 + 插值 + 滤波 + 声像 + 包络推进）。
    pub(super) fn render_frame(&mut self, fi: u32) -> (f32, f32) {
        if self.env_stage >= ENV_FINISHED || fi < self.start_offset {
            return (0.0, 0.0);
        }
        let mut my_l = 0.0f32;
        let mut my_r = 0.0f32;

        let t = self.time + f64::from(fi - self.start_offset) * f64::from(self.speed);
        let mut idx = t as u32;
        let frac = (t - f64::from(idx)) as f32;
        let max_idx = self.sample_length.saturating_sub(1);

        // 循环处理（与 xsynth 一致）：1=Continuous 恒循环；2=Sustain 仅未 release 循环
        let released = self.env_stage >= ENV_RELEASE;
        let loop_cont = self.loop_mode == 1;
        let loop_sus = self.loop_mode == 2 && !released;
        let has_loop = (loop_cont || loop_sus) && self.loop_end > self.loop_start;
        if has_loop && idx > self.loop_end {
            let loop_len = self.loop_end - self.loop_start;
            idx = (idx - self.loop_end - 1) % loop_len + self.loop_start;
        }

        if idx < self.sample_length {
            let scale = 1 + self.is_stereo as u32;
            let i = (self.sample_offset + idx * scale) as usize;
            let mut l0 = self.sample.get(i).copied().unwrap_or(0.0);
            let mut r0 = if self.is_stereo {
                self.sample.get(i + 1).copied().unwrap_or(0.0)
            } else {
                l0
            };
            if self.interp == 1 && idx < max_idx {
                let i1 = i + scale as usize;
                let l1 = self.sample.get(i1).copied().unwrap_or(0.0);
                let r1 = if self.is_stereo {
                    self.sample.get(i1 + 1).copied().unwrap_or(0.0)
                } else {
                    l1
                };
                l0 += (l1 - l0) * frac;
                r0 += (r1 - r0) * frac;
            }
            let mut s_l = l0 * self.base_gain * self.envelope;
            let mut s_r = r0 * self.base_gain * self.envelope;
            if self.cutoff > 0.0 {
                // DirectForm1 biquad：y = b0*x + b1*x1 + b2*x2 - a1*y1 - a2*y2
                let x1 = self.flt_x1;
                let x2 = self.flt_x2;
                let y1 = self.flt_y1;
                let y2 = self.flt_y2;
                let out_l = self.flt_b0 * s_l + self.flt_b1 * x1 + self.flt_b2 * x2
                    - self.flt_a1 * y1
                    - self.flt_a2 * y2;
                self.flt_x1 = s_l;
                self.flt_x2 = x1;
                self.flt_y1 = out_l;
                self.flt_y2 = y1;
                s_l = out_l;
                if self.is_stereo {
                    let x1r = self.flt_x1r;
                    let x2r = self.flt_x2r;
                    let y1r = self.flt_y1r;
                    let y2r = self.flt_y2r;
                    let out_r = self.flt_b0 * s_r + self.flt_b1 * x1r + self.flt_b2 * x2r
                        - self.flt_a1 * y1r
                        - self.flt_a2 * y2r;
                    self.flt_x1r = s_r;
                    self.flt_x2r = x1r;
                    self.flt_y1r = out_r;
                    self.flt_y2r = y1r;
                    s_r = out_r;
                } else {
                    // 单声道样本只用一组滤波器，右声道复用左输出（与 xsynth mono 一致）
                    s_r = s_l;
                }
            }
            my_l = s_l * self.pan_l;
            my_r = s_r * self.pan_r;
        } else if !loop_cont {
            // 采样播完（NoLoop/OneShot/LoopSustain release 后）：结束 voice
            self.env_stage = ENV_FINISHED;
        }

        self.advance_env();
        (my_l, my_r)
    }

    /// 推进 1 帧包络（与 WGSL `advance_env` 逐行等价）。
    fn advance_env(&mut self) {
        if self.env_stage >= ENV_FINISHED {
            return;
        }
        let peak = self.env_level;
        let sus = self.sustain_level * peak;
        match self.env_stage {
            0 => {
                // Delay
                if self.stage_progress + 1.0 >= self.delay_frames {
                    self.env_stage = 1;
                    self.stage_progress = 0.0;
                } else {
                    self.stage_progress += 1.0;
                }
            }
            1 => {
                // Attack：线性
                let n = self.stage_progress + 1.0;
                if n >= self.attack_frames {
                    self.envelope = peak;
                    self.env_stage = 2;
                    self.stage_progress = 0.0;
                } else {
                    self.envelope =
                        self.env_start + (peak - self.env_start) * (n / self.attack_frames);
                    self.stage_progress = n;
                }
            }
            2 => {
                // Hold
                if self.stage_progress + 1.0 >= self.hold_frames {
                    self.env_stage = 3;
                    self.decay_start = self.envelope;
                    self.stage_progress = 0.0;
                } else {
                    self.stage_progress += 1.0;
                }
            }
            3 => {
                // Decay：指数 (1-t)^8
                let n = self.stage_progress + 1.0;
                if n >= self.decay_frames {
                    self.envelope = sus;
                    self.env_stage = 4;
                    self.stage_progress = 0.0;
                } else {
                    let t = n / self.decay_frames;
                    self.envelope = sus + (self.decay_start - sus) * (1.0 - t).powi(8);
                    self.stage_progress = n;
                }
            }
            4 => {
                // Sustain
                self.envelope = sus;
            }
            5 => {
                // Release：指数 (1-t)^8
                let n = self.stage_progress + 1.0;
                if n >= self.release_frames {
                    self.envelope = 0.0;
                    self.env_stage = ENV_FINISHED;
                    self.stage_progress = 0.0;
                } else {
                    let t = n / self.release_frames;
                    self.envelope = self.env_start * (1.0 - t).powi(8);
                    self.stage_progress = n;
                }
            }
            _ => {}
        }
    }

    /// 块末推进：`time += speed × 实际播放帧数` 并回绕（复刻 WGSL 块末语义）；
    /// 未开始的 voice start_offset 前移。
    pub(super) fn advance_block(&mut self, frames: u32) {
        if frames > self.start_offset {
            let act_frames = frames - self.start_offset;
            self.start_offset = 0;
            if self.env_stage < ENV_FINISHED {
                self.time += f64::from(self.speed) * f64::from(act_frames);
                let looped = (self.loop_mode == 1
                    || (self.loop_mode == 2 && self.env_stage < ENV_RELEASE))
                    && self.loop_end > self.loop_start;
                if looped && self.time > f64::from(self.loop_end) {
                    let loop_len = f64::from(self.loop_end - self.loop_start);
                    let off = (self.time - f64::from(self.loop_end) - 1.0) % loop_len;
                    self.time = f64::from(self.loop_end) + 1.0 + off;
                }
            }
        } else {
            self.start_offset -= frames;
        }
    }
}
