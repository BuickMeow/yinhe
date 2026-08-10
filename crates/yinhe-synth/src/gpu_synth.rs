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
use crate::synth::{GpuAudioRenderer, GpuVoiceState, advance_voices};
use crate::wgpu;

/// MIDI 通道数（标准 16 通道）。
pub const MAX_CHANNELS: usize = 16;

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
                    // Reset All Controllers
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
}

/// voice + MIDI key + 所属通道 + 通道无关的基础参数。
#[derive(Clone, Debug)]
struct Voice {
    state: GpuVoiceState,
    key: u8,
    channel: u8,
    /// 音色库基础播放倍率（不含弯音）。
    base_speed: f32,
    /// 是否被延音踏板保持（CC64 踩着时 note_off 只标记不释放）。
    held_by_damper: bool,
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
    /// 16 通道 MIDI 控制状态
    channels: [ChannelState; MAX_CHANNELS],
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
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
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
    /// block 内按 CC 事件位置分段渲染：音符事件支持块内偏移，
    /// CC 事件在事件位置生效（与 CPU 路径的事件级时序对齐），
    /// 每段一次 GPU 提交。无 CC 的 block 仍是单段单次提交。
    pub fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;
        if frames == 0 {
            return;
        }

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;
        output.fill(0.0);

        let mut seg_start = block_start;
        let mut out_off = 0usize;
        loop {
            // 段边界 = block 内下一个 CC 事件位置（事件级时序）
            let next_cc = self.next_cc_in_block(seg_start, block_end);
            let seg_end = next_cc.unwrap_or(block_end);
            let seg_frames = (seg_end - seg_start) as usize;
            let out = &mut output[out_off * 2..(out_off + seg_frames) * 2];
            self.render_segment(out, seg_start, seg_end);
            out_off += seg_frames;
            seg_start = seg_end;

            if seg_end >= block_end {
                break;
            }

            // 段边界处处理该位置的所有事件（Control 更新通道状态；
            // 音符事件以 offset=0 分发——段边界即段开头）
            while self.event_cursor < self.events.len() {
                let ev = self.events[self.event_cursor];
                if ev.sample() != seg_end {
                    break;
                }
                match ev {
                    SynthEvent::Control { channel, event, .. } => {
                        self.process_control(channel, event);
                    }
                    SynthEvent::NoteOn {
                        channel,
                        key,
                        velocity,
                        ..
                    } => {
                        self.note_on(channel, key, velocity, 0);
                    }
                    SynthEvent::NoteOff { channel, key, .. } => {
                        self.note_off(channel, key);
                    }
                }
                self.event_cursor += 1;
            }
        }

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

    /// 渲染一段音频 [seg_start, seg_end)（段内分发音符事件，段内无 CC）。
    fn render_segment(&mut self, output: &mut [f32], seg_start: u64, seg_end: u64) {
        let frames = output.len() / 2;
        if frames == 0 {
            return;
        }

        // 段起点对齐所有 voice 的通道状态：弯音倍率 + 渐变当前值/步长/剩余帧数。
        // CPU 通道状态按段批量推进，与 shader 逐帧推进在无事件区间线性一致，
        // 因此段起点处两者相等；事件（含 CC 渐变变更）在段边界处理后在此生效。
        self.sync_channel_state();

        // 段内分发音符事件（CC 事件 sample >= seg_end 由主循环处理）
        while self.event_cursor < self.events.len() {
            let ev = self.events[self.event_cursor];
            if ev.sample() >= seg_end {
                break;
            }
            if ev.sample() >= seg_start {
                let offset = (ev.sample() - seg_start) as u32;
                match ev {
                    SynthEvent::NoteOn {
                        channel,
                        key,
                        velocity,
                        ..
                    } => self.note_on(channel, key, velocity, offset),
                    SynthEvent::NoteOff { channel, key, .. } => {
                        self.note_off(channel, key);
                    }
                    SynthEvent::Control { .. } => unreachable!("CC 由主循环在段边界处理"),
                }
            }
            self.event_cursor += 1;
        }

        // GPU 渲染：提取 voice states 到预分配缓冲区，零额外堆分配
        if !self.voices.is_empty() {
            self.states_buf.clear();
            self.states_buf.extend(self.voices.iter().map(|v| v.state));
            // 直接写入 output，避免中间 Vec 分配；渲染后读回滤波器 IIR 状态
            self.renderer
                .render_into(&mut self.states_buf, output, self.sample_rate);
            for (i, v) in self.voices.iter_mut().enumerate() {
                v.state.flt_x1 = self.states_buf[i].flt_x1;
                v.state.flt_x2 = self.states_buf[i].flt_x2;
                v.state.flt_y1 = self.states_buf[i].flt_y1;
                v.state.flt_y2 = self.states_buf[i].flt_y2;
                v.state.flt_x1r = self.states_buf[i].flt_x1r;
                v.state.flt_x2r = self.states_buf[i].flt_x2r;
                v.state.flt_y1r = self.states_buf[i].flt_y1r;
                v.state.flt_y2r = self.states_buf[i].flt_y2r;
            }
        }

        // 原地推进 voice 状态
        for v in &mut self.voices {
            advance_voices(std::slice::from_mut(&mut v.state), frames as u32);
        }

        // 清理已结束的 voice
        self.voices.retain(|v| v.state.env_stage < 6);

        // 推进通道渐变状态（与 xsynth 逐样本推进对齐；voice 侧由 shader 逐帧推进）
        for ch in &mut self.channels {
            ch.volume.advance(frames as f32);
            ch.expression.advance(frames as f32);
            ch.pan.advance(frames as f32);
        }
    }

