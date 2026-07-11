//! GPU-accelerated audio renderer for offline export.
//!
//! Uses wgpu compute shaders with multi-chunk sample buffers to handle
//! soundfont data larger than the GPU's max buffer binding size.

use std::sync::Arc;
use wgpu::util::DeviceExt;

const MAX_CHUNKS: usize = 5;
const CHUNK_SIZE: usize = 30_000_000; // 30M f32 = 120MB per chunk

/// Per-voice state that is uploaded to the GPU each block.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVoiceState {
    pub sample_offset: u32,
    pub sample_length: u32,
    pub speed: f32,
    pub gain: f32,
    pub time: f32,
    pub envelope: f32,
    pub env_stage: u32,
    pub env_level: f32,
    pub _pad: u32,
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
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
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
            device, queue, pipeline, bind_group_layout, dummy_buf,
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
                usage: wgpu::BufferUsages::STORAGE,
            })
        }).collect();

        // Create chunk_offsets buffer (uniform, max 6 u32 = 24 bytes)
        let mut offsets: Vec<u32> = Vec::with_capacity(MAX_CHUNKS + 1);
        let mut acc = 0u32;
        for chunk in &self.sample_chunks {
            offsets.push(acc);
            acc += chunk.len() as u32;
        }
        offsets.push(acc); // sentinel (total length)
        // Pad to MAX_CHUNKS+1 entries
        while offsets.len() < MAX_CHUNKS + 1 {
            offsets.push(acc);
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

/// CPU reference implementation.
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];
    for voice in voices.iter_mut() {
        for i in 0..frame_count as usize {
            if voice.env_stage >= 4 { continue; }
            match voice.env_stage {
                0 => { voice.envelope += 0.01; if voice.envelope >= voice.env_level { voice.envelope = voice.env_level; voice.env_stage = 1; } }
                1 => { voice.envelope -= 0.005; if voice.envelope <= 0.7 { voice.envelope = 0.7; voice.env_stage = 2; } }
                2 => {}
                3 => { voice.envelope -= 0.02; if voice.envelope <= 0.0 { voice.envelope = 0.0; voice.env_stage = 4; } }
                _ => {}
            }
            let t = voice.time;
            let idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);
            let a = sample_data[voice.sample_offset as usize + (idx as usize).min(max_idx as usize)];
            let b = sample_data[voice.sample_offset as usize + ((idx + 1) as usize).min(max_idx as usize)];
            let sample = a + (b - a) * frac;
            let out = sample * voice.gain * voice.envelope;
            output[i * 2] += out;
            output[i * 2 + 1] += out;
            voice.time += voice.speed;
        }
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
            speed, gain: 0.5, time: 0.0, envelope: 0.0, env_stage: 0, env_level: 1.0, _pad: 0,
        }).collect()
    }

    fn setup_gpu() -> Option<(GpuAudioRenderer, Vec<f32>)> {
        let mut renderer = GpuAudioRenderer::new_default().ok()?;
        let sample_len = 4096u32;
        let samples: Vec<f32> = (0..4).flat_map(|inst| {
            make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
        }).collect();
        renderer.upload_samples(&samples);
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
}
