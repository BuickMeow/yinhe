//! GPU-accelerated audio renderer for offline export.
//!
//! Uses wgpu compute shaders with multi-chunk sample buffers to handle
//! soundfont data larger than the GPU's max buffer binding size.

use std::sync::Arc;
use wgpu::util::DeviceExt;

const MAX_CHUNKS: usize = 5;
const CHUNK_SIZE: usize = 30_000_000; // 30M f32 = 120MB per chunk

/// Per-voice state that is uploaded to the GPU each block.
/// 布局必须与 WGSL 的 VoiceState 结构体严格对应。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVoiceState {
    // Sample playback
    pub sample_offset: u32,
    pub sample_length: u32,
    pub speed: f32,
    pub gain: f32,
    pub time: f32,
    pub start_offset: u32,  // 块内起始帧偏移
    // Envelope state at start of block
    pub envelope: f32,       // 当前 envelope 值
    pub env_stage: u32,      // 0=Delay,1=Attack,2=Hold,3=Decay,4=Sustain,5=Release,6=Finished
    pub stage_progress: f32, // 当前阶段已用帧数
    // Envelope parameters
    pub env_level: f32,      // peak = gain
    pub sustain_level: f32,  // 0..1
    pub env_start: f32,      // ampeg_start (0..1)
    // Stage durations (frames)
    pub delay_frames: f32,
    pub attack_frames: f32,
    pub hold_frames: f32,
    pub decay_frames: f32,
    pub release_frames: f32,
    // Pan
    pub pan_left: f32,
    pub pan_right: f32,
    // Loop
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_mode: u32,     // 0=NoLoop, 1=LoopContinuous, 2=LoopSustain, 3=OneShot
}

/// Uniform buffer for render parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderParams {
    pub frame_count: u32,
    pub voice_count: u32,
    pub sample_rate: u32,
    pub sample_chunk_count: u32,
}

/// CPU 端推进 voice 状态：用解析公式直接计算，不逐帧迭代。
/// 7 阶段: 0=Delay, 1=Attack(线性), 2=Hold, 3=Decay(指数), 4=Sustain, 5=Release(指数), 6=Finished
pub fn advance_voices(voices: &mut [GpuVoiceState], frame_count: u32) {
    for voice in voices.iter_mut() {
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.start_offset = 0;
        if voice.env_stage >= 6 || active_frames == 0 { continue; }
        voice.time += voice.speed * active_frames as f32;

        // 循环回绕
        let has_loop = voice.loop_mode > 0 && voice.loop_end > voice.loop_start;
        if has_loop && voice.time >= voice.loop_end as f32 {
            let loop_len = (voice.loop_end - voice.loop_start) as f32;
            if loop_len > 0.0 {
                voice.time = voice.loop_start as f32 + ((voice.time - voice.loop_start as f32) % loop_len);
            }
        }

        let peak = voice.env_level;
        let sus = voice.sustain_level * peak;
        let mut remaining = active_frames as f32;

        while remaining > 0.0 && voice.env_stage < 6 {
            match voice.env_stage {
                0 => { // Delay
                    let dur = voice.delay_frames - voice.stage_progress;
                    if remaining < dur {
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        remaining -= dur;
                        voice.env_stage = 1;
                        voice.stage_progress = 0.0;
                    }
                }
                1 => { // Attack: 线性
                    let dur = voice.attack_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.attack_frames;
                        voice.envelope = voice.env_start + (peak - voice.env_start) * t;
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = peak;
                        remaining -= dur;
                        voice.env_stage = 2;
                        voice.stage_progress = 0.0;
                    }
                }
                2 => { // Hold
                    let dur = voice.hold_frames - voice.stage_progress;
                    if remaining < dur {
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        remaining -= dur;
                        voice.env_stage = 3;
                        voice.stage_progress = 0.0;
                    }
                }
                3 => { // Decay: 指数 (1-t)^8
                    let dur = voice.decay_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.decay_frames;
                        voice.envelope = sus + (peak - sus) * (1.0 - t).powi(8);
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = sus;
                        remaining -= dur;
                        voice.env_stage = 4;
                        voice.stage_progress = 0.0;
                    }
                }
                4 => { remaining = 0.0; } // Sustain: 无限
                5 => { // Release: 指数 (1-t)^8
                    let dur = voice.release_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.release_frames;
                        voice.envelope = voice.env_start * (1.0 - t).powi(8);
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = 0.0;
                        remaining -= dur;
                        voice.env_stage = 6;
                        voice.stage_progress = 0.0;
                    }
                }
                _ => break,
            }
        }
    }
}

