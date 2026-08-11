//! GPU 合成器高层封装 — 统一播放和导出接口。
//!
//! 和 xsynth 的 ChannelGroup 对等：
//! - `note_on` / `note_off` / CC 控制事件接收
//! - 16 通道 MIDI 状态机（volume/expression/pan/pitch bend/RPN/damper）
//! - `render` 一次性渲染整个 block
//! - `load_events` 批量加载预排序事件列表（用于导出/Seek）
//!
//! voice 管理、通道状态、ADSR 推进、限幅全部封装在内部。

use std::collections::HashMap;
use std::sync::Arc;

use crate::limiter::VolumeLimiter;
use crate::sfz_parser;
use crate::synth::GpuAudioRenderer;
use crate::synth::{ChState, EnvUpdateCmd, GpuVoiceState, ReleaseCmd, SegInfo};
use crate::wgpu;

/// MIDI 通道数（dense 通道 = port×16+ch，支持 2 端口 32 通道）。
pub const MAX_CHANNELS: usize = 32;

/// 合成器事件（sample 域，按 sample 排序后由 `load_events` 加载）。
#[derive(Clone, Copy, Debug)]
pub enum SynthEvent {
    NoteOn {
        sample: u64,
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        sample: u64,
        channel: u8,
        key: u8,
    },
    Control {
        sample: u64,
        channel: u8,
        event: ControlEvent,
    },
}

impl SynthEvent {
    pub fn sample(&self) -> u64 {
        match self {
            SynthEvent::NoteOn { sample, .. }
            | SynthEvent::NoteOff { sample, .. }
            | SynthEvent::Control { sample, .. } => *sample,
        }
    }
}

/// 通道控制事件（语义与 xsynth `ControlEvent` 对齐）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlEvent {
    /// 原始 MIDI CC 事件 (controller, value)。
    Raw(u8, u8),
    /// 弯音值 -1..1。
    PitchBend(f32),
    /// 弯音灵敏度（半音）。
    PitchBendSensitivity(f32),
    /// 微调（音分）。
    FineTune(f32),
    /// 粗调（半音）。
    CoarseTune(f32),
    /// 音色更换（单音色库下仅记录，无行为）。
    ProgramChange(u8),
}

/// 通道渐变值（与 xsynth `ValueLerp` 语义对齐）：CC7/10/11 的值在
/// 10ms（sample_rate × 0.01 帧）内线性渐变，`set_end` 从当前值起算步长，
/// 逐样本推进并钳制到终点。
#[derive(Clone, Copy, Debug)]
struct ValueLerp {
    length: f32,
    step: f32,
    current: f32,
    end: f32,
}

impl ValueLerp {
    fn new(current: f32, sample_rate: u32) -> Self {
        Self {
            length: sample_rate as f32 * 0.01,
            step: 0.0,
            current,
            end: current,
        }
    }

    fn set_end(&mut self, end: f32) {
        self.step = (end - self.current) / self.length;
        self.end = end;
    }

    /// 推进 `frames` 帧后的值（线性 + 终点钳制，与逐帧 get_next 终点一致）。
    fn value_at(&self, frames: f32) -> f32 {
        if self.step > 0.0 {
            (self.current + self.step * frames).min(self.end)
        } else if self.step < 0.0 {
            (self.current + self.step * frames).max(self.end)
        } else {
            self.current
        }
    }

    fn advance(&mut self, frames: f32) {
        self.current = self.value_at(frames);
    }

    /// `offset` 帧后该渐变还剩余多少帧（0 = 已到终点或未在渐变）。
    fn frames_after(&self, offset: f32) -> u32 {
        if self.step == 0.0 {
            return 0;
        }
        let remaining = (self.end - self.current) / self.step;
        (remaining - offset).max(0.0).ceil() as u32
    }
}

/// 通道级低通滤波器状态（CC74 cutoff + CC71 resonance 的 DF1，每声道独立）。
/// 状态跨 block 保留；系数变化（cutoff 渐变）时 DF1 状态保留（biquad DF1 在线重调）。
#[derive(Clone, Copy, Debug, Default)]
struct ChannelFltState {
    x1l: f32,
    x2l: f32,
    y1l: f32,
    y2l: f32,
    x1r: f32,
    x2r: f32,
    y1r: f32,
    y2r: f32,
}

/// 单通道 MIDI 控制状态（默认值与 xsynth `ControlEventData::new_defaults` 对齐）。
#[derive(Clone, Copy, Debug)]
struct ChannelState {
    volume: ValueLerp,           // 0..1（CC7），10ms 渐变
    expression: ValueLerp,       // 0..1（CC11），10ms 渐变
    pan: ValueLerp,              // 0..1（CC10/8），10ms 渐变
    damper: bool,                // CC64 >= 64
    pitch_bend: f32,             // -1..1
    pitch_bend_sensitivity: f32, // 半音（RPN0 = msb + lsb/100），默认 2
    pbs_msb: u8,                 // RPN0 data msb（CC6）
    pbs_lsb: u8,                 // RPN0 data lsb（CC38）
    fine_tune: f32,              // 音分（RPN1）
    fine_tune_msb: u8,           // RPN1 data msb（CC6）
    fine_tune_lsb: u8,           // RPN1 data lsb（CC38）
    coarse_tune: f32,            // 半音（RPN2）
    program: u8,
    // RPN 选择器状态（CC100/101）
    rpn_msb: i8,
    rpn_lsb: i8,
    /// 渐变长度基准（CC79 重置时需要重建 ValueLerp）
    sample_rate: u32,
    /// 通道级低通（CC74 目标频率；None = 旁路，DF1 状态保留）
    cutoff: Option<f32>,
    /// 通道滤波器 Q（CC71；None = Butterworth 0.7071），仅 cutoff 开启时生效
    resonance: Option<f32>,
    /// 截止频率渐变（初始 sr/2 = 全通，与 xsynth MultiChannelBiQuad 一致）
    cutoff_lerp: ValueLerp,
    /// set_end 的 512 帧对齐基准（= 最近一次 CC 事件位置；xsynth 每个 read 块
    /// 末 set_end，块从事件位置重新 512 对齐，跨 render block 保持）
    cutoff_align: u64,
    /// DF1 状态（每声道，跨 block 保留）
    flt: ChannelFltState,
    /// CC73 attack 时长倍率（u8，None = 用 region 原始值）
    env_attack: Option<u8>,
    /// CC72 release 时长倍率（u8，None = 用 region 原始值）
    env_release: Option<u8>,
}

