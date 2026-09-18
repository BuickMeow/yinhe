//! CPU voice：采样播放（loop/插值）+ 7 阶段包络 + per-voice biquad。
//!
//! 逐帧语义与 WGSL shader（`shaders/voice_render.wgsl`）逐行对齐：
//! - 采样位置用**锚点解析式** `t = anchor_time + (绝对位置 − anchor_abs) × speed`，
//!   段边界换速时按 WGSL 公式校正 `time` 保持位置连续；
//! - 循环回绕 `idx > loop_end → (idx - end - 1) % len + start`（与 xsynth 一致）；
//! - 包络 7 阶段：Attack 线性、Decay/Release 指数 `(1-t)^8`；
//! - biquad **Transposed DirectForm2**（状态 2 个/声道），单声道样本右声道复用左声道输出；
//! - 块末 `time += speed * act_frames` 并回绕（避免长曲 f32 精度漂移）。
//!
//! 与 GPU 的差异（有意）：`time` 用 f64（对齐 xsynth 的 `position: f64`，
//! 无长曲漂移），采样长度按**帧**而非元素计算（修正 GPU 立体声样本的越界）。

use std::sync::Arc;

use crate::channel_state::{ChannelState, env_curve_frames};
use crate::sf_parser::KeyInfo;

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
    /// 已发 kill（淘汰中，1ms 淡出）：不再计入活跃数、不参与淘汰候选
    /// （xsynth `Voice::is_killed()` 语义）。
    pub(super) killed: bool,

    // 采样
    sample: Arc<[f32]>,
    is_stereo: bool,
    interp: u32,
    sample_length: u32,
    sample_offset: u32,
    speed: f32,
    base_speed: f32,
    base_gain: f32,
    /// 完全重复音符的合批引用数：N 个同参数 NoteOn 合成一个 voice，
    /// note_off 逐个递减，归 1 才真正释放。有效增益 = base_gain × dup。
    pub(super) dup: u32,
    /// 已开始渲染（合批判定：同帧未开始才能合并；替代旧的 time != 0 检查）。
    started: bool,
    /// 位置锚点：`anchor_abs`（绝对 sample，锚定时的块首）处采样进度为
    /// `anchor_time`。渲染时 `t = anchor_time + (当前绝对位置 − anchor_abs) ×
    /// speed` 解析计算——块末不再需要 O(V) 的 time 累加/回绕（WGSL 同语义）。
    anchor_abs: u64,
    anchor_time: f64,
    /// 块内起始帧（NoteOn 所在帧；块末清零）。
    start_offset: u32,

    // 包络（字段与 WGSL VoiceState 对应；pub(super) 供交叉测试比对）
    pub(super) envelope: f32,
    pub(super) env_stage: u32,
    pub(super) stage_progress: f32,
    pub(super) env_level: f32,
    pub(super) sustain_level: f32,
    pub(super) env_start: f32,
    pub(super) decay_start: f32,
    pub(super) delay_frames: f32,
    pub(super) attack_frames: f32,
    pub(super) hold_frames: f32,
    pub(super) decay_frames: f32,
    pub(super) release_frames: f32,
    orig_attack_frames: f32,
    orig_release_frames: f32,

    // 声像（音色库基础声像；通道音量/声像在 yinhe-dsp）
    pan_l: f32,
    pan_r: f32,

    // per-voice biquad（Transposed DirectForm2；cutoff > 0 启用，状态 2 个/声道）
    cutoff: f32,
    flt_b0: f32,
    flt_b1: f32,
    flt_b2: f32,
    flt_a1: f32,
    flt_a2: f32,
    flt_s1: f32,
    flt_s2: f32,
    flt_s1r: f32,
    flt_s2r: f32,

    // 循环
    loop_mode: u32,
    loop_start: u32,
    loop_end: u32,
}

mod render;

