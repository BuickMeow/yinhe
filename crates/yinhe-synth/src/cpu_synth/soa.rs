//! SoA 声部池：voice 间 SIMD 内核（一个 lane = 一个 voice）。
//!
//! 布局与对齐：
//! - 所有字段数组等长（容量），活跃 voice 连续存放在 `0..len`；
//! - 渲染按 native 宽度（4/8/16）整块处理，范围向上取整到宽度，尾部
//!   lane 为哑 voice（`env_stage = ENV_FINISHED`、增益 0），参与计算但
//!   结果无害，因此热循环内没有 tail 分支；
//! - 状态数组用 `Vec<T>` 而不是 `[T; N]`：native 宽度由运行时 SIMD
//!   等级决定（SSE2/NEON 4、AVX2 8、AVX-512 16），固定块宽会浪费一半
//!   寄存器宽度。
//!
//! 为什么是 voice 间而非帧内（xsynth 做法）：
//! - 每 lane 内部仍逐帧串行，浮点运算顺序与 AoS 实现逐位一致（parity 友好）；
//! - 包络递推、biquad 反馈、采样位置累加这些"逐帧依赖"在 lane 间互相
//!   独立，可以真正并行；xsynth 的帧内 SIMD 在这些部分仍是标量串行。
//!
//! 采样位置/循环/gather 仍是逐 lane 标量（NEON 无 gather；标量 load 后
//! 组装向量比 AVX2 gather 更快），向量化的是包络、插值乘加、biquad 与
//! 输出增益。这是与 xsynth 的关键差别：它的 biquad 是逐 lane 标量。
//!
//! 本模块正在分批移植（已含包络、采样、插值、滤波、输出；待接入渲染
//! 路径与生命周期），接入后移除 `#[allow(dead_code)]`。

use std::sync::Arc;

use fearless_simd::{Select, Simd, SimdBase, SimdMask};

use super::simd::powi8_neg;
use crate::channel_state::ChannelState;
use crate::sfz_parser::{KeyInfo, LoopMode};

/// 包络结束阶段（与 `voice.rs` 的 `ENV_FINISHED` 一致）。
pub const ENV_FINISHED: f32 = 6.0;
/// release 阶段（与 `voice.rs` 的 `ENV_RELEASE` 一致）。
pub const ENV_RELEASE: f32 = 5.0;
/// lane 对齐：AVX-512 的 f32 native 宽度为 16，容量按此对齐。
pub const LANES_ALIGN: usize = 16;
/// 单块最大 lane 数（native 宽度上限）。
const MAX_LANES: usize = 16;

/// SoA 声部池。
pub struct VoiceSoa {
    /// 活跃 voice 数（有效 slot 为 `0..len`）。
    len: usize,

    // ── 包络（每帧状态）──
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
    /// region 原始 attack/release 时长（CC72/73 重算的基准）。
    pub orig_attack_frames: Vec<f32>,
    pub orig_release_frames: Vec<f32>,

    // ── 采样（`samples` 每 lane 持 Arc，保证样本存活；渲染用裸引用）──
    samples: Vec<Arc<[f32]>>,
    /// 1.0 = 交错立体声；0.0 = 单声道。
    pub is_stereo: Vec<f32>,
    /// 插值器：0 = Nearest，1 = Linear。
    pub interp: Vec<f32>,
    /// 播放长度（帧）。
    pub sample_length: Vec<u32>,
    /// 采样起始偏移（帧）。
    pub sample_offset: Vec<u32>,
    /// 当前播放倍率（`base_speed × pitch_multiplier`）。
    pub speed: Vec<f32>,
    /// region 基础倍率（`info.speed_mult`）。
    pub base_speed: Vec<f32>,
    /// 线性增益。
    pub base_gain: Vec<f32>,
    pub pan_l: Vec<f32>,
    pub pan_r: Vec<f32>,

    // ── 位置（f64 逐 lane，长曲无漂移）──
    pub time: Vec<f64>,
    /// 块内起始帧（NoteOn 所在帧；段末由 `advance_block` 清零/前移）。
    pub start_offset: Vec<u32>,

    // ── 循环 ──
    /// 0 = NoLoop，1 = LoopContinuous，2 = LoopSustain，3 = OneShot。
    pub loop_mode: Vec<f32>,
    pub loop_start: Vec<u32>,
    pub loop_end: Vec<u32>,

    // ── per-voice biquad（cutoff=0 时系数为直通：b0=1，其余 0）──
    pub flt_b0: Vec<f32>,
    pub flt_b1: Vec<f32>,
    pub flt_b2: Vec<f32>,
    pub flt_a1: Vec<f32>,
    pub flt_a2: Vec<f32>,
    pub flt_x1: Vec<f32>,
    pub flt_x2: Vec<f32>,
    pub flt_y1: Vec<f32>,
    pub flt_y2: Vec<f32>,
    pub flt_x1r: Vec<f32>,
    pub flt_x2r: Vec<f32>,
    pub flt_y1r: Vec<f32>,
    pub flt_y2r: Vec<f32>,

    // ── 生命周期 ──
    /// 音符结束的绝对 sample（NoteOn 携带；到期自释）。
    pub end_sample: Vec<u64>,
    /// 已发 release（1.0/0.0；防止重复触发）。
    pub released: Vec<f32>,
    /// 被延音踏板保持（1.0/0.0；damper 期间不释放）。
    pub held_by_damper: Vec<f32>,
    /// 淘汰中（1ms 淡出；xsynth `Voice::is_killed()` 语义：不计入活跃数、
    /// 不参与淘汰候选，避免 enforce 反复选中同一 voice 死循环）。
    pub killed: Vec<f32>,
    /// 输出通道（dense 槽位）。
    pub channel: Vec<u8>,
    /// MIDI key（索引重建用）。
    pub key: Vec<u8>,
    /// NoteOn 力度（layer 淘汰用）。
    pub velocity: Vec<u8>,
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
            orig_attack_frames: Vec::new(),
            orig_release_frames: Vec::new(),
            samples: Vec::new(),
            is_stereo: Vec::new(),
            interp: Vec::new(),
            sample_length: Vec::new(),
            sample_offset: Vec::new(),
            speed: Vec::new(),
            base_speed: Vec::new(),
            base_gain: Vec::new(),
            pan_l: Vec::new(),
            pan_r: Vec::new(),
            time: Vec::new(),
            start_offset: Vec::new(),
            loop_mode: Vec::new(),
            loop_start: Vec::new(),
            loop_end: Vec::new(),
            flt_b0: Vec::new(),
            flt_b1: Vec::new(),
            flt_b2: Vec::new(),
            flt_a1: Vec::new(),
            flt_a2: Vec::new(),
            flt_x1: Vec::new(),
            flt_x2: Vec::new(),
            flt_y1: Vec::new(),
            flt_y2: Vec::new(),
            flt_x1r: Vec::new(),
            flt_x2r: Vec::new(),
            flt_y1r: Vec::new(),
            flt_y2r: Vec::new(),
            end_sample: Vec::new(),
            released: Vec::new(),
            held_by_damper: Vec::new(),
            killed: Vec::new(),
            channel: Vec::new(),
            key: Vec::new(),
            velocity: Vec::new(),
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