impl ChannelState {
    fn new(sample_rate: u32) -> Self {
        Self {
            volume: ValueLerp::new(1.0, sample_rate),
            expression: ValueLerp::new(1.0, sample_rate),
            pan: ValueLerp::new(0.5, sample_rate),
            damper: false,
            pitch_bend: 0.0,
            pitch_bend_sensitivity: 2.0,
            pbs_msb: 2,
            pbs_lsb: 0,
            fine_tune: 0.0,
            fine_tune_msb: 0,
            fine_tune_lsb: 0,
            coarse_tune: 0.0,
            program: 0,
            rpn_msb: -1,
            rpn_lsb: -1,
            sample_rate,
            cutoff: None,
            resonance: None,
            // 初始频率 = sr/2（≈全通），与 xsynth MultiChannelBiQuad 构造一致
            cutoff_lerp: ValueLerp::new(sample_rate as f32 / 2.0, sample_rate),
            cutoff_align: 0,
            flt: ChannelFltState::default(),
            env_attack: None,
            env_release: None,
        }
    }

    /// 弯音倍率：2^((bend×sensitivity + coarse + fine/100) / 12)（与 xsynth 一致）。
    fn pitch_multiplier(&self) -> f32 {
        let combined = self.pitch_bend * self.pitch_bend_sensitivity
            + self.coarse_tune
            + self.fine_tune / 100.0;
        2.0f32.powf(combined / 12.0)
    }

    /// 处理一个控制事件（语义对齐 xsynth `process_control_event`）。
    /// 返回是否触发了 damper 松开（需要释放 held voices）。
    fn process_control(&mut self, event: ControlEvent) -> bool {
        match event {
            ControlEvent::Raw(controller, value) => match controller {
                0x00 => { /* Bank select：单音色库忽略 */ }
                0x64 => self.rpn_lsb = value as i8,
                0x65 => self.rpn_msb = value as i8,
                0x06 | 0x26 => {
                    if self.rpn_msb == 0 {
                        match self.rpn_lsb {
                            0 => {
                                // Pitch bend sensitivity（RPN0 = msb + lsb/100）
                                if controller == 0x06 {
                                    self.pbs_msb = value;
                                } else {
                                    self.pbs_lsb = value;
                                }
                                self.pitch_bend_sensitivity =
                                    self.pbs_msb as f32 + self.pbs_lsb as f32 / 100.0;
                            }
                            1 => {
                                // Fine tune（RPN1，14-bit：msb<<6 + lsb）
                                if controller == 0x06 {
                                    self.fine_tune_msb = value;
                                } else {
                                    self.fine_tune_lsb = value;
                                }
                                let val: u16 =
                                    ((self.fine_tune_msb as u16) << 6) + self.fine_tune_lsb as u16;
                                self.fine_tune = (val as f32 - 4096.0) / 4096.0 * 100.0;
                            }
                            2 if controller == 0x06 => {
                                // Coarse tune（RPN2）
                                self.coarse_tune = value as f32 - 64.0;
                            }
                            _ => {}
                        }
                    }
                }
                0x07 => self.volume.set_end(value as f32 / 128.0),
                0x0A | 0x08 => self.pan.set_end(value as f32 / 128.0),
                0x0B => self.expression.set_end(value as f32 / 128.0),
                0x47 if value > 64 => {
                    // CC71 resonance：线性 Q = db_to_amp((v-64)/2.4) × Butterworth 基准
                    let db = (value as f32 - 64.0) / 2.4;
                    self.resonance =
                        Some(10.0f32.powf(db / 20.0) * std::f32::consts::FRAC_1_SQRT_2);
                }
                0x47 => self.resonance = None,
                0x48 => self.env_release = Some(value),
                0x49 => self.env_attack = Some(value),
                0x4A if value < 64 => {
                    // CC74 cutoff：键频表 FREQS[value+64] = 2^((key-69)/12)×440，
                    // 超 7000Hz 的部分 ×2.36 抬升（与 xsynth 一致）
                    let key = value as f32 + 64.0;
                    let mut freq = 2.0f32.powf((key - 69.0) / 12.0) * 440.0;
                    if freq > 7000.0 {
                        let mult = freq / 7000.0 - 1.0;
                        freq = (mult * 2.36 + 1.0) * 7000.0;
                    }
                    self.cutoff = Some(freq);
                }
                0x4A => self.cutoff = None,
                0x40 => {
                    let damper = value >= 64;
                    let released = self.damper && !damper;
                    self.damper = damper;
                    return released;
                }
                0x78 if value == 0 => {
                    // All Sounds Off：立即结束所有 voice
                    return false; // 由调用方处理
                }
                0x79 if value == 0 => {
                    // Reset All Controllers（含 cutoff 旁路；DF1 状态保留，与 xsynth 一致）
                    *self = ChannelState::new(self.sample_rate);
                    return true; // damper 松开语义
                }
                0x7B if value == 0 => {
                    // All Notes Off：释放所有非 held voice
                    return false;
                }
                _ => {}
            },
            ControlEvent::PitchBend(value) => self.pitch_bend = value,
            ControlEvent::PitchBendSensitivity(value) => self.pitch_bend_sensitivity = value,
            ControlEvent::FineTune(value) => self.fine_tune = value,
            ControlEvent::CoarseTune(value) => self.coarse_tune = value,
            ControlEvent::ProgramChange(value) => self.program = value,
        }
        false
    }