impl CpuVoice {
    /// 由 key map 快照（`note_on` 时零公式计算）构造 voice。
    /// `end_sample`：音符结束的绝对 sample（到期自释）。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(super) fn new(
        info: &KeyInfo,
        channel: u8,
        key: u8,
        velocity: u8,
        end_sample: u64,
        start_offset: u32,
        sample_rate: u32,
        ch: &ChannelState,
        block_start_abs: u64,
    ) -> Self {
        // 展开共享参数（speed/增益/声像/滤波/包络时长；公式唯一实现在
        // `voice_params::VoiceParams`，CPU/GPU 不再各写一份）。
        // 声像说明：等功率法则（与 xsynth stereo spawner 一致，左右各 1.42
        // 补偿，中心 pan → 1.0）。xsynth 的净输出另被其通道层无条件 pan=0.5
        // 衰减 √2，我们不做该层（声像保持标准正确）；响度差由输出软限幅兜底。
        // 增益在加载期烘焙（`sf_parser::pan_gains`），运行期零三角函数。
        let p = crate::voice_params::VoiceParams::from_key_info(info, ch, sample_rate);

        let mut voice = Self {
            channel,
            key,
            velocity,
            end_sample,
            held_by_damper: false,
            released: false,
            killed: false,
            sample: Arc::clone(&info.sample_data),
            is_stereo: p.is_stereo,
            interp: p.interp,
            sample_length: 0,
            sample_offset: info.offset,
            speed: p.speed,
            base_speed: p.base_speed,
            base_gain: p.base_gain,
            dup: 1,
            started: false,
            // 锚点 = 起音绝对位置（t = 0 处）；`start_offset` 仅用于跳过起音前帧
            anchor_abs: block_start_abs + u64::from(start_offset),
            anchor_time: 0.0,
            start_offset,
            envelope: p.env_start,
            env_stage: 0,
            stage_progress: 0.0,
            env_level: 1.0,
            sustain_level: p.sustain_level,
            env_start: p.env_start,
            decay_start: p.env_start,
            delay_frames: p.delay_frames,
            attack_frames: p.attack_frames,
            hold_frames: p.hold_frames,
            decay_frames: p.decay_frames,
            release_frames: p.release_frames,
            orig_attack_frames: p.orig_attack_frames,
            orig_release_frames: p.orig_release_frames,
            pan_l: p.pan_l,
            pan_r: p.pan_r,
            cutoff: info.cutoff,
            flt_b0: 0.0,
            flt_b1: 0.0,
            flt_b2: 0.0,
            flt_a1: 0.0,
            flt_a2: 0.0,
            flt_s1: 0.0,
            flt_s2: 0.0,
            flt_s1r: 0.0,
            flt_s2r: 0.0,
            loop_mode: p.loop_mode,
            loop_start: p.loop_start,
            loop_end: p.loop_end,
        };
        // 播放长度（帧）：min(采样帧数, stop) - offset。
        let total = frame_count(info);
        voice.sample_length = match info.stop {
            Some(stop) => stop
                .saturating_sub(info.offset)
                .min(total.saturating_sub(info.offset)),
            None => total.saturating_sub(info.offset),
        };
        // per-voice biquad 系数：加载期烘焙（`sf_parser::bake_biquad`，
        // RBJ cookbook 与 GPU/xsynth 一致）；cutoff=0 时无滤波器
        if let Some([b0, b1, b2, a1, a2]) = p.biquad {
            voice.flt_b0 = b0;
            voice.flt_b1 = b1;
            voice.flt_b2 = b2;
            voice.flt_a1 = a1;
            voice.flt_a2 = a2;
        }
        voice
    }

    pub(in crate::cpu_synth) fn finished(&self) -> bool {
        self.env_stage >= ENV_FINISHED
    }

    /// 淘汰中（1ms 淡出），xsynth `Voice::is_killed()` 语义。
    pub(in crate::cpu_synth) fn is_killed(&self) -> bool {
        self.killed
    }

    /// 完全重复合批判定（黑乐谱重复 NoteOn 常态）：同采样/播放倍率/力度/
    /// 结束时刻/起始帧且尚未渲染、未释放、未淡出时命中（线性系统里 N 个
    /// 同相位同参数 voice 之和 = 单个 × N，无损）。
    #[allow(clippy::too_many_arguments)] // 匹配键透传，见 AGENTS 约定
    pub(super) fn matches_batch(
        &self,
        sample: &Arc<[f32]>,
        sample_offset: u32,
        base_speed: f32,
        speed: f32,
        velocity: u8,
        end_sample: u64,
        start_offset: u32,
    ) -> bool {
        if self.released || self.killed || self.finished() || self.started {
            return false;
        }
        self.velocity == velocity
            && self.end_sample == end_sample
            && self.start_offset == start_offset
            && self.sample_offset == sample_offset
            && self.base_speed == base_speed
            && self.speed == speed
            && Arc::ptr_eq(&self.sample, sample)
    }

    /// 合并一个完全重复的 NoteOn：引用数 +1，不再新建 voice。
    pub(super) fn absorb(&mut self) {
        self.dup += 1;
    }

    /// 段边界换速（弯音/调音/音色切换）：复刻 WGSL 的 time 校正，
    /// 保持"上一帧末 + 新速度"的位置连续（`(n−1)(old−new)` 逐字保留）。
    /// 锚点版：把校正结果记为新的 `(anchor_abs = 块首, anchor_time)`。
    pub(super) fn set_speed(&mut self, multiplier: f32, block_frame: u32, sample_start: u64) {
        let new_speed = self.base_speed * multiplier;
        if new_speed == self.speed {
            return;
        }
        let old_speed = self.speed;
        self.speed = new_speed;
        // 块首进度（旧速度外推）→ 应用校正 → 重新锚定到块首
        let time_at_block_start =
            self.anchor_time + (sample_start - self.anchor_abs) as f64 * f64::from(old_speed);
        // n 基准：创建块用 start_offset（此后 start_offset 已是历史值，忽略）
        let base = if self.started { 0 } else { self.start_offset };
        let n = block_frame.saturating_sub(base) as f64;
        self.anchor_time = time_at_block_start + (n - 1.0) * (old_speed - new_speed) as f64;
        self.anchor_abs = sample_start;
    }

    /// kill：淘汰时 1ms 淡出（xsynth `ReleaseType::Kill` 语义——把 release 时长
    /// 改为 1ms 并从当前幅度衰减）。xsynth `fade_out_killing` 默认 false（立即
    /// 移除），但它无全局上限、淘汰罕见；我们淘汰频繁（8192 上限 + 同键 layer），
    /// 硬切会产生 click（用户实测）。
    pub(in crate::cpu_synth) fn signal_kill(&mut self, sample_rate: u32) {
        self.env_start = self.envelope;
        self.env_stage = ENV_RELEASE;
        self.stage_progress = 0.0;
        self.released = true;
        self.killed = true;
        self.release_frames = 0.001 * sample_rate as f32;
    }

    /// release/kill：复刻 WGSL release 指令（env_start = 当前 amp，从当前阶段重走）。
    pub(in crate::cpu_synth) fn signal_release(&mut self, stage: u32) {
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

    /// 推进 1 帧包络（与 WGSL `advance_env` 逐行等价）。
    pub(in crate::cpu_synth) fn advance_env(&mut self) {
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
                // 与 shader 一致：envelope < 1e-4（-80dB，听不见）提前结束释放尾音
                if n >= self.release_frames || self.envelope < 1e-4 {
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

    /// 块末：仅**尚未渲染过**的 voice（事件在块末、本块无渲染区间）需要把
    /// `start_offset` 前移一整个块；已渲染 voice 直接跳过（单布尔分支）。
    /// 位置进度由锚点解析计算，无需逐 voice 累加 time。
    pub(super) fn advance_block(&mut self, frames: u32) {
        if !self.started {
            if frames > self.start_offset {
                self.start_offset = 0;
            } else {
                self.start_offset -= frames;
            }
        }
    }
}