    /// 由 key map 快照构造一个 lane（与 `CpuVoice::new` 逐字一致）。
    /// 返回 slot 下标。
    #[allow(clippy::too_many_arguments)] // 上下文透传，见 AGENTS 约定
    pub fn init_lane(
        &mut self,
        info: &KeyInfo,
        channel: u8,
        key: u8,
        velocity: u8,
        end_sample: u64,
        start_offset: u32,
        sample_rate: u32,
        ch: &ChannelState,
    ) -> usize {
        let slot = self.push_lane();
        let sr = sample_rate as f32;

        // 声像：等功率法则（与 xsynth stereo spawner 一致）
        let angle = info.pan * std::f32::consts::FRAC_PI_2;
        let (pan_l, pan_r) = ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0));

        let orig_attack_frames = info.ampeg_attack * sr;
        let orig_release_frames = info.ampeg_release * sr;
        let attack_frames = match ch.env_attack {
            Some(cc) => {
                crate::channel_state::env_curve_frames(cc, orig_attack_frames, sample_rate, false)
            }
            None => orig_attack_frames,
        };
        let release_frames = match ch.env_release {
            Some(cc) => {
                crate::channel_state::env_curve_frames(cc, orig_release_frames, sample_rate, true)
            }
            None => orig_release_frames,
        };

        // 播放长度（帧）：min(采样帧数, stop) - offset
        let total = (info.sample_data.len() / (1 + info.is_stereo as usize)) as u32;
        let sample_length = match info.stop {
            Some(stop) => stop
                .saturating_sub(info.offset)
                .min(total.saturating_sub(info.offset)),
            None => total.saturating_sub(info.offset),
        };

        // biquad：cutoff=0 时直通（b0=1），渲染时免分支
        let (b0, b1, b2, a1, a2) = if info.cutoff > 0.0 {
            crate::synth::biquad_coeffs(
                crate::sfz_parser::filter_type_code(info.filter_type),
                info.cutoff,
                info.resonance,
                sr,
            )
        } else {
            (1.0, 0.0, 0.0, 0.0, 0.0)
        };

        self.envelope[slot] = info.ampeg_start;
        self.env_stage[slot] = 0.0;
        self.stage_progress[slot] = 0.0;
        self.env_start[slot] = info.ampeg_start;
        self.decay_start[slot] = info.ampeg_start;
        self.sustain_level[slot] = info.ampeg_sustain;
        self.env_level[slot] = 1.0;
        self.delay_frames[slot] = info.ampeg_delay * sr;
        self.attack_frames[slot] = attack_frames;
        self.hold_frames[slot] = info.ampeg_hold * sr;
        self.decay_frames[slot] = info.ampeg_decay * sr;
        self.release_frames[slot] = release_frames;
        self.orig_attack_frames[slot] = orig_attack_frames;
        self.orig_release_frames[slot] = orig_release_frames;

        self.samples[slot] = Arc::clone(&info.sample_data);
        self.is_stereo[slot] = info.is_stereo as u8 as f32;
        self.interp[slot] = info.interp as f32;
        self.sample_length[slot] = sample_length;
        self.sample_offset[slot] = info.offset;
        self.speed[slot] = info.speed_mult * ch.pitch_multiplier();
        self.base_speed[slot] = info.speed_mult;
        self.base_gain[slot] = info.volume;
        self.pan_l[slot] = pan_l;
        self.pan_r[slot] = pan_r;

        self.time[slot] = 0.0;
        self.start_offset[slot] = start_offset;

        self.loop_mode[slot] = match info.loop_mode {
            LoopMode::NoLoop => 0.0,
            LoopMode::LoopContinuous => 1.0,
            LoopMode::LoopSustain => 2.0,
            LoopMode::OneShot => 3.0,
        };
        self.loop_start[slot] = info.loop_start;
        self.loop_end[slot] = info.loop_end;

        self.flt_b0[slot] = b0;
        self.flt_b1[slot] = b1;
        self.flt_b2[slot] = b2;
        self.flt_a1[slot] = a1;
        self.flt_a2[slot] = a2;
        self.flt_x1[slot] = 0.0;
        self.flt_x2[slot] = 0.0;
        self.flt_y1[slot] = 0.0;
        self.flt_y2[slot] = 0.0;
        self.flt_x1r[slot] = 0.0;
        self.flt_x2r[slot] = 0.0;
        self.flt_y1r[slot] = 0.0;
        self.flt_y2r[slot] = 0.0;

        self.end_sample[slot] = end_sample;
        self.released[slot] = 0.0;
        self.held_by_damper[slot] = 0.0;
        self.killed[slot] = 0.0;
        self.channel[slot] = channel;
        self.key[slot] = key;
        self.velocity[slot] = velocity;

        slot
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
        self.orig_attack_frames[slot] = 0.0;
        self.orig_release_frames[slot] = 0.0;
        self.samples[slot] = Arc::from([]);
        self.is_stereo[slot] = 0.0;
        self.interp[slot] = 0.0;
        self.sample_length[slot] = 0;
        self.sample_offset[slot] = 0;
        self.speed[slot] = 0.0;
        self.base_speed[slot] = 0.0;
        self.base_gain[slot] = 0.0;
        self.pan_l[slot] = 0.0;
        self.pan_r[slot] = 0.0;
        self.time[slot] = 0.0;
        self.start_offset[slot] = 0;
        self.loop_mode[slot] = 0.0;
        self.loop_start[slot] = 0;
        self.loop_end[slot] = 0;
        self.flt_b0[slot] = 1.0;
        self.flt_b1[slot] = 0.0;
        self.flt_b2[slot] = 0.0;
        self.flt_a1[slot] = 0.0;
        self.flt_a2[slot] = 0.0;
        self.flt_x1[slot] = 0.0;
        self.flt_x2[slot] = 0.0;
        self.flt_y1[slot] = 0.0;
        self.flt_y2[slot] = 0.0;
        self.flt_x1r[slot] = 0.0;
        self.flt_x2r[slot] = 0.0;
        self.flt_y1r[slot] = 0.0;
        self.flt_y2r[slot] = 0.0;
        self.end_sample[slot] = u64::MAX;
        self.released[slot] = 0.0;
        self.held_by_damper[slot] = 0.0;
        self.killed[slot] = 0.0;
        self.channel[slot] = 0;
        self.key[slot] = 0;
        self.velocity[slot] = 0;
    }

    /// 扩容（翻倍，最小 `LANES_ALIGN`），新增 slot 为哑 voice。
    fn grow(&mut self) {
        let new_cap = (self.capacity() * 2).max(LANES_ALIGN);
        self.envelope.resize(new_cap, 0.0);
        self.env_stage.resize(new_cap, ENV_FINISHED);
        self.stage_progress.resize(new_cap, 0.0);
        self.env_start.resize(new_cap, 0.0);
        self.decay_start.resize(new_cap, 0.0);
        self.sustain_level.resize(new_cap, 0.0);
        self.env_level.resize(new_cap, 1.0);
        self.delay_frames.resize(new_cap, 0.0);
        self.attack_frames.resize(new_cap, 0.0);
        self.hold_frames.resize(new_cap, 0.0);
        self.decay_frames.resize(new_cap, 0.0);
        self.release_frames.resize(new_cap, 0.0);
        self.orig_attack_frames.resize(new_cap, 0.0);
        self.orig_release_frames.resize(new_cap, 0.0);
        self.samples.resize(new_cap, Arc::from([]));
        self.is_stereo.resize(new_cap, 0.0);
        self.interp.resize(new_cap, 0.0);
        self.sample_length.resize(new_cap, 0);
        self.sample_offset.resize(new_cap, 0);
        self.speed.resize(new_cap, 0.0);
        self.base_speed.resize(new_cap, 0.0);
        self.base_gain.resize(new_cap, 0.0);
        self.pan_l.resize(new_cap, 0.0);
        self.pan_r.resize(new_cap, 0.0);
        self.time.resize(new_cap, 0.0);
        self.start_offset.resize(new_cap, 0);
        self.loop_mode.resize(new_cap, 0.0);
        self.loop_start.resize(new_cap, 0);
        self.loop_end.resize(new_cap, 0);
        self.flt_b0.resize(new_cap, 1.0);
        self.flt_b1.resize(new_cap, 0.0);
        self.flt_b2.resize(new_cap, 0.0);
        self.flt_a1.resize(new_cap, 0.0);
        self.flt_a2.resize(new_cap, 0.0);
        self.flt_x1.resize(new_cap, 0.0);
        self.flt_x2.resize(new_cap, 0.0);
        self.flt_y1.resize(new_cap, 0.0);
        self.flt_y2.resize(new_cap, 0.0);
        self.flt_x1r.resize(new_cap, 0.0);
        self.flt_x2r.resize(new_cap, 0.0);
        self.flt_y1r.resize(new_cap, 0.0);
        self.flt_y2r.resize(new_cap, 0.0);
        self.end_sample.resize(new_cap, u64::MAX);
        self.released.resize(new_cap, 0.0);
        self.held_by_damper.resize(new_cap, 0.0);
        self.killed.resize(new_cap, 0.0);
        self.channel.resize(new_cap, 0);
        self.key.resize(new_cap, 0);
        self.velocity.resize(new_cap, 0);
    }
}