    /// 通道级低通滤波（CC74 开启时）：混音后最后一步，作用于该通道的立体声混音。
    /// 与 xsynth `MultiChannelBiQuad` 对齐：
    /// - 每声道一个 DF1 biquad，系数按 RBJ LowPass cookbook（与 per-voice 同源）
    /// - 截止频率 ValueLerp 渐变；每 2 sample（声道对起点）取一次渐变值并更新系数
    /// - set_end 按 512 帧块边界执行（xsynth 每 read 块末 set_end，从当前值重算 step），
    ///   块从最近事件位置（`cutoff_align`）重新对齐并跨 render block 保持——
    ///   xsynth 的块只被事件位置切断，不会被 render 的 512 帧 block 边界切断
    /// - Q = CC71 线性 Q，未设置时 Butterworth（0.7071）
    /// - 旁路（cutoff None）时不处理，DF1 状态保留（无 click）
    fn apply_cutoff_filter(&mut self, mix: &mut [f32], sample_rate: u32, seg_start: u64) {
        let Some(cutoff) = self.cutoff else {
            return;
        };
        let q = self.resonance.unwrap_or(std::f32::consts::FRAC_1_SQRT_2);
        let mut pair = 0u64;
        while (pair as usize) * 2 + 1 < mix.len() {
            // 音频帧位置（pair = 帧索引；每帧 = L+R 两个 sample）。
            // set_end 每 512 帧一次（xsynth 每 read 块一次，块 = 事件位置起 512 帧对齐）。
            let frame = seg_start + pair;
            if frame >= self.cutoff_align && (frame - self.cutoff_align).is_multiple_of(512) {
                self.cutoff_lerp.set_end(cutoff);
            }
            self.cutoff_lerp.advance(1.0);
            let freq = self.cutoff_lerp.current;
            let (b0, b1, b2, a1, a2) = crate::synth::biquad_coeffs(0, freq, q, sample_rate as f32);
            // 左声道
            let x = mix[pair as usize * 2];
            let y = b0 * x + b1 * self.flt.x1l + b2 * self.flt.x2l
                - a1 * self.flt.y1l
                - a2 * self.flt.y2l;
            self.flt.x2l = self.flt.x1l;
            self.flt.x1l = x;
            self.flt.y2l = self.flt.y1l;
            self.flt.y1l = y;
            mix[pair as usize * 2] = y;
            // 右声道
            let x = mix[pair as usize * 2 + 1];
            let y = b0 * x + b1 * self.flt.x1r + b2 * self.flt.x2r
                - a1 * self.flt.y1r
                - a2 * self.flt.y2r;
            self.flt.x2r = self.flt.x1r;
            self.flt.x1r = x;
            self.flt.y2r = self.flt.y1r;
            self.flt.y1r = y;
            mix[pair as usize * 2 + 1] = y;
            pair += 1;
        }
    }
}

/// xsynth `calculate_curve`：CC72/73 值缩放 region 原始时长（秒）。
/// v<=64: (v/64)^5 × dur；v>64: dur + ((v-64)/64)^3 × 15
/// release 有 0.02s 下限；attack 无下限。返回帧数。
fn env_curve_frames(value: u8, orig_frames: f32, sample_rate: u32, is_release: bool) -> f32 {
    let dur = orig_frames / sample_rate as f32;
    let curve = if value <= 64 {
        (value as f32 / 64.0).powi(5) * dur
    } else {
        dur + ((value as f32 - 64.0) / 64.0).powi(3) * 15.0
    };
    let secs = if is_release { curve.max(0.02) } else { curve };
    secs * sample_rate as f32
}

/// voice + MIDI key + 所属通道 + 通道无关的基础参数。
#[derive(Clone, Debug)]
struct Voice {
    state: GpuVoiceState,
    key: u8,
    channel: u8,
    /// 音色库基础播放倍率（不含弯音）。
    base_speed: f32,
    /// region 原始 attack/release 帧数（CC72/73 重算的基准，多次 CC 不累积）
    orig_attack_frames: f32,
    orig_release_frames: f32,
    /// 是否被延音踏板保持（CC64 踩着时 note_off 只标记不释放）。
    held_by_damper: bool,
    /// 已发 release 指令等待 shader 在指令帧应用（防止同 key 重复匹配）。
    /// 不预置 env_stage：预置会让 shader 在指令应用前就按 release 阶段推进
    /// （旧 env_start=0 会把 envelope 清零）。
    release_pending: bool,
}

/// GPU 合成器 — 封装 GPU 渲染器 + voice 管理 + 通道状态 + 事件调度 + 限幅。
///
/// 接口设计参照 xsynth ChannelGroup：
/// - 播放时通过 `load_events` 加载预排序事件，`render` 逐块渲染
/// - 通道状态机处理 CC7/10/11/64/100/101/6/38 + pitch bend + RPN
pub struct GpuSynth {
    renderer: GpuAudioRenderer,
    key_map: Vec<Vec<sfz_parser::KeyInfo>>,
    /// 采样数据在 GPU 上传块中的 (offset, len)，按 Arc 身份（指针 as usize）去重
    sample_offsets: HashMap<usize, (u32, u32)>,
    voices: Vec<Voice>,
    /// 预分配的 voice states 缓冲区，避免每帧分配
    states_buf: Vec<GpuVoiceState>,
    /// 32 通道混音缓冲（GPU 输出读回，CPU 通道滤波 + 求和）
    channel_mix: Vec<f32>,
    /// 32 通道 MIDI 控制状态
    channels: [ChannelState; MAX_CHANNELS],
    /// 全局 voice 上限（黑乐谱长 sustain/无 note_off 的 voice 会累积，
    /// 超限时淘汰最老的 release 中 voice，否则最老的 active——与 xsynth voice 限制同思路）
    max_voices: usize,
    limiter: VolumeLimiter,
    /// 渲染后是否应用限幅器（默认开；对比测试可关闭）
    limiter_enabled: bool,
    sample_rate: u32,
    /// 排序好的事件列表（导出/Seek 用）
    events: Vec<SynthEvent>,
    event_cursor: usize,
    /// 当前渲染位置
    sample_position: u64,
}

