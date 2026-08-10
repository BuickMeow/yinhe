//! GPU 合成器高层封装 — 统一播放和导出接口。
//!
//! 和 xsynth 的 ChannelGroup 对等：
//! - `note_on` / `note_off` 接收 MIDI 事件
//! - `render` 一次性渲染整个 block
//! - `load_events` 批量加载预排序事件列表（用于导出/Seek）
//!
//! voice 管理、ADSR 推进、限幅全部封装在内部。

use std::collections::HashMap;
use std::sync::Arc;

use xsynth_core::effects::VolumeLimiter;

use crate::sfz_parser;
use crate::synth::{GpuAudioRenderer, GpuVoiceState, advance_voices};
use crate::wgpu;

/// 一个 MIDI 事件（NoteOn 或 NoteOff）
#[derive(Clone, Copy, Debug)]
pub struct SynthEvent {
    /// 全局采样位置
    pub sample: u64,
    pub key: u8,
    pub velocity: u8,
    pub is_on: bool,
}

/// voice + 对应的 MIDI key
#[derive(Clone, Debug)]
struct Voice {
    state: GpuVoiceState,
    key: u8,
}

/// GPU 合成器 — 封装 GPU 渲染器 + voice 管理 + 事件调度 + 限幅。
///
/// 接口设计参照 xsynth ChannelGroup：
/// - 播放时通过 `note_on`/`note_off` 逐事件分发
/// - 导出时通过 `load_events` 批量加载排序好的事件列表
/// - 两种场景都调用 `render()` 获取音频数据
pub struct GpuSynth {
    renderer: GpuAudioRenderer,
    key_map: Vec<Vec<sfz_parser::KeyInfo>>,
    /// 采样数据在 GPU 上传块中的 (offset, len)，按 Arc 身份（指针 as usize）去重
    sample_offsets: HashMap<usize, (u32, u32)>,
    voices: Vec<Voice>,
    /// 预分配的 voice states 缓冲区，避免每帧分配
    states_buf: Vec<GpuVoiceState>,
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
        self.event_cursor = self.events.partition_point(|e| e.sample < sample);
        self.voices.clear();
    }

    /// 渲染一块音频到 output（output.len() = frames * 2，立体声交错）
    pub fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;
        if frames == 0 {
            return;
        }

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;
        output.fill(0.0);

        // 从事件列表分发
        while self.event_cursor < self.events.len() {
            let ev = self.events[self.event_cursor];
            if ev.sample >= block_end {
                break;
            }
            if ev.sample >= block_start {
                let offset = (ev.sample - block_start) as u32;
                if ev.is_on {
                    self.note_on(ev.key, ev.velocity, offset);
                } else {
                    self.note_off(ev.key);
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

        // 限幅（真实路径保留；对比测试可关闭）
        if self.limiter_enabled {
            self.limiter.limit(output);
        }

        self.sample_position = block_end;
    }

    /// NoteOn（块内偏移由 offset_in_block 指定）。
    /// key_map 已按 (key, vel) 展开为最终参数快照，这里零公式计算直接消费。
    pub fn note_on(&mut self, key: u8, vel: u8, offset_in_block: u32) {
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

        // 声像：等功率法则（xsynth stereo spawner 公式，左右各 1.42 补偿）
        let angle = info.pan * std::f32::consts::FRAC_PI_2;
        let (pan_l, pan_r) = ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0));
        // channel 层默认中置 pan（xsynth 默认 0.5 → 左右 0.707），P3 做 CC 时改为实时控制
        let (pan_l, pan_r) = (
            pan_l * std::f32::consts::FRAC_1_SQRT_2,
            pan_r * std::f32::consts::FRAC_1_SQRT_2,
        );

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

        let sr = self.sample_rate as f32;
        self.voices.push(Voice {
            key,
            state: GpuVoiceState {
                sample_offset: offset + info.offset,
                sample_length: length - info.offset.min(length),
                speed: info.speed_mult,
                gain: info.volume,
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
                pan_left: pan_l,
                pan_right: pan_r,
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

    /// NoteOff — 将最后一个匹配的 voice 转入 Release 阶段
    pub fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut().rev() {
            if v.key == key && v.state.env_stage < 5 {
                v.state.env_start = v.state.envelope;
                v.state.env_stage = 5;
                v.state.stage_progress = 0.0;
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