/// Persistent GPU state — all buffers allocated once, reused every block.
struct GpuBuffers {
    #[allow(dead_code)]
    sample_chunks: Vec<wgpu::Buffer>,
    #[allow(dead_code)]
    chunk_offsets_buf: wgpu::Buffer,
    chunk_count: u32,
    voice_state_buf: wgpu::Buffer,
    max_voices: u32,
    final_output_buf: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    staging: [wgpu::Buffer; 2],
    staging_idx: usize,
    bind_groups: [wgpu::BindGroup; 2],
}

/// GPU-accelerated audio renderer with persistent buffers.
pub struct GpuAudioRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::ComputePipeline,
    #[allow(dead_code)]
    pipeline_layout: wgpu::PipelineLayout,
    bind_group_layout: wgpu::BindGroupLayout,
    dummy_buf: wgpu::Buffer,
    buffers: Option<GpuBuffers>,
    /// Persistent copy of sample data chunks (never consumed, reused for buffer rebuilds).
    sample_chunks: Vec<Vec<f32>>,
    frame_count: u32,
}

impl GpuAudioRenderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader_source = include_str!("shaders/voice_render.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voice_render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // 9-binding layout (max 8 storage buffers):
        // 0: params+chunk_offsets (uniform)
        // 1: voice_states (storage read_write)
        // 2: final_output (storage read_write)
        // 3-7: 5 sample chunks (storage read)
        // 8: chunk_offsets (uniform, separate)
        let mut entries = Vec::with_capacity(9);
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 0, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        });
        for i in 0..MAX_CHUNKS {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: (3 + i) as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            });
        }
        // chunk_offsets uniform (binding 8)
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 8, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("audio_render_bgl"),
            entries: &entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("audio_render_pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("audio_render_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Dummy 1-element buffer for unused sample chunks
        let dummy_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dummy"),
            contents: bytemuck::bytes_of(&0.0f32),
            usage: wgpu::BufferUsages::STORAGE,
        });

        Ok(Self {
            device, queue, pipeline, pipeline_layout, bind_group_layout, dummy_buf,
            buffers: None, sample_chunks: Vec::new(), frame_count: 0,
        })
    }

    /// Create a renderer with its own wgpu device/queue (for standalone use).
    pub fn new_default() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|_| "No GPU adapter found")?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("gpu_audio"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits {
                    max_storage_buffer_binding_size: 512 * 1024 * 1024,
                    max_buffer_size: 512 * 1024 * 1024,
                    ..wgpu::Limits::default()
                },
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            },
        ))
        .map_err(|e| format!("Failed to create device: {}", e))?;
        Self::new(Arc::new(device), Arc::new(queue))
    }

    /// Upload soundfont sample data. Splits into chunks for GPU buffer limits.
    pub fn upload_samples(&mut self, sample_data: &[f32]) {
        self.sample_chunks = sample_data.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();
        self.buffers = None;
    }

    fn ensure_buffers(&mut self, voice_count: u32, frame_count: u32) {
        // 幂增长策略：向上取整到 2 的幂次，避免每个 block 都重建缓冲区
        let rounded_voices = voice_count.max(64).next_power_of_two();
        let needs_recreate = if self.sample_chunks.is_empty() {
            return;
        } else {
            match &self.buffers {
                Some(b) => b.max_voices < rounded_voices || self.frame_count < frame_count,
                None => true,
            }
        };
        if !needs_recreate {
            return;
        }

        let device = &self.device;
        let chunk_count = self.sample_chunks.len().min(MAX_CHUNKS) as u32;

        // Create sample chunk buffers
        let sample_chunks: Vec<wgpu::Buffer> = self.sample_chunks.iter().map(|data| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sample_chunk"),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            })
        }).collect();

        // Create chunk_offsets buffer (uniform, padded to 32 bytes = 8 u32 for 16-byte alignment)
        let mut offsets: Vec<u32> = Vec::with_capacity(8);
        let mut acc = 0u32;
        for chunk in &self.sample_chunks {
            offsets.push(acc);
            acc += chunk.len() as u32;
        }
        offsets.push(acc); // total = sentinel
        // Pad to exactly 8 entries for WGSL struct alignment
        while offsets.len() < 8 {
            offsets.push(0);
        }
        let chunk_offsets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_offsets"),
            contents: bytemuck::cast_slice(&offsets),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Other persistent buffers（用 rounded_voices 分配，和 max_voices 一致）
        let voice_state_size = (rounded_voices as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        let final_output_size = (frame_count.max(1) as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let params_size = std::mem::size_of::<RenderParams>() as u64;

        let voice_state_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_voice_states"), size: voice_state_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let final_output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_final_output"), size: final_output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_params"), size: params_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let staging0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_0"), size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let staging1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_1"), size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });

        // Build bind group entries
        let make_bg = |p: &wgpu::Buffer, v: &wgpu::Buffer, f: &wgpu::Buffer,
                       co: &wgpu::Buffer, sc: &[wgpu::Buffer], db: &wgpu::Buffer| {
            let mut bg_entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: p.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: v.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: f.as_entire_binding() },
            ];
            for i in 0..MAX_CHUNKS {
                let buf = if (i as u32) < chunk_count {
                    sc[i].as_entire_binding()
                } else {
                    db.as_entire_binding()
                };
                bg_entries.push(wgpu::BindGroupEntry { binding: (3 + i) as u32, resource: buf });
            }
            bg_entries.push(wgpu::BindGroupEntry { binding: 8, resource: co.as_entire_binding() });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("audio_bg"),
                layout: &self.bind_group_layout,
                entries: &bg_entries,
            })
        };

        self.buffers = Some(GpuBuffers {
            bind_groups: [
                make_bg(&params_buf, &voice_state_buf, &final_output_buf, &chunk_offsets_buf, &sample_chunks, &self.dummy_buf),
                make_bg(&params_buf, &voice_state_buf, &final_output_buf, &chunk_offsets_buf, &sample_chunks, &self.dummy_buf),
            ],
            sample_chunks,
            chunk_offsets_buf,
            chunk_count,
            voice_state_buf,
            max_voices: rounded_voices,
            final_output_buf,
            params_buf,
            staging: [staging0, staging1],
            staging_idx: 0,
        });
        self.frame_count = frame_count;
    }

    /// Render a block of audio using the GPU.
    /// 渲染一块音频。输出写入 `output`（长度 = frame_count * 2，立体声交错）。
    /// 返回实际 voice 数量（0 表示静音）。
    pub fn render_into(
        &mut self,
        voices: &[GpuVoiceState],
        output: &mut [f32],
        sample_rate: u32,
    ) -> u32 {
        let frame_count = (output.len() / 2) as u32;
        let voice_count = voices.len() as u32;
        if voice_count == 0 || frame_count == 0 {
            output.fill(0.0);
            return 0;
        }

        self.ensure_buffers(voice_count, frame_count);
        let buf = self.buffers.as_mut().unwrap();

        self.queue.write_buffer(&buf.voice_state_buf, 0, bytemuck::cast_slice(voices));
        let params = RenderParams {
            frame_count,
            voice_count,
            sample_rate,
            sample_chunk_count: buf.chunk_count,
        };
        self.queue.write_buffer(&buf.params_buf, 0, bytemuck::bytes_of(&params));

        let idx = buf.staging_idx;
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("audio_render"),
        });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voice_pass"), ..Default::default()
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &buf.bind_groups[idx], &[]);
            cpass.dispatch_workgroups(frame_count, 1, 1);
        }

        let final_output_size = (frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        encoder.copy_buffer_to_buffer(&buf.final_output_buf, 0, &buf.staging[idx], 0, final_output_size);
        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = buf.staging[idx].slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| { let _ = sender.send(result); });
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        receiver.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let gpu_output: &[f32] = bytemuck::cast_slice(&data);
        output[..gpu_output.len()].copy_from_slice(gpu_output);
        drop(data);
        buf.staging[idx].unmap();
        buf.staging_idx = 1 - buf.staging_idx;

        voice_count
    }

    /// 渲染一块音频（返回新分配的 Vec，兼容旧接口）。
    pub fn render_block(
        &mut self,
        voices: &[GpuVoiceState],
        frame_count: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let mut output = vec![0.0; frame_count as usize * 2];
        self.render_into(voices, &mut output, sample_rate);
        output
    }
}

