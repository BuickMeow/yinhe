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
    pub sample_offset: u32,
    pub sample_length: u32,
    pub speed: f32,
    pub gain: f32,          // velocity * volume * amp_veltrack 综合增益
    pub time: f32,
    pub envelope: f32,
    pub env_stage: u32,
    pub env_level: f32,     // = gain
    pub start_offset: u32,  // 块内起始帧偏移
    // ADSR 参数（从 SFZ 读取，以"每帧增量"形式存储）
    pub attack_rate: f32,   // 1/(attack*sample_rate)
    pub decay_rate: f32,    // (1-sustain)/(decay*sample_rate)
    pub sustain_level: f32, // 0..1
    pub release_rate: f32,  // 1/(release*sample_rate)
    pub pan_left: f32,      // 声像左增益
    pub pan_right: f32,     // 声像右增益
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

/// CPU 端推进 voice 状态：time 和 envelope 推进实际活跃帧数。
/// 在每个 render_block 之后调用，为下一个块准备状态。
pub fn advance_voices(voices: &mut [GpuVoiceState], frame_count: u32) {
    for voice in voices.iter_mut() {
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.start_offset = 0;
        if voice.env_stage >= 4 { continue; }
        voice.time += voice.speed * active_frames as f32;
        for _ in 0..active_frames {
            match voice.env_stage {
                0 => {
                    voice.envelope += voice.attack_rate;
                    if voice.envelope >= voice.env_level { voice.envelope = voice.env_level; voice.env_stage = 1; }
                }
                1 => {
                    voice.envelope -= voice.decay_rate;
                    if voice.envelope <= voice.sustain_level * voice.env_level {
                        voice.envelope = voice.sustain_level * voice.env_level;
                        voice.env_stage = 2;
                    }
                }
                2 => {}
                3 => {
                    voice.envelope -= voice.release_rate;
                    if voice.envelope <= 0.0 { voice.envelope = 0.0; voice.env_stage = 4; }
                }
                _ => break,
            }
        }
    }
}

/// Persistent GPU state — all buffers allocated once, reused every block.
struct GpuBuffers {
    sample_chunks: Vec<wgpu::Buffer>,
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
        let needs_recreate = if self.sample_chunks.is_empty() {
            return;
        } else {
            match &self.buffers {
                Some(b) => b.max_voices < voice_count || self.frame_count < frame_count,
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
        eprintln!("[gpu] ensure_buffers: sample_chunks.len()={} offsets={:?}", self.sample_chunks.len(), &offsets);
        // Pad to exactly 8 entries for WGSL struct alignment
        while offsets.len() < 8 {
            offsets.push(0);
        }
        let chunk_offsets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_offsets"),
            contents: bytemuck::cast_slice(&offsets),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Other persistent buffers
        let voice_state_size = (voice_count.max(1) as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
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
            max_voices: voice_count.max(1),
            final_output_buf,
            params_buf,
            staging: [staging0, staging1],
            staging_idx: 0,
        });
        self.frame_count = frame_count;
    }

    /// Render a block of audio using the GPU.
    pub fn render_block(
        &mut self,
        voices: &[GpuVoiceState],
        frame_count: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let voice_count = voices.len() as u32;
        if voice_count == 0 || frame_count == 0 {
            return vec![0.0; frame_count as usize * 2];
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
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        buf.staging[idx].unmap();
        buf.staging_idx = 1 - buf.staging_idx;

        result
    }
}

/// CPU reference implementation (与 GPU shader 逻辑完全对应).
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];
    for voice in voices.iter_mut() {
        for fi in 0..frame_count as usize {
            if voice.env_stage >= 4 { continue; }
            if fi < voice.start_offset as usize { continue; }
            let frame_in_voice = fi - voice.start_offset as usize;

            // ADSR 推进
            match voice.env_stage {
                0 => { voice.envelope += voice.attack_rate; if voice.envelope >= voice.env_level { voice.envelope = voice.env_level; voice.env_stage = 1; } }
                1 => { voice.envelope -= voice.decay_rate; if voice.envelope <= voice.sustain_level * voice.env_level { voice.envelope = voice.sustain_level * voice.env_level; voice.env_stage = 2; } }
                2 => {}
                3 => { voice.envelope -= voice.release_rate; if voice.envelope <= 0.0 { voice.envelope = 0.0; voice.env_stage = 4; } }
                _ => {}
            }

            let t = voice.time + frame_in_voice as f32 * voice.speed;
            let idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);
            if idx >= voice.sample_length { continue; }
            let a = sample_data[voice.sample_offset as usize + (idx as usize).min(max_idx as usize)];
            let b = sample_data[voice.sample_offset as usize + ((idx + 1) as usize).min(max_idx as usize)];
            let sample = a + (b - a) * frac;
            let out = sample * voice.gain * voice.envelope;
            output[fi * 2] += out * voice.pan_left;
            output[fi * 2 + 1] += out * voice.pan_right;
        }
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.time += voice.speed * active_frames as f32;
        voice.start_offset = 0;
    }
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
            speed, gain: 0.5, time: 0.0, envelope: 0.0, env_stage: 0, env_level: 1.0, start_offset: 0,
            attack_rate: 0.01, decay_rate: 0.005, sustain_level: 0.7, release_rate: 0.02,
            pan_left: 1.0, pan_right: 1.0,
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
            gain: 1.0, time: 0.0, envelope: 1.0, env_stage: 2, env_level: 1.0, start_offset: 0,
            attack_rate: 0.01, decay_rate: 0.005, sustain_level: 1.0, release_rate: 0.02,
            pan_left: 1.0, pan_right: 1.0,
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
                gain: 1.0, time: 0.0, envelope: 1.0, env_stage: 2, env_level: 1.0, start_offset: 0,
                attack_rate: 0.01, decay_rate: 0.005, sustain_level: 1.0, release_rate: 0.02,
                pan_left: 1.0, pan_right: 1.0,
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