/// 包络状态的向量视图（渲染期间常驻寄存器，避免每帧 load/store）。
struct EnvVecs<S: Simd> {
    stage: S::f32s,
    progress: S::f32s,
    envelope: S::f32s,
    env_start: S::f32s,
    decay_start: S::f32s,
    sustain_level: S::f32s,
    peak: S::f32s,
    delay_frames: S::f32s,
    attack_frames: S::f32s,
    hold_frames: S::f32s,
    decay_frames: S::f32s,
    release_frames: S::f32s,
}

/// 推进一帧包络（`active` lane 才写回；与标量逐位等价）。
#[inline(always)]
fn advance_env_vectors<S: Simd>(simd: S, env: &mut EnvVecs<S>, active: S::mask32s) {
    let one = S::f32s::splat(simd, 1.0);
    let two = S::f32s::splat(simd, 2.0);
    let three = S::f32s::splat(simd, 3.0);
    let four = S::f32s::splat(simd, 4.0);
    let six = S::f32s::splat(simd, 6.0);
    let zero = S::f32s::splat(simd, 0.0);

    let stage = env.stage;
    let prog = env.progress;
    let envelope = env.envelope;
    let decay_start = env.decay_start;

    let prog1 = prog + one;
    let sus = env.sustain_level * env.peak;

    // 各阶段候选值（公式与标量逐字一致）
    let attack_env = env.env_start + (env.peak - env.env_start) * (prog1 / env.attack_frames);
    let decay_env = sus + (decay_start - sus) * powi8_neg(simd, prog1 / env.decay_frames);
    let release_env = env.env_start * powi8_neg(simd, prog1 / env.release_frames);

    // 阶段完成判定 + lane 掩码
    let m0 = stage.simd_eq(0.0);
    let m1 = stage.simd_eq(1.0);
    let m2 = stage.simd_eq(2.0);
    let m3 = stage.simd_eq(3.0);
    let m4 = stage.simd_eq(4.0);
    let m5 = stage.simd_eq(5.0);
    let finished = stage.simd_ge(ENV_FINISHED);

    let done0 = m0 & prog1.simd_ge(env.delay_frames);
    let done1 = m1 & prog1.simd_ge(env.attack_frames);
    let done2 = m2 & prog1.simd_ge(env.hold_frames);
    let done3 = m3 & prog1.simd_ge(env.decay_frames);
    let done5 = m5 & prog1.simd_ge(env.release_frames);

    // 默认：stage 不变、progress 推进一帧、幅度不变、decay_start 不变
    let mut new_stage = stage;
    let mut new_prog = prog1;
    let mut new_env = envelope;
    let mut new_decay_start = decay_start;

    // 0 Delay：到期进 Attack（幅度不变）
    new_stage = done0.select(one, new_stage);
    new_prog = done0.select(zero, new_prog);

    // 1 Attack：线性；到期置 peak 进 Hold
    new_env = done1.select(env.peak, new_env);
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

    // 未开始的 lane 整体不推进
    env.stage = active.select(new_stage, stage);
    env.progress = active.select(new_prog, prog);
    env.envelope = active.select(new_env, envelope);
    env.decay_start = active.select(new_decay_start, decay_start);
}