    /// 段起点同步所有 voice：弯音倍率（speed）+ 通道渐变快照（ch_vol/expr/pan）。
    /// 与 note_on 快照、shader 逐帧推进三方一致（无事件区间线性，事件边界重对齐）。
    fn sync_channel_state(&mut self) {
        for v in &mut self.voices {
            // dense 通道号可能超过 15（多端口 MIDI：port×16+ch），取模折叠到 16 通道状态
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

    /// 处理一个通道控制事件（含 damper 松开时释放 held voices）。
    fn process_control(&mut self, channel: u8, event: ControlEvent) {
        let ch_idx = channel as usize % MAX_CHANNELS;
        match event {
            ControlEvent::Raw(0x78, 0) => {
                // All Sounds Off：立即结束所有 voice
                for v in &mut self.voices {
                    v.state.env_stage = 6;
                }
                return;
            }
            ControlEvent::Raw(0x7B, 0) => {
                // All Notes Off：释放该通道所有非 held voice（held 等待 damper 松开）
                for v in &mut self.voices {
                    if v.channel == channel && !v.held_by_damper && v.state.env_stage < 5 {
                        v.state.env_start = v.state.envelope;
                        v.state.env_stage = 5;
                        v.state.stage_progress = 0.0;
                    }
                }
                return;
            }
            _ => {}
        }
        let ch = &mut self.channels[ch_idx];
        let damper_released = ch.process_control(event);
        if damper_released {
            // 松开延音踏板：释放该通道所有被保持的 voice
            for v in &mut self.voices {
                if v.channel == channel {
                    if v.held_by_damper && v.state.env_stage < 5 {
                        v.state.env_start = v.state.envelope;
                        v.state.env_stage = 5;
                        v.state.stage_progress = 0.0;
                    }
                    v.held_by_damper = false;
                }
            }
        }
        // 弯音/渐变在下一段起点由 sync_channel_state 统一应用
    }

    /// NoteOn（块内偏移由 offset_in_block 指定）。
    /// key_map 已按 (key, vel) 展开为最终参数快照，这里零公式计算直接消费。
    pub fn note_on(&mut self, channel: u8, key: u8, vel: u8, offset_in_block: u32) {
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
        // 通道渐变快照：note 起点处（seg_start + offset）的通道值 + 剩余渐变帧数。
        // shader 内 voice 与通道以相同步长逐帧推进，事件边界由 sync_channel_voices 重新对齐。
        let offset_f = offset_in_block as f32;
        self.voices.push(Voice {
            key,
            channel,
            base_speed: info.speed_mult,
            held_by_damper: false,
            state: GpuVoiceState {
                sample_offset: offset + info.offset,
                sample_length: length - info.offset.min(length),
                speed: info.speed_mult * ch.pitch_multiplier(),
                base_gain: info.volume,
                time: 0.0,
                start_offset: offset_in_block,
                envelope: info.ampeg_start,
                env_stage: 0,
                stage_progress: 0.0,
                // envelope 归一化 0..1，增益由 gain 单独乘（xsynth 语义）
                env_level: 1.0,
                sustain_level: info.ampeg_sustain,
                env_start: info.ampeg_start,
                delay_frames: info.ampeg_delay * sr,
                attack_frames: info.ampeg_attack * sr,
                hold_frames: info.ampeg_hold * sr,
                decay_frames: info.ampeg_decay * sr,
                release_frames: info.ampeg_release * sr,
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
    }

    /// NoteOff — 释放该 (channel, key) 最老的未释放 voice（与 xsynth 一致）。
    /// 延音踏板踩着时只标记 held，不释放。
    pub fn note_off(&mut self, channel: u8, key: u8) {
        let damper = self.channels[channel as usize % MAX_CHANNELS].damper;
        for v in self.voices.iter_mut() {
            if v.channel == channel && v.key == key && v.state.env_stage < 5 {
                if damper {
                    v.held_by_damper = true;
                } else {
                    v.state.env_start = v.state.envelope;
                    v.state.env_stage = 5;
                    v.state.stage_progress = 0.0;
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