impl GpuSynth {
    /// 创建合成器（自动创建 wgpu device/queue）
    pub fn new_default(soundfont_path: &std::path::Path, sample_rate: u32) -> Result<Self, String> {
        let renderer = GpuAudioRenderer::new_default()
            .map_err(|e| format!("GPU renderer init failed: {}", e))?;
        Self::from_renderer(renderer, soundfont_path, sample_rate)
    }

    /// 创建合成器（使用指定的 wgpu device/queue）
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        soundfont_path: &std::path::Path,
        sample_rate: u32,
    ) -> Result<Self, String> {
        let renderer = GpuAudioRenderer::new(device, queue)
            .map_err(|e| format!("GPU renderer init failed: {}", e))?;
        Self::from_renderer(renderer, soundfont_path, sample_rate)
    }

    fn from_renderer(
        mut renderer: GpuAudioRenderer,
        soundfont_path: &std::path::Path,
        sample_rate: u32,
    ) -> Result<Self, String> {
        // key_map 已按 (key, vel) 展开且采样已重采样到目标采样率
        let key_map = sfz_parser::build_key_map(soundfont_path, sample_rate)?;

        // 采样数据按 Arc 身份去重后拼成大块上传 GPU（同一采样被多层共享，零拷贝）
        let mut sample_data: Vec<f32> = Vec::new();
        let mut sample_offsets: HashMap<usize, (u32, u32)> = HashMap::new();
        for key_layers in &key_map {
            for info in key_layers {
                let ptr = info.sample_data.as_ptr() as usize;
                if sample_offsets.contains_key(&ptr) {
                    continue;
                }
                let offset = sample_data.len() as u32;
                sample_data.extend_from_slice(&info.sample_data);
                sample_offsets.insert(ptr, (offset, info.sample_data.len() as u32));
            }
        }

        renderer.upload_samples(&sample_data);

        Ok(Self {
            renderer,
            key_map,
            sample_offsets,
            voices: Vec::new(),
            states_buf: Vec::new(),
            channel_mix: Vec::new(),
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            max_voices: 8192,
            limiter: VolumeLimiter::new(2),
            limiter_enabled: true,
            sample_rate,
            events: Vec::new(),
            event_cursor: 0,
            sample_position: 0,
        })
    }

    /// 批量加载排序好的事件列表（导出/Seek 用）。重置渲染位置到 0。
    pub fn load_events(&mut self, events: Vec<SynthEvent>) {
        self.events = events;
        self.event_cursor = 0;
        self.voices.clear();
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        self.sample_position = 0;
    }

    /// 当前渲染位置
    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// 当前活跃 voice 数量（含 release 阶段）。导出余韵循环用它早退。
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// 开关渲染后的限幅器（默认开）。对比测试用于排除限幅差异。
    pub fn set_limiter_enabled(&mut self, enabled: bool) {
        self.limiter_enabled = enabled;
    }

    /// 设置全局 voice 上限（默认 8192）。超过时淘汰最老的 release 中 voice。
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max;
    }

    /// Seek 到指定位置
    pub fn seek(&mut self, sample: u64) {
        self.sample_position = sample;
        self.event_cursor = self.events.partition_point(|e| e.sample() < sample);
        self.voices.clear();
        // 通道状态在 seek 时重置（chase 由 yinhe-audio 的 cc_events 重建保证）
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
    }

    /// 渲染一块音频到 output（output.len() = frames * 2，立体声交错）。
    ///
    /// 块内事件（CC 段边界、note on/off、release/env 指令）在 CPU 收集为段结构，
    /// **一次 GPU 提交**渲染整块；voice 状态在 GPU 内逐帧推进（块末全字段读回）。
    pub fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;
        if frames == 0 {
            return;
        }

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;
        output.fill(0.0);

        // 收集块内事件为段结构（同时创建 voice、发 release/env 指令、推进 CPU 通道状态）
        let mut segs: Vec<SegInfo> = Vec::new();
        let mut ch_updates: Vec<ChState> = Vec::new();
        let mut releases: Vec<ReleaseCmd> = Vec::new();
        let mut env_cmds: Vec<EnvUpdateCmd> = Vec::new();
        self.collect_block(
            block_start,
            block_end,
            &mut segs,
            &mut ch_updates,
            &mut releases,
            &mut env_cmds,
        );

        // 上传块起点 voice 状态（含块内新增）→ 一次提交 → 全字段读回
        self.states_buf.clear();
        self.states_buf.extend(self.voices.iter().map(|v| v.state));
        self.channel_mix.resize(MAX_CHANNELS * frames * 2, 0.0);
        if !self.states_buf.is_empty() {
            self.renderer.render_block(
                &mut self.states_buf,
                &mut self.channel_mix,
                &segs,
                &ch_updates,
                &releases,
                &env_cmds,
                self.sample_rate,
            );
            // 读回 GPU 推进后的全字段状态（下块起点 = 本块末）
            for (v, st) in self.voices.iter_mut().zip(&self.states_buf) {
                v.state = *st;
            }
        }

        // 各通道：CC74 通道滤波（若开启）→ 求和（xsynth 顺序：vol/pan → cutoff → sum）。
        // 无论有无 voice 都执行：cutoff 渐变与 DF1 状态照常推进（xsynth 的
        // apply_channel_effects 每块无条件调用，对空信号滤波时 lerp 不中断）。
        output.fill(0.0);
        for (ch_idx, ch) in self.channels.iter_mut().enumerate() {
            let base = ch_idx * frames * 2;
            let ch_mix = &mut self.channel_mix[base..base + frames * 2];
            ch.apply_cutoff_filter(ch_mix, self.sample_rate, block_start);
            for (i, o) in output.iter_mut().enumerate() {
                *o += ch_mix[i];
            }
        }

        // 清理已结束的 voice（GPU 推进后的 env_stage）
        self.voices.retain(|v| v.state.env_stage < 6);

        // 限幅（真实路径保留；对比测试可关闭）
        if self.limiter_enabled {
            self.limiter.limit(output);
        }

        self.sample_position = block_end;
    }

    /// 在 [start, block_end) 内查找下一个 CC 事件的位置（从 event_cursor 开始）。
    fn next_cc_in_block(&self, start: u64, block_end: u64) -> Option<u64> {
        for ev in &self.events[self.event_cursor..] {
            let s = ev.sample();
            if s >= block_end {
                return None;
            }
            if s >= start && matches!(ev, SynthEvent::Control { .. }) {
                return Some(s);
            }
        }
        None
    }

    /// 收集块内事件为段结构：
    /// - 段边界 = CC 事件位置（ch_updates 记录受影响通道的状态快照）
    /// - note_on 创建 voice（块内帧偏移）；note_off 发 release 指令（帧 + vid）
    /// - CC72/73/121 发 env 指令；damper 松开/AllNotesOff 发 release/kill 指令
    /// - CPU 通道状态按段推进（与 shader 逐帧推进线性一致）
    fn collect_block(
        &mut self,
        block_start: u64,
        block_end: u64,
        segs: &mut Vec<SegInfo>,
        ch_updates: &mut Vec<ChState>,
        releases: &mut Vec<ReleaseCmd>,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        // 块起点：所有 voice 对齐通道状态（speed/ch_vol/expr/pan）。
        // 块起点若有 CC 事件（sample == block_start），其更新记录在段 0，
        // 由 shader 在初始化时应用——段 0 渲染起点值 = 这里的 sync 值（与现状一致）。
        self.sync_channel_state();

        let mut seg_start = block_start;
        let mut seg_frame = 0u32;
        let mut seg_ch_off = 0usize;

        loop {
            let next_cc = self.next_cc_in_block(seg_start, block_end);

            // 段 [seg_start, next_cc) 内的音符事件（sample == next_cc 的留给段边界）
            while self.event_cursor < self.events.len() {
                let ev = self.events[self.event_cursor];
                if ev.sample() >= next_cc.unwrap_or(block_end) || ev.sample() >= block_end {
                    break;
                }
                if ev.sample() >= seg_start {
                    let seg_offset = (ev.sample() - seg_start) as u32;
                    let block_frame = seg_frame + seg_offset;
                    match ev {
                        SynthEvent::NoteOn {
                            channel,
                            key,
                            velocity,
                            ..
                        } => {
                            self.note_on(channel, key, velocity, block_frame, seg_offset, releases)
                        }
                        SynthEvent::NoteOff { channel, key, .. } => {
                            self.note_off_to_cmd(channel, key, block_frame, releases);
                        }
                        SynthEvent::Control { .. } => unreachable!("CC 由段边界处理"),
                    }
                }
                self.event_cursor += 1;
            }

            // 段边界（CC 事件位置）：推进通道 → 处理该位置所有事件（CC + 音符）
            let Some(cc_sample) = next_cc.filter(|&s| s < block_end) else {
                break;
            };
            let seg_len = (cc_sample - seg_start) as f32;
            for ch in &mut self.channels {
                ch.volume.advance(seg_len);
                ch.expression.advance(seg_len);
                ch.pan.advance(seg_len);
            }
            let frame = (cc_sample - block_start) as u32;
            let seg_ch_off_before = seg_ch_off;
            self.process_events_at(cc_sample, frame, ch_updates, releases, env_cmds);
            let ch_count = ch_updates.len() - seg_ch_off_before;

            segs.push(SegInfo {
                start_frame: seg_frame,
                ch_off: seg_ch_off_before as u32,
                ch_count: ch_count as u32,
                _pad: 0,
            });
            seg_frame = frame;
            seg_ch_off = ch_updates.len();
            seg_start = cc_sample;
        }

        // 最后一段 [seg_start, block_end)
        let last_len = (block_end - seg_start) as f32;
        for ch in &mut self.channels {
            ch.volume.advance(last_len);
            ch.expression.advance(last_len);
            ch.pan.advance(last_len);
        }
        segs.push(SegInfo {
            start_frame: seg_frame,
            ch_off: seg_ch_off as u32,
            ch_count: (ch_updates.len() - seg_ch_off) as u32,
            _pad: 0,
        });
    }

    /// 处理段边界（同一 sample 位置）的所有事件：CC 更新通道状态并记录 ch_updates、
    /// 音符按偏移 0 分发（note_on 用段边界通道值快照）；damper 释放 / env 指令同发。
    fn process_events_at(
        &mut self,
        sample: u64,
        frame: u32,
        ch_updates: &mut Vec<ChState>,
        releases: &mut Vec<ReleaseCmd>,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        while self.event_cursor < self.events.len() {
            let ev = self.events[self.event_cursor];
            if ev.sample() != sample {
                break;
            }
            match ev {
                SynthEvent::NoteOn {
                    channel,
                    key,
                    velocity,
                    ..
                } => self.note_on(channel, key, velocity, frame, 0, releases),
                SynthEvent::NoteOff { channel, key, .. } => {
                    self.note_off_to_cmd(channel, key, frame, releases);
                }
                SynthEvent::Control { channel, event, .. } => {
                    let ch_idx = channel as usize % MAX_CHANNELS;
                    match event {
                        ControlEvent::Raw(0x78, 0) => {
                            // All Sounds Off：kill 所有 voice
                            for (i, v) in self.voices.iter_mut().enumerate() {
                                if v.state.env_stage < 6 {
                                    v.state.env_stage = 6;
                                    releases.push(ReleaseCmd {
                                        frame,
                                        vid: i as u32,
                                        mode: 6,
                                        _pad: 0,
                                    });
                                }
                            }
                        }
                        ControlEvent::Raw(0x7B, 0) => {
                            // All Notes Off：kill 所有非 held voice（held 等待 damper 松开）
                            for (i, v) in self.voices.iter_mut().enumerate() {
                                if v.state.env_stage < 6 && !v.held_by_damper {
                                    v.state.env_stage = 6;
                                    releases.push(ReleaseCmd {
                                        frame,
                                        vid: i as u32,
                                        mode: 6,
                                        _pad: 0,
                                    });
                                }
                            }
                        }
                        _ => {
                            let damper_released = self.channels[ch_idx].process_control(event);
                            if damper_released {
                                // 松开延音踏板：释放该通道所有被保持的 voice
                                for (i, v) in self.voices.iter_mut().enumerate() {
                                    if v.channel == channel {
                                        if v.held_by_damper
                                            && v.state.env_stage < 5
                                            && !v.release_pending
                                        {
                                            v.release_pending = true;
                                            releases.push(ReleaseCmd {
                                                frame,
                                                vid: i as u32,
                                                mode: 5,
                                                _pad: 0,
                                            });
                                        }
                                        v.held_by_damper = false;
                                    }
                                }
                            }
                            // CC72/73 修改包络时长、CC121 重置包络：传播到该通道活跃 voice
                            if matches!(
                                event,
                                ControlEvent::Raw(0x48 | 0x49, _) | ControlEvent::Raw(0x79, 0)
                            ) {
                                self.propagate_env_controls_to_cmds(ch_idx, frame, env_cmds);
                            }
                            // 记录该通道的状态快照（shader 段边界应用）
                            let ch = self.channels[ch_idx];
                            // xsynth 每个事件位置都重置 read 块边界（cutoff set_end 的
                            // 512 帧对齐基准随之重排），跨 render block 保持
                            self.channels[ch_idx].cutoff_align = sample;
                            ch_updates.push(ChState {
                                ch: ch_idx as u32,
                                speed_mult: ch.pitch_multiplier(),
                                ch_vol: ch.volume.current,
                                ch_vol_step: ch.volume.step,
                                ch_vol_frames: ch.volume.frames_after(0.0),
                                ch_expr: ch.expression.current,
                                ch_expr_step: ch.expression.step,
                                ch_expr_frames: ch.expression.frames_after(0.0),
                                ch_pan: ch.pan.current,
                                ch_pan_step: ch.pan.step,
                                ch_pan_frames: ch.pan.frames_after(0.0),
                            });
                        }
                    }
                }
            }
            self.event_cursor += 1;
        }
    }

    /// CC72/73（及 CC121 重置）后重算该通道所有活跃 voice 的 attack/release 时长：
    /// 基于 region 原始值重算（多次 CC 不累积），shader 从当前 amp 重走当前阶段。
    fn propagate_env_controls_to_cmds(
        &mut self,
        ch_idx: usize,
        frame: u32,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        let ch = self.channels[ch_idx];
        for (i, v) in self.voices.iter_mut().enumerate() {
            if v.channel as usize != ch_idx || v.state.env_stage >= 6 {
                continue;
            }
            let attack_frames = match ch.env_attack {
                Some(cc) => env_curve_frames(cc, v.orig_attack_frames, self.sample_rate, false),
                None => v.state.attack_frames,
            };
            let release_frames = match ch.env_release {
                Some(cc) => env_curve_frames(cc, v.orig_release_frames, self.sample_rate, true),
                None => v.state.release_frames,
            };
            v.state.attack_frames = attack_frames;
            v.state.release_frames = release_frames;
            env_cmds.push(EnvUpdateCmd {
                frame,
                vid: i as u32,
                attack_frames,
                release_frames,
            });
        }
    }

    /// 段起点同步所有 voice：弯音倍率（speed）+ 通道渐变快照（ch_vol/expr/pan）。
    /// 与 note_on 快照、shader 逐帧推进三方一致（无事件区间线性，事件边界重对齐）。
    fn sync_channel_state(&mut self) {
        for v in &mut self.voices {
            // dense 通道号可能超过 31（多端口 MIDI：port×16+ch），取模折叠到 32 通道状态
            let ch = self.channels[v.channel as usize % MAX_CHANNELS];
            v.state.speed = v.base_speed * ch.pitch_multiplier();
            v.state.ch_vol = ch.volume.current;
            v.state.ch_vol_step = ch.volume.step;
            v.state.ch_vol_frames = ch.volume.frames_after(0.0);
            v.state.ch_expr = ch.expression.current;
            v.state.ch_expr_step = ch.expression.step;
            v.state.ch_expr_frames = ch.expression.frames_after(0.0);
            v.state.ch_pan = ch.pan.current;
            v.state.ch_pan_step = ch.pan.step;
            v.state.ch_pan_frames = ch.pan.frames_after(0.0);
        }
    }

    /// NoteOn（block_frame = 块内起始帧；seg_offset = 段内偏移，用于通道值快照）。
    /// key_map 已按 (key, vel) 展开为最终参数快照，这里零公式计算直接消费。
    /// 超 voice 上限时淘汰最老的 voice（发 kill 指令，不 remove——索引保持稳定）。
    pub fn note_on(
        &mut self,
        channel: u8,
        key: u8,
        vel: u8,
        block_frame: u32,
        seg_offset: u32,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        let info = match sfz_parser::select_key_info(&self.key_map, key, vel) {
            Some(i) => i,
            None => return,
        };
        let (offset, length) = match self
            .sample_offsets
            .get(&(info.sample_data.as_ptr() as usize))
        {
            Some(&v) => v,
            None => return,
        };
        if length == 0 {
            return;
        }

        // 音色库声像：等功率法则（xsynth stereo spawner 公式，左右各 1.42 补偿）
        let angle = info.pan * std::f32::consts::FRAC_PI_2;
        let (base_pan_l, base_pan_r) =
            ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0));

        // per-voice biquad 系数（RBJ cookbook，与 xsynth 一致）；cutoff=0 时无滤波器
        let (flt_b0, flt_b1, flt_b2, flt_a1, flt_a2) = if info.cutoff > 0.0 {
            crate::synth::biquad_coeffs(
                filter_type_to_u32(info.filter_type),
                info.cutoff,
                info.resonance,
                self.sample_rate as f32,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0)
        };

        let ch = self.channels[channel as usize % MAX_CHANNELS];
        let sr = self.sample_rate as f32;
        // CC72/73：用通道当前值缩放 region 原始时长（多次 CC 不累积）
        let orig_attack_frames = info.ampeg_attack * sr;
        let orig_release_frames = info.ampeg_release * sr;
        let attack_frames = match ch.env_attack {
            Some(cc) => env_curve_frames(cc, orig_attack_frames, self.sample_rate, false),
            None => orig_attack_frames,
        };
        let release_frames = match ch.env_release {
            Some(cc) => env_curve_frames(cc, orig_release_frames, self.sample_rate, true),
            None => orig_release_frames,
        };
        // 通道渐变快照：note 起点处（段起点 + seg_offset）的通道值 + 剩余渐变帧数。
        // shader 内 voice 与通道以相同步长逐帧推进，段边界由 ChState 重新对齐。
        let offset_f = seg_offset as f32;

        self.voices.push(Voice {
            key,
            channel,
            base_speed: info.speed_mult,
            orig_attack_frames,
            orig_release_frames,
            held_by_damper: false,
            release_pending: false,
            state: GpuVoiceState {
                sample_offset: offset + info.offset,
                sample_length: length - info.offset.min(length),
                speed: info.speed_mult * ch.pitch_multiplier(),
                base_speed: info.speed_mult,
                base_gain: info.volume,
                time: 0.0,
                start_offset: block_frame,
                // 取模后的通道号（与 ChannelState 索引/pass2 归约一致）
                channel: channel as u32 % MAX_CHANNELS as u32,
                envelope: info.ampeg_start,
                env_stage: 0,
                stage_progress: 0.0,
                // envelope 归一化 0..1，增益由 gain 单独乘（xsynth 语义）
                env_level: 1.0,
                sustain_level: info.ampeg_sustain,
                env_start: info.ampeg_start,
                decay_start: info.ampeg_start,
                delay_frames: info.ampeg_delay * sr,
                attack_frames,
                hold_frames: info.ampeg_hold * sr,
                decay_frames: info.ampeg_decay * sr,
                release_frames,
                base_pan_l,
                base_pan_r,
                ch_vol: ch.volume.value_at(offset_f),
                ch_vol_step: ch.volume.step,
                ch_vol_frames: ch.volume.frames_after(offset_f),
                ch_expr: ch.expression.value_at(offset_f),
                ch_expr_step: ch.expression.step,
                ch_expr_frames: ch.expression.frames_after(offset_f),
                ch_pan: ch.pan.value_at(offset_f),
                ch_pan_step: ch.pan.step,
                ch_pan_frames: ch.pan.frames_after(offset_f),
                loop_start: info.loop_start,
                loop_end: info.loop_end,
                loop_mode: info.loop_mode as u32,
                is_stereo: info.is_stereo as u32,
                interp: info.interp,
                cutoff: info.cutoff,
                resonance: info.resonance,
                filter_type: filter_type_to_u32(info.filter_type),
                flt_b0,
                flt_b1,
                flt_b2,
                flt_a1,
                flt_a2,
                flt_x1: 0.0,
                flt_x2: 0.0,
                flt_y1: 0.0,
                flt_y2: 0.0,
                flt_x1r: 0.0,
                flt_x2r: 0.0,
                flt_y1r: 0.0,
                flt_y2r: 0.0,
            },
        });

        // 超限淘汰：优先杀最老的 release 中 voice（听感最弱），否则杀最老的 active。
        // 只预置 stage 6 + 发 kill 指令，不 remove——本块已生成的指令索引保持稳定。
        while self.voices.len() > self.max_voices {
            let idx = self
                .voices
                .iter()
                .position(|v| v.state.env_stage == 5)
                .unwrap_or(0);
            let v = &mut self.voices[idx];
            if v.state.env_stage < 6 {
                v.state.env_stage = 6;
                v.held_by_damper = false;
                releases.push(ReleaseCmd {
                    frame: block_frame,
                    vid: idx as u32,
                    mode: 6,
                    _pad: 0,
                });
            } else {
                break; // 其余已被淘汰（块末统一清理），不再继续
            }
        }
    }

    /// NoteOff — 释放该 (channel, key) 最老的未释放 voice（与 xsynth `release_next_voice` 一致：
    /// 同 key 多次按下的 voice 逐个释放，后按的 voice 继续响）。
    /// 延音踏板踩着时只标记 held，不释放。
    /// 实际释放由 shader 在 frame 帧应用 release 指令完成。
    pub fn note_off_to_cmd(
        &mut self,
        channel: u8,
        key: u8,
        frame: u32,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        let damper = self.channels[channel as usize % MAX_CHANNELS].damper;
        for (i, v) in self.voices.iter_mut().enumerate() {
            // 跳过已 held 的 voice（xsynth damper 分支只匹配 "isn't being held" 的
            // voice：否则同 key 多个 off 会重复匹配同一个 held voice，其余 voice 永不释放）
            if v.channel == channel
                && v.key == key
                && v.state.env_stage < 5
                && !v.held_by_damper
                && !v.release_pending
            {
                if damper {
                    v.held_by_damper = true;
                } else {
                    v.release_pending = true;
                    releases.push(ReleaseCmd {
                        frame,
                        vid: i as u32,
                        mode: 5,
                        _pad: 0,
                    });
                }
                break;
            }
        }
    }
}