/// 生命周期与状态推进（供 `CpuSynth` 声部管理调用；渲染外的低频路径）。
impl VoiceSoa {
    /// 活跃 voice 数（未结束）。
    pub fn voice_count(&self) -> usize {
        self.env_stage[..self.len]
            .iter()
            .filter(|&&s| s < ENV_FINISHED)
            .count()
    }

    /// 删除已结束的 voice（**保持创建顺序**压缩），返回删除数。
    /// `key_indices` 的"最老 voice"语义依赖顺序，块末统一重建索引。
    pub fn compact(&mut self) -> usize {
        let mut write = 0usize;
        for read in 0..self.len {
            if self.env_stage[read] < ENV_FINISHED {
                if write != read {
                    self.copy_lane(read, write);
                }
                write += 1;
            }
        }
        let removed = self.len - write;
        for slot in write..self.len {
            self.reset_lane(slot);
        }
        self.len = write;
        removed
    }

    /// 把 `from` lane 的全部状态复制到 `to`（紧凑压缩用）。
    fn copy_lane(&mut self, from: usize, to: usize) {
        self.envelope[to] = self.envelope[from];
        self.env_stage[to] = self.env_stage[from];
        self.stage_progress[to] = self.stage_progress[from];
        self.env_start[to] = self.env_start[from];
        self.decay_start[to] = self.decay_start[from];
        self.sustain_level[to] = self.sustain_level[from];
        self.env_level[to] = self.env_level[from];
        self.delay_frames[to] = self.delay_frames[from];
        self.attack_frames[to] = self.attack_frames[from];
        self.hold_frames[to] = self.hold_frames[from];
        self.decay_frames[to] = self.decay_frames[from];
        self.release_frames[to] = self.release_frames[from];
        self.orig_attack_frames[to] = self.orig_attack_frames[from];
        self.orig_release_frames[to] = self.orig_release_frames[from];
        self.samples[to] = Arc::clone(&self.samples[from]);
        self.is_stereo[to] = self.is_stereo[from];
        self.interp[to] = self.interp[from];
        self.sample_length[to] = self.sample_length[from];
        self.sample_offset[to] = self.sample_offset[from];
        self.speed[to] = self.speed[from];
        self.base_speed[to] = self.base_speed[from];
        self.base_gain[to] = self.base_gain[from];
        self.pan_l[to] = self.pan_l[from];
        self.pan_r[to] = self.pan_r[from];
        self.time[to] = self.time[from];
        self.start_offset[to] = self.start_offset[from];
        self.loop_mode[to] = self.loop_mode[from];
        self.loop_start[to] = self.loop_start[from];
        self.loop_end[to] = self.loop_end[from];
        self.flt_b0[to] = self.flt_b0[from];
        self.flt_b1[to] = self.flt_b1[from];
        self.flt_b2[to] = self.flt_b2[from];
        self.flt_a1[to] = self.flt_a1[from];
        self.flt_a2[to] = self.flt_a2[from];
        self.flt_x1[to] = self.flt_x1[from];
        self.flt_x2[to] = self.flt_x2[from];
        self.flt_y1[to] = self.flt_y1[from];
        self.flt_y2[to] = self.flt_y2[from];
        self.flt_x1r[to] = self.flt_x1r[from];
        self.flt_x2r[to] = self.flt_x2r[from];
        self.flt_y1r[to] = self.flt_y1r[from];
        self.flt_y2r[to] = self.flt_y2r[from];
        self.end_sample[to] = self.end_sample[from];
        self.released[to] = self.released[from];
        self.held_by_damper[to] = self.held_by_damper[from];
        self.killed[to] = self.killed[from];
        self.channel[to] = self.channel[from];
        self.key[to] = self.key[from];
        self.velocity[to] = self.velocity[from];
    }

    /// release/kill：从当前幅度进入 release（与 `CpuVoice::signal_release` 一致）。
    pub fn signal_release(&mut self, slot: usize, stage: f32) {
        self.env_start[slot] = self.envelope[slot];
        self.env_stage[slot] = stage;
        self.stage_progress[slot] = 0.0;
        if stage >= ENV_RELEASE {
            self.released[slot] = 1.0;
        }
    }

    /// kill：淘汰时 1ms 淡出（与 `CpuVoice::signal_kill` 一致）。
    pub fn signal_kill(&mut self, slot: usize, sample_rate: u32) {
        self.env_start[slot] = self.envelope[slot];
        self.env_stage[slot] = ENV_RELEASE;
        self.stage_progress[slot] = 0.0;
        self.released[slot] = 1.0;
        self.killed[slot] = 1.0;
        self.release_frames[slot] = 0.001 * sample_rate as f32;
    }

    /// 段边界换速（弯音/调音；含 time 校正，与 `CpuVoice::set_speed` 一致）。
    pub fn set_speed(&mut self, slot: usize, multiplier: f32, block_frame: u32) {
        let new_speed = self.base_speed[slot] * multiplier;
        if new_speed == self.speed[slot] {
            return;
        }
        let old_speed = self.speed[slot];
        self.speed[slot] = new_speed;
        let n = block_frame.saturating_sub(self.start_offset[slot]) as f64;
        self.time[slot] += (n - 1.0) * (old_speed - new_speed) as f64;
    }

