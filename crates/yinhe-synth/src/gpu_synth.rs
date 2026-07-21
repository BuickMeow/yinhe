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

use crate::synth::{GpuAudioRenderer, GpuVoiceState, advance_voices};
use crate::sfz_parser;
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
    sample_offsets: HashMap<String, (u32, u32)>,
    voices: Vec<Voice>,
    /// 预分配的 voice states 缓冲区，避免每帧分配
    states_buf: Vec<GpuVoiceState>,
    limiter: VolumeLimiter,
    sample_rate: u32,
    /// 排序好的事件列表（导出/Seek 用）
    events: Vec<SynthEvent>,
    event_cursor: usize,
    /// 当前渲染位置
    sample_position: u64,
}

impl GpuSynth {
    /// 创建合成器（自动创建 wgpu device/queue）
    pub fn new_default(
        soundfont_path: &std::path::Path,
        sample_rate: u32,
    ) -> Result<Self, String> {
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
        let key_map = sfz_parser::build_key_map(soundfont_path)?;

        // 加载采样数据（按路径/数据去重）
        let mut sample_data: Vec<f32> = Vec::new();
        let mut sample_offsets: HashMap<String, (u32, u32)> = HashMap::new();

        for key_layers in &key_map {
            for info in key_layers {
                let dedup_key = if let Some(ref path) = info.sample_path {
                    path.to_string_lossy().to_string()
                } else if let Some(ref data) = info.sample_data {
                    format!("sf2_{:p}_{}", data.as_ptr(), data.len())
                } else {
                    continue;
                };
                if sample_offsets.contains_key(&dedup_key) { continue; }

                if let Some(ref data) = info.sample_data {
                    let offset = sample_data.len() as u32;
                    // data 是 Arc<[f32]>，sample_rate 匹配时零拷贝共享
                    let samples: Arc<[f32]> = if info.sample_rate != sample_rate {
                        xsynth_soundfonts::resample::resample_vec(data.to_vec(), info.sample_rate as f32, sample_rate as f32)
                    } else {
                        Arc::clone(data)
                    };
                    let len = samples.len() as u32;
                    sample_data.extend_from_slice(&samples);
                    sample_offsets.insert(dedup_key, (offset, len));
                } else if let Some(ref path) = info.sample_path {
                    if path.to_string_lossy() == "missing" { continue; }
                    let offset = sample_data.len() as u32;
                    match sfz_parser::load_wav_as_f32(path) {
                        Ok((samples, src_sr)) => {
                            let samples = if src_sr != sample_rate {
                                xsynth_soundfonts::resample::resample_vec(samples, src_sr as f32, sample_rate as f32).to_vec()
                            } else {
                                samples
                            };
                            let len = samples.len() as u32;
                            sample_data.extend_from_slice(&samples);
                            sample_offsets.insert(dedup_key, (offset, len));
                        }
                        Err(e) => {
                            eprintln!("[gpu-synth] Warning: failed to load {:?}: {}", path, e);
                        }
                    }
                }
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
            sample_rate,
            events: Vec::new(),
            event_cursor: 0,
            sample_position: 0,
        })
    }

    /// 批量加载排序好的事件列表（导出/Seek 用）。
    pub fn load_events(&mut self, events: Vec<SynthEvent>) {
        self.events = events;
        self.event_cursor = 0;
        self.voices.clear();
    }

    /// 当前渲染位置
    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// 当前活跃 voice 数量（含 release 阶段）。导出余韵循环用它早退。
    pub fn voice_count(&self) -> usize {
        self.voices.len()
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
        if frames == 0 { return; }

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;
        output.fill(0.0);

        // 从事件列表分发
        while self.event_cursor < self.events.len() {
            let ev = self.events[self.event_cursor];
            if ev.sample >= block_end { break; }
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
            // 直接写入 output，避免中间 Vec 分配
            self.renderer.render_into(&self.states_buf, output, self.sample_rate);
        }

        // 原地推进 voice 状态
        for v in &mut self.voices {
            advance_voices(std::slice::from_mut(&mut v.state), frames as u32);
        }

        // 清理已结束的 voice
        self.voices.retain(|v| v.state.env_stage < 6);

        // 限幅
        self.limiter.limit(output);

        self.sample_position = block_end;
    }

    /// NoteOn（块内偏移由 offset_in_block 指定）
    pub fn note_on(&mut self, key: u8, vel: u8, offset_in_block: u32) {
        let info = match sfz_parser::select_key_info(&self.key_map, key, vel) {
            Some(i) => i,
            None => return,
        };
        let dedup_key = if let Some(ref path) = info.sample_path {
            path.to_string_lossy().to_string()
        } else if let Some(ref data) = info.sample_data {
            format!("sf2_{:p}_{}", data.as_ptr(), data.len())
        } else {
            return;
        };
        let (offset, length) = match self.sample_offsets.get(&dedup_key) {
            Some(&v) => v,
            None => return,
        };
        if length == 0 { return; }

        let pitch_semitones = (key as f32 - info.pitch_keycenter as f32)
            + info.tune as f32 / 100.0;
        let speed = 2.0f32.powf(pitch_semitones / 12.0);

        let vel_norm = vel as f32 / 127.0;
        let vel_gain = if info.amp_veltrack >= 100.0 {
            vel_norm
        } else {
            vel_norm.powf(100.0 / info.amp_veltrack.max(1.0))
        };
        let gain = vel_gain * info.volume;

        let (pan_l, pan_r) = if info.pan == 0.0 {
            (1.0, 1.0)
        } else {
            let angle = info.pan * std::f32::consts::FRAC_PI_4;
            (angle.cos(), angle.sin())
        };

        let sr = self.sample_rate as f32;
        self.voices.push(Voice {
            key,
            state: GpuVoiceState {
                sample_offset: offset + info.offset,
                sample_length: length - info.offset.min(length),
                speed,
                gain,
                time: 0.0,
                start_offset: offset_in_block,
                envelope: info.ampeg_start,
                env_stage: 0,
                stage_progress: 0.0,
                env_level: gain,
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