/// xsynth FilterType → shader 滤波器类型编号（与 voice_render.wgsl 一致）
fn filter_type_to_u32(ft: xsynth_soundfonts::FilterType) -> u32 {
    match ft {
        xsynth_soundfonts::FilterType::LowPass => 0,
        xsynth_soundfonts::FilterType::HighPass => 1,
        xsynth_soundfonts::FilterType::BandPass => 2,
        xsynth_soundfonts::FilterType::LowPassPole => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ValueLerp 与 xsynth 语义逐项核对：10ms 线性、set_end 从当前值起算、终点钳制。
    #[test]
    fn value_lerp_matches_xsynth() {
        let mut v = ValueLerp::new(1.0, 44100);
        assert_eq!(v.length, 441.0);
        assert_eq!(v.value_at(0.0), 1.0);

        // CC7=100 → 0.78125：441 帧线性到终点
        v.set_end(100.0 / 128.0);
        assert_eq!(v.step, (100.0 / 128.0 - 1.0) / 441.0);
        assert!((v.value_at(220.0) - (1.0 + v.step * 220.0)).abs() < 1e-6);
        assert_eq!(v.value_at(1000.0), 100.0 / 128.0); // 钳制在终点
        assert_eq!(v.frames_after(0.0), 441);
        assert_eq!(v.frames_after(500.0), 0); // 已越过终点

        // 中途二次 set_end：从当前值重算步长
        v.advance(100.0);
        v.set_end(0.0);
        let cur = v.current;
        assert!(cur < 1.0 && cur > 100.0 / 128.0);
        assert_eq!(v.step, (0.0 - cur) / 441.0);
        assert_eq!(v.value_at(441.0), 0.0);
        assert_eq!(v.frames_after(0.0), 441);

        // 不变更目标时 step == 0
        let mut v2 = ValueLerp::new(0.5, 44100);
        assert_eq!(v2.step, 0.0);
        assert_eq!(v2.frames_after(0.0), 0);
        v2.set_end(0.5);
        assert_eq!(v2.step, 0.0);
    }

    /// 通道状态机：CC7/10/11 渐变目标、CC64 damper 阈值、RPN、CC79 重置。
    #[test]
    fn channel_cc_semantics() {
        let mut ch = ChannelState::new(44100);

        // CC7/10/11 → set_end（当前值不变，终点/步长更新）
        ch.process_control(ControlEvent::Raw(7, 100));
        assert_eq!(ch.volume.end, 100.0 / 128.0);
        ch.process_control(ControlEvent::Raw(10, 80));
        assert_eq!(ch.pan.end, 80.0 / 128.0);
        ch.process_control(ControlEvent::Raw(11, 64));
        assert_eq!(ch.expression.end, 0.5);
        // CC8 balance 与 CC10 同语义
        ch.process_control(ControlEvent::Raw(8, 30));
        assert_eq!(ch.pan.end, 30.0 / 128.0);

        // CC64：<64 关，>=64 开；松开返回 true
        assert!(!ch.process_control(ControlEvent::Raw(64, 63)));
        assert!(!ch.damper);
        assert!(!ch.process_control(ControlEvent::Raw(64, 127)));
        assert!(ch.damper);
        assert!(ch.process_control(ControlEvent::Raw(64, 0)));
        assert!(!ch.damper);

        // RPN0 弯音灵敏度（默认 2.0）：CC101/100 选择 RPN0，CC6 设 msb，CC38 设 lsb
        let mut ch = ChannelState::new(44100);
        ch.process_control(ControlEvent::Raw(0x65, 0)); // RPN msb
        ch.process_control(ControlEvent::Raw(0x64, 0)); // RPN lsb = 0
        ch.process_control(ControlEvent::Raw(0x06, 5));
        assert_eq!(ch.pitch_bend_sensitivity, 5.0);
        ch.process_control(ControlEvent::Raw(0x26, 50));
        assert_eq!(ch.pitch_bend_sensitivity, 5.5);

        // RPN1 微调（14-bit：msb<<6 + lsb）
        ch.process_control(ControlEvent::Raw(0x64, 1));
        ch.process_control(ControlEvent::Raw(0x06, 64));
        ch.process_control(ControlEvent::Raw(0x26, 0));
        assert_eq!(ch.fine_tune, (4096.0 - 4096.0) / 4096.0 * 100.0); // 中心
        ch.process_control(ControlEvent::Raw(0x06, 65));
        ch.process_control(ControlEvent::Raw(0x26, 0));
        assert!((ch.fine_tune - (4160.0 - 4096.0) / 4096.0 * 100.0).abs() < 1e-3);

        // RPN2 粗调：CC6 设值 - 64
        ch.process_control(ControlEvent::Raw(0x64, 2));
        ch.process_control(ControlEvent::Raw(0x06, 70));
        assert_eq!(ch.coarse_tune, 6.0);

        // 弯音：bend × 灵敏度（5.5）+ 粗调 6 + 微调 1.5625 音分
        ch.process_control(ControlEvent::PitchBend(0.5));
        assert!(
            (ch.pitch_multiplier() - 2.0f32.powf((2.75 + 6.0 + 1.5625 / 100.0) / 12.0)).abs()
                < 1e-5
        );

        // CC79 重置全部控制器（含渐变回到默认）
        assert!(ch.process_control(ControlEvent::Raw(0x79, 0)));
        assert_eq!(ch.volume.end, 1.0);
        assert_eq!(ch.pan.end, 0.5);
        assert_eq!(ch.expression.end, 1.0);
        assert_eq!(ch.pitch_bend_sensitivity, 2.0);
        assert_eq!(ch.coarse_tune, 0.0);
    }
}