    /// CC72/73：重算 attack/release 时长并从当前幅度重走当前阶段
    /// （与 `CpuVoice::apply_env_update` 一致）。
    pub fn apply_env_update(&mut self, slot: usize, ch: &ChannelState, sample_rate: u32) {
        self.attack_frames[slot] = match ch.env_attack {
            Some(cc) => crate::channel_state::env_curve_frames(
                cc,
                self.orig_attack_frames[slot],
                sample_rate,
                false,
            ),
            None => self.orig_attack_frames[slot],
        };
        self.release_frames[slot] = match ch.env_release {
            Some(cc) => crate::channel_state::env_curve_frames(
                cc,
                self.orig_release_frames[slot],
                sample_rate,
                true,
            ),
            None => self.orig_release_frames[slot],
        };
        match self.env_stage[slot] as u32 {
            0 => self.stage_progress[slot] = 0.0,
            1 => {
                self.env_start[slot] = self.envelope[slot];
                self.stage_progress[slot] = 0.0;
            }
            2 => self.stage_progress[slot] = 0.0,
            3 => {
                self.decay_start[slot] = self.envelope[slot];
                self.stage_progress[slot] = 0.0;
            }
            5 => {
                self.env_start[slot] = self.envelope[slot];
                self.stage_progress[slot] = 0.0;
            }
            _ => {}
        }
    }

    /// 块末推进：`time += speed × 实际播放帧数` 并回绕
    /// （与 `CpuVoice::advance_block` 一致）。
    pub fn advance_block(&mut self, slot: usize, frames: u32) {
        if frames > self.start_offset[slot] {
            let act_frames = frames - self.start_offset[slot];
            self.start_offset[slot] = 0;
            if self.env_stage[slot] < ENV_FINISHED {
                self.time[slot] += f64::from(self.speed[slot]) * f64::from(act_frames);
                let looped = (self.loop_mode[slot] == 1.0
                    || (self.loop_mode[slot] == 2.0 && self.env_stage[slot] < ENV_RELEASE))
                    && self.loop_end[slot] > self.loop_start[slot];
                if looped && self.time[slot] > f64::from(self.loop_end[slot]) {
                    let loop_len = f64::from(self.loop_end[slot] - self.loop_start[slot]);
                    let off = (self.time[slot] - f64::from(self.loop_end[slot]) - 1.0) % loop_len;
                    self.time[slot] = f64::from(self.loop_end[slot]) + 1.0 + off;
                }
            }
        } else {
            self.start_offset[slot] -= frames;
        }
    }
}

/// 按 lane 范围切分的渲染视图（见 [`VoiceSoa::par_views`]）。
///
/// 生命周期绑定在 `&mut VoiceSoa` 上：视图存活期间池不可被访问（借用检查
/// 保证），视图之间 lane 区间不相交（`chunks_mut` 保证），因此无需 unsafe
/// 即可交给 Rayon 并行。
pub struct VoiceSoaView<'a> {
    pub envelope: &'a mut [f32],
    pub env_stage: &'a mut [f32],
    pub stage_progress: &'a mut [f32],
    pub env_start: &'a mut [f32],
    pub decay_start: &'a mut [f32],
    pub flt_x1: &'a mut [f32],
    pub flt_x2: &'a mut [f32],
    pub flt_y1: &'a mut [f32],
    pub flt_y2: &'a mut [f32],
    pub flt_x1r: &'a mut [f32],
    pub flt_x2r: &'a mut [f32],
    pub flt_y1r: &'a mut [f32],
    pub flt_y2r: &'a mut [f32],
    pub released: &'a mut [f32],
    pub held_by_damper: &'a mut [f32],
    pub time: &'a mut [f64],
    pub start_offset: &'a mut [u32],
    pub sustain_level: &'a [f32],
    pub env_level: &'a [f32],
    pub delay_frames: &'a [f32],
    pub attack_frames: &'a [f32],
    pub hold_frames: &'a [f32],
    pub decay_frames: &'a [f32],
    pub release_frames: &'a [f32],
    pub is_stereo: &'a [f32],
    pub interp: &'a [f32],
    pub speed: &'a [f32],
    pub base_gain: &'a [f32],
    pub pan_l: &'a [f32],
    pub pan_r: &'a [f32],
    pub loop_mode: &'a [f32],
    pub flt_b0: &'a [f32],
    pub flt_b1: &'a [f32],
    pub flt_b2: &'a [f32],
    pub flt_a1: &'a [f32],
    pub flt_a2: &'a [f32],
    pub sample_length: &'a [u32],
    pub sample_offset: &'a [u32],
    pub loop_start: &'a [u32],
    pub loop_end: &'a [u32],
    pub end_sample: &'a [u64],
    pub channel: &'a [u8],
    pub samples: &'a [Arc<[f32]>],
}