/// CPU reference implementation (与 GPU shader 逻辑完全对应).
/// 7 阶段: 0=Delay, 1=Attack, 2=Hold, 3=Decay, 4=Sustain, 5=Release, 6=Finished
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];
    for voice in voices.iter_mut() {
        for fi in 0..frame_count as usize {
            if voice.env_stage >= 6 { continue; }
            if fi < voice.start_offset as usize { continue; }
            let frame_in_voice = fi - voice.start_offset as usize;

            let peak = voice.env_level;
            let sus = voice.sustain_level * peak;
            let progress = voice.stage_progress + frame_in_voice as f32;

            // 解析计算 envelope
            let env = match voice.env_stage {
                0 => voice.env_start, // Delay
                1 => { // Attack: 线性
                    let t = if voice.attack_frames > 0.0 { (progress / voice.attack_frames).min(1.0) } else { 1.0 };
                    voice.env_start + (peak - voice.env_start) * t
                }
                2 => peak, // Hold
                3 => { // Decay: 指数 (1-t)^8
                    let t = if voice.decay_frames > 0.0 { (progress / voice.decay_frames).min(1.0) } else { 1.0 };
                    sus + (peak - sus) * (1.0 - t).powi(8)
                }
                4 => sus, // Sustain
                5 => { // Release: 指数 (1-t)^8
                    let t = if voice.release_frames > 0.0 { (progress / voice.release_frames).min(1.0) } else { 1.0 };
                    voice.env_start * (1.0 - t).powi(8)
                }
                _ => 0.0,
            };

            let t = voice.time + frame_in_voice as f32 * voice.speed;
            let mut idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);

            // 循环回绕
            let has_loop = voice.loop_mode > 0 && voice.loop_end > voice.loop_start;
            if has_loop && idx >= voice.loop_end {
                let loop_len = voice.loop_end - voice.loop_start;
                if loop_len > 0 {
                    idx = voice.loop_start + ((idx - voice.loop_start) % loop_len);
                }
            }

            if idx >= voice.sample_length { continue; }
            let a = sample_data[voice.sample_offset as usize + (idx as usize).min(max_idx as usize)];
            let b = sample_data[voice.sample_offset as usize + ((idx + 1) as usize).min(max_idx as usize)];
            let sample = a + (b - a) * frac;
            let out = sample * voice.gain * env;
            output[fi * 2] += out * voice.pan_left;
            output[fi * 2 + 1] += out * voice.pan_right;
        }
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.time += voice.speed * active_frames as f32;
        voice.start_offset = 0;
    }
    // advance_voices handles the state progression
    advance_voices(voices, frame_count);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sine_samples(len: usize, freq: f32, sr: f32) -> Vec<f32> {
        (0..len).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin()).collect()
    }

    fn make_voices(sample_len: u32, count: u32, speed: f32) -> Vec<GpuVoiceState> {
        (0..count).map(|i| GpuVoiceState {
            sample_offset: (i % 4) * sample_len, sample_length: sample_len,
            speed, gain: 0.5, time: 0.0, start_offset: 0,
            envelope: 0.0, env_stage: 4, stage_progress: 0.0,
            env_level: 1.0, sustain_level: 1.0, env_start: 0.0,
            delay_frames: 0.0, attack_frames: 1.0, hold_frames: 0.0,
            decay_frames: 1.0, release_frames: 1.0,
            pan_left: 1.0, pan_right: 1.0,
            loop_start: 0, loop_end: 0, loop_mode: 0,
        }).collect()
    }

    fn setup_gpu() -> Option<(GpuAudioRenderer, Vec<f32>)> {
        let mut renderer = GpuAudioRenderer::new_default().ok()?;
        let sample_len = 4096u32;
        let samples: Vec<f32> = (0..4).flat_map(|inst| {
            make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
        }).collect();
        renderer.upload_samples(&samples);
        let limits = renderer.device.limits();
        eprintln!("GPU limits: min_storage_buf_align={} max_buf_binding={}", limits.min_storage_buffer_offset_alignment, limits.max_storage_buffer_binding_size);
        Some((renderer, samples))
    }

    fn bench_samples(sample_len: u32) -> Vec<f32> {
        (0..4).flat_map(|inst| make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)).collect()
    }

    #[test]
    fn phase15_single_pass_smoke() {
        let (mut renderer, _samples) = match setup_gpu() { Some(r) => r, None => { eprintln!("No GPU"); return; } };
        let voices = make_voices(4096, 16, 1.0);
        let result = renderer.render_block(&voices, 1024, 44100);
        assert_eq!(result.len(), 1024 * 2);
        assert!(result.iter().fold(0.0f32, |m, &s| m.max(s.abs())) > 0.0);
    }

    #[test]
    fn phase15_benchmark() {
        let (mut renderer, _samples) = match setup_gpu() { Some(r) => r, None => { eprintln!("No GPU"); return; } };
        let sample_len = 4096u32;
        let samples = bench_samples(sample_len);
        let frame_count = 1024u32;

        for &vc in &[4, 16, 64, 256, 1024, 4096, 15000] {
            let voices = make_voices(sample_len, vc, 1.0);
            for _ in 0..3 { let _ = renderer.render_block(&voices, frame_count, 44100); }
            let n = 10;
            let gpu_start = std::time::Instant::now();
            for _ in 0..n { let _ = renderer.render_block(&voices, frame_count, 44100); }
            let gpu_per_block = gpu_start.elapsed() / n;
            let cpu_start = std::time::Instant::now();
            for _ in 0..n {
                let mut v = make_voices(sample_len, vc, 1.0);
                let _ = cpu_render_voices(&samples, &mut v, frame_count);
            }
            let cpu_per_block = cpu_start.elapsed() / n;
            let speedup = cpu_per_block.as_secs_f64() / gpu_per_block.as_secs_f64();
            eprintln!("Voices={vc:>6}: CPU={cpu_per_block:>8.2?} GPU={gpu_per_block:>8.2?} speedup={speedup:.2}x");
        }
    }

    #[test]
    fn gpu_vs_cpu_correctness() {
        let (mut renderer, samples) = match setup_gpu() { Some(r) => r, None => { eprintln!("No GPU"); return; } };
        let sample_len = 4096u32;
        eprintln!("Sample data: len={} first5={:?}", samples.len(), &samples[..5.min(samples.len())]);

        // Test: manually create a 1-chunk renderer with known data
        let test_data: Vec<f32> = (0..1024).map(|i| (i as f32 / 1024.0)).collect();
        renderer.upload_samples(&test_data);
        // Force recreate buffers
        renderer.buffers = None;

        // voice 在 sustain 阶段: envelope = sustain_level * env_level = 1.0 * 1.0 = 1.0
        let gpu_voices = vec![GpuVoiceState {
            sample_offset: 0, sample_length: 1024, speed: 1.0,
            gain: 1.0, time: 0.0, start_offset: 0,
            envelope: 1.0, env_stage: 4, stage_progress: 0.0,
            env_level: 1.0, sustain_level: 1.0, env_start: 0.0,
            delay_frames: 0.0, attack_frames: 1.0, hold_frames: 0.0,
            decay_frames: 1.0, release_frames: 1.0,
            pan_left: 1.0, pan_right: 1.0,
            loop_start: 0, loop_end: 0, loop_mode: 0,
        }];
        let gpu_out = renderer.render_block(&gpu_voices, 8, 44100);

        eprintln!("GPU output (8 frames, should be test_data[0..8]):");
        for i in 0..8 {
            eprintln!("  frame[{i}]: gpu={:.6} expected={:.6}", gpu_out[i*2], test_data[i]);
        }

        // Fresh buffer test: create ALL buffers from scratch, no reuse
        let device = &renderer.device;
        eprintln!("  test_data len={} first3={:?}", test_data.len(), &test_data[..3]);
        // Try two-step approach: create buffer, then write data
        let sample_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fresh_sample"),
            size: (test_data.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        renderer.queue.write_buffer(&sample_buf, 0, bytemuck::cast_slice(&test_data));
        renderer.queue.submit(std::iter::empty::<wgpu::CommandBuffer>()); // flush the write
        eprintln!("  sample_buf created: size={}", sample_buf.size());
        let voice_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fresh_voice"),
            contents: bytemuck::cast_slice(&[GpuVoiceState {
                sample_offset: 0, sample_length: 1024, speed: 1.0,
                gain: 1.0, time: 0.0, start_offset: 0,
                envelope: 1.0, env_stage: 4, stage_progress: 0.0,
                env_level: 1.0, sustain_level: 1.0, env_start: 0.0,
                delay_frames: 0.0, attack_frames: 1.0, hold_frames: 0.0,
                decay_frames: 1.0, release_frames: 1.0,
                pan_left: 1.0, pan_right: 1.0,
                loop_start: 0, loop_end: 0, loop_mode: 0,
            }]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fresh_output"), size: 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let params_data = RenderParams { frame_count: 8, voice_count: 1, sample_rate: 44100, sample_chunk_count: 1 };
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fresh_params"), contents: bytemuck::bytes_of(&params_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let offsets_data = [0u32, 1024, 0, 0, 0, 1024, 0, 0]; // o0,o1,o2,o3,o4,total,_pad0,_pad1
        eprintln!("  offsets_data={:?}", offsets_data);
        eprintln!("  offsets_data size={} bytes", offsets_data.len() * 4);
        let offsets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fresh_offsets"), contents: bytemuck::cast_slice(&offsets_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fresh_staging"), size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fresh_bg"), layout: &renderer.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: voice_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: output_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sample_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: offsets_buf.as_entire_binding() },
            ],
        });
        // Create const_one pipeline for verification test
        let const_shader = include_str!("shaders/const_one.wgsl");
        let const_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("const_one"), source: wgpu::ShaderSource::Wgsl(const_shader.into()),
        });
        let const_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("const_pipe"), layout: Some(&renderer.pipeline_layout),
            module: &const_module, entry_point: Some("vs_main"),
            compilation_options: Default::default(), cache: None,
        });
        // Verify sample_chunks[0] has test data
        let bufs = renderer.buffers.as_ref().unwrap();
        let verify_size = bufs.sample_chunks[0].size();
        let verify_staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("verify"), size: verify_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let mut enc_v = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("verify") });
        enc_v.copy_buffer_to_buffer(&bufs.sample_chunks[0], 0, &verify_staging, 0, verify_size);
        renderer.queue.submit(std::iter::once(enc_v.finish()));
        let vslice = verify_staging.slice(..);
        let (vtx, vrx) = std::sync::mpsc::channel();
        vslice.map_async(wgpu::MapMode::Read, move |r| { let _ = vtx.send(r); });
        let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        vrx.recv().unwrap().unwrap();
        let vdata = vslice.get_mapped_range();
        let verify: Vec<f32> = bytemuck::cast_slice(&vdata).to_vec();
        drop(vdata); verify_staging.unmap();
        eprintln!("Verify sample_chunks[0]: len={} [0]={} [1]={} [2]={}", verify.len(), verify[0], verify[1], verify[2]);

        // Create test_read pipeline (reads chunk_0[fi] directly)
        let test_read_shader = include_str!("shaders/test_read.wgsl");
        let test_read_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test_read"), source: wgpu::ShaderSource::Wgsl(test_read_shader.into()),
        });
        let test_read_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("test_read_pipe"), layout: Some(&renderer.pipeline_layout),
            module: &test_read_module, entry_point: Some("vs_main"),
            compilation_options: Default::default(), cache: None,
        });
        // Test: write test data into a FRESH buffer (not sample_chunks[0])
        let fresh_sample = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fresh_sample2"), contents: bytemuck::cast_slice(&test_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output_buf3 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test3_output"), size: 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let staging_buf3 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test3_staging"), size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let bg3 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test3_bg"), layout: &renderer.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: voice_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: output_buf3.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: fresh_sample.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: renderer.dummy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: offsets_buf.as_entire_binding() },
            ],
        });
        let mut enc3 = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("test3") });
        { let mut cp = enc3.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("test3"), ..Default::default() });
          cp.set_pipeline(&test_read_pipeline); cp.set_bind_group(0, &bg3, &[]);
          cp.dispatch_workgroups(8, 1, 1); }
        enc3.copy_buffer_to_buffer(&output_buf3, 0, &staging_buf3, 0, 64);
        renderer.queue.submit(std::iter::once(enc3.finish()));
        let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let slice3 = staging_buf3.slice(..);
        let (tx3, rx3) = std::sync::mpsc::channel();
        slice3.map_async(wgpu::MapMode::Read, move |r| { let _ = tx3.send(r); });
        let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx3.recv().unwrap().unwrap();
        let data3 = slice3.get_mapped_range();
        let out3: Vec<f32> = bytemuck::cast_slice(&data3).to_vec();
        drop(data3); staging_buf3.unmap();
        eprintln!("test_read with fresh sample_buf:");
        for i in 0..8 {
            eprintln!("  frame[{i}]: gpu={:.6} expected={:.6}", out3[i*2], test_data[i]);
        }
    }
}