impl VoiceSoa {
    /// 把池按 lane 范围切成可变异步视图（Rayon 分片单元）。
    ///
    /// `chunk` 向上对齐到 [`LANES_ALIGN`]（保证每个视图都是 native 宽度的
    /// 整数倍，渲染无需尾部处理）。分片数 = `capacity / chunk`。
    pub fn par_views(&mut self, chunk: usize) -> Vec<VoiceSoaView<'_>> {
        let chunk = chunk.next_multiple_of(LANES_ALIGN).max(LANES_ALIGN);
        let n = self.capacity().div_ceil(chunk);
        if n == 0 {
            return Vec::new();
        }
        let mut envelope: Vec<&mut [f32]> = self.envelope.chunks_mut(chunk).collect();
        let mut env_stage: Vec<&mut [f32]> = self.env_stage.chunks_mut(chunk).collect();
        let mut stage_progress: Vec<&mut [f32]> = self.stage_progress.chunks_mut(chunk).collect();
        let mut env_start: Vec<&mut [f32]> = self.env_start.chunks_mut(chunk).collect();
        let mut decay_start: Vec<&mut [f32]> = self.decay_start.chunks_mut(chunk).collect();
        let mut flt_x1: Vec<&mut [f32]> = self.flt_x1.chunks_mut(chunk).collect();
        let mut flt_x2: Vec<&mut [f32]> = self.flt_x2.chunks_mut(chunk).collect();
        let mut flt_y1: Vec<&mut [f32]> = self.flt_y1.chunks_mut(chunk).collect();
        let mut flt_y2: Vec<&mut [f32]> = self.flt_y2.chunks_mut(chunk).collect();
        let mut flt_x1r: Vec<&mut [f32]> = self.flt_x1r.chunks_mut(chunk).collect();
        let mut flt_x2r: Vec<&mut [f32]> = self.flt_x2r.chunks_mut(chunk).collect();
        let mut flt_y1r: Vec<&mut [f32]> = self.flt_y1r.chunks_mut(chunk).collect();
        let mut flt_y2r: Vec<&mut [f32]> = self.flt_y2r.chunks_mut(chunk).collect();
        let mut released: Vec<&mut [f32]> = self.released.chunks_mut(chunk).collect();
        let mut held_by_damper: Vec<&mut [f32]> = self.held_by_damper.chunks_mut(chunk).collect();
        let mut time: Vec<&mut [f64]> = self.time.chunks_mut(chunk).collect();
        let mut start_offset: Vec<&mut [u32]> = self.start_offset.chunks_mut(chunk).collect();
        let sustain_level: Vec<&[f32]> = self.sustain_level.chunks(chunk).collect();
        let env_level: Vec<&[f32]> = self.env_level.chunks(chunk).collect();
        let delay_frames: Vec<&[f32]> = self.delay_frames.chunks(chunk).collect();
        let attack_frames: Vec<&[f32]> = self.attack_frames.chunks(chunk).collect();
        let hold_frames: Vec<&[f32]> = self.hold_frames.chunks(chunk).collect();
        let decay_frames: Vec<&[f32]> = self.decay_frames.chunks(chunk).collect();
        let release_frames: Vec<&[f32]> = self.release_frames.chunks(chunk).collect();
        let is_stereo: Vec<&[f32]> = self.is_stereo.chunks(chunk).collect();
        let interp: Vec<&[f32]> = self.interp.chunks(chunk).collect();
        let speed: Vec<&[f32]> = self.speed.chunks(chunk).collect();
        let base_gain: Vec<&[f32]> = self.base_gain.chunks(chunk).collect();
        let pan_l: Vec<&[f32]> = self.pan_l.chunks(chunk).collect();
        let pan_r: Vec<&[f32]> = self.pan_r.chunks(chunk).collect();
        let loop_mode: Vec<&[f32]> = self.loop_mode.chunks(chunk).collect();
        let flt_b0: Vec<&[f32]> = self.flt_b0.chunks(chunk).collect();
        let flt_b1: Vec<&[f32]> = self.flt_b1.chunks(chunk).collect();
        let flt_b2: Vec<&[f32]> = self.flt_b2.chunks(chunk).collect();
        let flt_a1: Vec<&[f32]> = self.flt_a1.chunks(chunk).collect();
        let flt_a2: Vec<&[f32]> = self.flt_a2.chunks(chunk).collect();
        let sample_length: Vec<&[u32]> = self.sample_length.chunks(chunk).collect();
        let sample_offset: Vec<&[u32]> = self.sample_offset.chunks(chunk).collect();
        let loop_start: Vec<&[u32]> = self.loop_start.chunks(chunk).collect();
        let loop_end: Vec<&[u32]> = self.loop_end.chunks(chunk).collect();
        let end_sample: Vec<&[u64]> = self.end_sample.chunks(chunk).collect();
        let channel: Vec<&[u8]> = self.channel.chunks(chunk).collect();
        let samples: Vec<&[Arc<[f32]>]> = self.samples.chunks(chunk).collect();
        (0..n)
            .map(|i| VoiceSoaView {
                envelope: std::mem::take(&mut envelope[i]),
                env_stage: std::mem::take(&mut env_stage[i]),
                stage_progress: std::mem::take(&mut stage_progress[i]),
                env_start: std::mem::take(&mut env_start[i]),
                decay_start: std::mem::take(&mut decay_start[i]),
                flt_x1: std::mem::take(&mut flt_x1[i]),
                flt_x2: std::mem::take(&mut flt_x2[i]),
                flt_y1: std::mem::take(&mut flt_y1[i]),
                flt_y2: std::mem::take(&mut flt_y2[i]),
                flt_x1r: std::mem::take(&mut flt_x1r[i]),
                flt_x2r: std::mem::take(&mut flt_x2r[i]),
                flt_y1r: std::mem::take(&mut flt_y1r[i]),
                flt_y2r: std::mem::take(&mut flt_y2r[i]),
                released: std::mem::take(&mut released[i]),
                held_by_damper: std::mem::take(&mut held_by_damper[i]),
                time: std::mem::take(&mut time[i]),
                start_offset: std::mem::take(&mut start_offset[i]),
                sustain_level: sustain_level[i],
                env_level: env_level[i],
                delay_frames: delay_frames[i],
                attack_frames: attack_frames[i],
                hold_frames: hold_frames[i],
                decay_frames: decay_frames[i],
                release_frames: release_frames[i],
                is_stereo: is_stereo[i],
                interp: interp[i],
                speed: speed[i],
                base_gain: base_gain[i],
                pan_l: pan_l[i],
                pan_r: pan_r[i],
                loop_mode: loop_mode[i],
                flt_b0: flt_b0[i],
                flt_b1: flt_b1[i],
                flt_b2: flt_b2[i],
                flt_a1: flt_a1[i],
                flt_a2: flt_a2[i],
                sample_length: sample_length[i],
                sample_offset: sample_offset[i],
                loop_start: loop_start[i],
                loop_end: loop_end[i],
                end_sample: end_sample[i],
                channel: channel[i],
                samples: samples[i],
            })
            .collect()
    }
}

impl VoiceSoaView<'_> {
    /// 本视图的 lane 数。
    pub fn len(&self) -> usize {
        self.envelope.len()
    }

    /// 渲染本视图的 lane（块内所有帧），输出**累加**到分片 scratch
    /// （每通道 `frames × 2` 交错立体声区域，通道 stride = `frames * 2`）。
    ///
    /// 语义与 `CpuVoice::render_block`（`profile_mode = 0`）逐位一致。
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub fn render<S: Simd>(
        &mut self,
        simd: S,
        out: &mut [f32],
        fi_start: usize,
        frames: usize,
        sample_start: u64,
        damper: &[bool],
    ) {
        let width = S::f32s::LEN;
        let n = self.len();
        let mut start = 0;
        while start < n {
            let w = width.min(n - start);
            self.render_lanes(simd, start, w, out, fi_start, frames, sample_start, damper);
            start += w;
        }
    }

    /// 渲染本视图内 `[start, start + width)` 的 lane（块内所有帧）。
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn render_lanes<S: Simd>(
        &mut self,
        simd: S,
        start: usize,
        width: usize,
        out: &mut [f32],
        fi_start: usize,
        frames: usize,
        sample_start: u64,
        damper: &[bool],
    ) {
        let slot_of = |lane: usize| start + lane;

        // ── 常数参数（帧不变，load 一次）──
        let gain = S::f32s::from_slice(simd, &self.base_gain[start..start + width]);
        let pan_l = S::f32s::from_slice(simd, &self.pan_l[start..start + width]);
        let pan_r = S::f32s::from_slice(simd, &self.pan_r[start..start + width]);
        let is_stereo = S::f32s::from_slice(simd, &self.is_stereo[start..start + width]);
        let stereo_mask = is_stereo.simd_gt(0.5);
        let b0 = S::f32s::from_slice(simd, &self.flt_b0[start..start + width]);
        let b1 = S::f32s::from_slice(simd, &self.flt_b1[start..start + width]);
        let b2 = S::f32s::from_slice(simd, &self.flt_b2[start..start + width]);
        let a1 = S::f32s::from_slice(simd, &self.flt_a1[start..start + width]);
        let a2 = S::f32s::from_slice(simd, &self.flt_a2[start..start + width]);

        // ── 每帧状态 ──
        let mut env = EnvVecs::load_view(simd, self, start, width);
        let mut x1 = S::f32s::from_slice(simd, &self.flt_x1[start..start + width]);
        let mut x2 = S::f32s::from_slice(simd, &self.flt_x2[start..start + width]);
        let mut y1 = S::f32s::from_slice(simd, &self.flt_y1[start..start + width]);
        let mut y2 = S::f32s::from_slice(simd, &self.flt_y2[start..start + width]);
        let mut x1r = S::f32s::from_slice(simd, &self.flt_x1r[start..start + width]);
        let mut x2r = S::f32s::from_slice(simd, &self.flt_x2r[start..start + width]);
        let mut y1r = S::f32s::from_slice(simd, &self.flt_y1r[start..start + width]);
        let mut y2r = S::f32s::from_slice(simd, &self.flt_y2r[start..start + width]);
        let mut released = S::f32s::from_slice(simd, &self.released[start..start + width]);
        let mut held = S::f32s::from_slice(simd, &self.held_by_damper[start..start + width]);

        // ── 常数的 lane 参数（帧不变，标量栈数组）──
        let mut begins = [0u32; MAX_LANES];
        let mut release_ats = [0u32; MAX_LANES];
        let mut damper_bits = 0u64;
        for lane in 0..width {
            let slot = slot_of(lane);
            begins[lane] = self.start_offset[slot];
            release_ats[lane] = if self.released[slot] != 0.0 || self.held_by_damper[slot] != 0.0 {
                u32::MAX
            } else {
                self.end_sample[slot]
                    .saturating_sub(sample_start)
                    .min(u32::MAX as u64) as u32
            };
            if damper
                .get(self.channel[slot] as usize)
                .copied()
                .unwrap_or(false)
            {
                damper_bits |= 1 << lane;
            }
        }
        let begin_vec = S::u32s::from_slice(simd, &begins[..width]);
        let release_at_vec = S::u32s::from_slice(simd, &release_ats[..width]);
        let damper_mask = S::mask32s::from_bitmask(simd, damper_bits);
        let mut pan_l_arr = [0f32; MAX_LANES];
        let mut pan_r_arr = [0f32; MAX_LANES];
        pan_l.store_slice(&mut pan_l_arr[..width]);
        pan_r.store_slice(&mut pan_r_arr[..width]);

        let zero = S::f32s::splat(simd, 0.0);
        let one = S::f32s::splat(simd, 1.0);
        let five = S::f32s::splat(simd, ENV_RELEASE);
        let four = S::f32s::splat(simd, 4.0);
        let six = S::f32s::splat(simd, ENV_FINISHED);

        // ── 帧循环 ──
        for i in 0..frames {
            let fi_abs = (fi_start + i) as u32;
            let i_vec = S::u32s::splat(simd, i as u32);
            let fi_vec = S::u32s::splat(simd, fi_abs);

            // 释放触发（踏板按住时改为 held）
            let rel_now = released.simd_eq(0.0) & held.simd_eq(0.0) & i_vec.simd_ge(release_at_vec);
            let rel_mask = rel_now & !damper_mask;
            let hold_mask = rel_now & damper_mask;
            env.env_start = rel_mask.select(env.envelope, env.env_start);
            env.stage = rel_mask.select(five, env.stage);
            env.progress = rel_mask.select(zero, env.progress);
            released = rel_mask.select(one, released);
            held = hold_mask.select(one, held);

            // 本帧活跃 lane：已开始且未结束
            let begun = fi_vec.simd_ge(begin_vec);
            let live = env.stage.simd_lt(six) & begun;
            let live_bits = live.to_bitmask();

            // ── 采样（逐 lane 标量：位置 f64、循环回绕、gather）──
            let mut l0s = [0f32; MAX_LANES];
            let mut r0s = [0f32; MAX_LANES];
            let mut finished_bits = 0u64;
            if live_bits != 0 {
                let rel_stage_bits = env.stage.simd_ge(five).to_bitmask();
                for lane in 0..width {
                    if (live_bits >> lane) & 1 == 0 {
                        continue;
                    }
                    let slot = slot_of(lane);
                    let n = (fi_start + i - begins[lane] as usize) as f64;
                    let t = self.time[slot] + n * f64::from(self.speed[slot]);
                    let mut idx = t as u32;
                    let frac = (t - f64::from(idx)) as f32;
                    let sample_len = self.sample_length[slot];
                    let max_idx = sample_len.saturating_sub(1);
                    let is_released = ((rel_stage_bits >> lane) & 1) != 0;
                    let loop_cont = self.loop_mode[slot] == 1.0;
                    let loop_sus = self.loop_mode[slot] == 2.0 && !is_released;
                    let has_loop =
                        (loop_cont || loop_sus) && self.loop_end[slot] > self.loop_start[slot];
                    if has_loop && idx > self.loop_end[slot] {
                        let loop_len = self.loop_end[slot] - self.loop_start[slot];
                        idx = (idx - self.loop_end[slot] - 1) % loop_len + self.loop_start[slot];
                    }
                    if idx < sample_len {
                        let scale = 1 + self.is_stereo[slot] as u32;
                        let si = (self.sample_offset[slot] + idx * scale) as usize;
                        let sample = &self.samples[slot];
                        let mut l0 = sample.get(si).copied().unwrap_or(0.0);
                        let mut r0 = if self.is_stereo[slot] != 0.0 {
                            sample.get(si + 1).copied().unwrap_or(0.0)
                        } else {
                            l0
                        };
                        if self.interp[slot] == 1.0 && idx < max_idx {
                            let i1 = si + scale as usize;
                            let l1 = sample.get(i1).copied().unwrap_or(0.0);
                            let r1 = if self.is_stereo[slot] != 0.0 {
                                sample.get(i1 + 1).copied().unwrap_or(0.0)
                            } else {
                                l1
                            };
                            l0 += (l1 - l0) * frac;
                            r0 += (r1 - r0) * frac;
                        }
                        l0s[lane] = l0;
                        r0s[lane] = r0;
                    } else if !loop_cont {
                        finished_bits |= 1 << lane;
                    }
                }
            }

            // ── 增益 / 插值结果 / biquad（向量）──
            let raw_l = S::f32s::from_slice(simd, &l0s[..width]);
            let raw_r = S::f32s::from_slice(simd, &r0s[..width]);
            let mut s_l = raw_l * gain * env.envelope;
            let mut s_r = raw_r * gain * env.envelope;

            let out_l = b0 * s_l + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = s_l;
            y2 = y1;
            y1 = out_l;
            s_l = out_l;

            let sr_in = s_r;
            let out_r = b0 * s_r + b1 * x1r + b2 * x2r - a1 * y1r - a2 * y2r;
            x2r = x1r;
            x1r = sr_in;
            y2r = y1r;
            y1r = out_r;
            s_r = stereo_mask.select(out_r, s_l);

            // ── 输出（逐 lane 累加到通道区域）──
            if live_bits != 0 {
                let mut sl_arr = [0f32; MAX_LANES];
                let mut sr_arr = [0f32; MAX_LANES];
                s_l.store_slice(&mut sl_arr[..width]);
                s_r.store_slice(&mut sr_arr[..width]);
                let base_frame = i * 2;
                for lane in 0..width {
                    if (live_bits >> lane) & 1 == 0 {
                        continue;
                    }
                    let ch = self.channel[slot_of(lane)] as usize;
                    let base = ch * frames * 2 + base_frame;
                    out[base] += sl_arr[lane] * pan_l_arr[lane];
                    out[base + 1] += sr_arr[lane] * pan_r_arr[lane];
                }
            }

            // 越界结束（本帧后不再输出）
            if finished_bits != 0 {
                env.stage = S::mask32s::from_bitmask(simd, finished_bits).select(six, env.stage);
            }

            // 包络推进：块内全为 Sustain/Finished（无包络工作）时整体跳过
            // 7 阶段状态机——Sustain 的 envelope 在 Decay 完成时已置为
            // `sustain_level × peak`，期间无状态推进（AoS 的常数段跳过在
            // SoA 的等价物；长音主体收益最大，实测 352 voice 同起音场景
            // 命中率 100%）。
            let no_env_work = env.stage.simd_eq(four) | env.stage.simd_ge(six);
            if !no_env_work.all_true() {
                advance_env_vectors(simd, &mut env, begun);
            }
        }

        // 段末释放：标量在子段末检查 `release_at <= done + sub`，当释放点
        // 恰为段末（frames）时在本段结束时触发（否则要等下一段段首）。
        let end_rel = released.simd_eq(0.0)
            & held.simd_eq(0.0)
            & S::u32s::splat(simd, frames as u32).simd_ge(release_at_vec);
        let end_rel_mask = end_rel & !damper_mask;
        let end_hold_mask = end_rel & damper_mask;
        env.env_start = end_rel_mask.select(env.envelope, env.env_start);
        env.stage = end_rel_mask.select(five, env.stage);
        env.progress = end_rel_mask.select(zero, env.progress);
        released = end_rel_mask.select(one, released);
        held = end_hold_mask.select(one, held);

        // ── 写回状态 ──
        env.store_view(self, start, width);
        x1.store_slice(&mut self.flt_x1[start..start + width]);
        x2.store_slice(&mut self.flt_x2[start..start + width]);
        y1.store_slice(&mut self.flt_y1[start..start + width]);
        y2.store_slice(&mut self.flt_y2[start..start + width]);
        x1r.store_slice(&mut self.flt_x1r[start..start + width]);
        x2r.store_slice(&mut self.flt_x2r[start..start + width]);
        y1r.store_slice(&mut self.flt_y1r[start..start + width]);
        y2r.store_slice(&mut self.flt_y2r[start..start + width]);
        released.store_slice(&mut self.released[start..start + width]);
        held.store_slice(&mut self.held_by_damper[start..start + width]);
    }
}

impl<S: Simd> EnvVecs<S> {
    #[inline(always)]
    fn load_view(simd: S, v: &VoiceSoaView, start: usize, width: usize) -> Self {
        let r = start..start + width;
        Self {
            stage: S::f32s::from_slice(simd, &v.env_stage[r.clone()]),
            progress: S::f32s::from_slice(simd, &v.stage_progress[r.clone()]),
            envelope: S::f32s::from_slice(simd, &v.envelope[r.clone()]),
            env_start: S::f32s::from_slice(simd, &v.env_start[r.clone()]),
            decay_start: S::f32s::from_slice(simd, &v.decay_start[r.clone()]),
            sustain_level: S::f32s::from_slice(simd, &v.sustain_level[r.clone()]),
            peak: S::f32s::from_slice(simd, &v.env_level[r.clone()]),
            delay_frames: S::f32s::from_slice(simd, &v.delay_frames[r.clone()]),
            attack_frames: S::f32s::from_slice(simd, &v.attack_frames[r.clone()]),
            hold_frames: S::f32s::from_slice(simd, &v.hold_frames[r.clone()]),
            decay_frames: S::f32s::from_slice(simd, &v.decay_frames[r.clone()]),
            release_frames: S::f32s::from_slice(simd, &v.release_frames[r]),
        }
    }

    #[inline(always)]
    fn store_view(&self, v: &mut VoiceSoaView, start: usize, width: usize) {
        let r = start..start + width;
        self.stage.store_slice(&mut v.env_stage[r.clone()]);
        self.progress.store_slice(&mut v.stage_progress[r.clone()]);
        self.envelope.store_slice(&mut v.envelope[r.clone()]);
        self.env_start.store_slice(&mut v.env_start[r.clone()]);
        self.decay_start.store_slice(&mut v.decay_start[r.clone()]);
    }
}

#[cfg(test)]
mod tests;
