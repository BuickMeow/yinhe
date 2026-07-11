//! GPU-accelerated audio renderer for offline export.
//!
//! Phase 1.5: Persistent buffers + double-buffered readback.
//! All GPU buffers are allocated once and reused across render_block calls,
//! eliminating the per-block allocation overhead that dominated Phase 1.

use std::sync::Arc;
use wgpu::util::DeviceExt;

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
    pub _pad: u32,
}

/// Persistent GPU state — all buffers allocated once, reused every block.
struct GpuBuffers {
    /// Soundfont sample data (uploaded once, rarely changes).
    sample_buf: wgpu::Buffer,
    /// Per-voice state — written via queue.write_buffer each block.
    voice_state_buf: wgpu::Buffer,
    /// Max voice capacity this buffer was created for.
    max_voices: u32,
    /// Intermediate per-voice output (voice_count × frame_count × 2 × f32).
    voice_output_buf: wgpu::Buffer,
    /// Final mixed stereo output.
    final_output_buf: wgpu::Buffer,
    /// Uniform params — updated each block.
    params_buf: wgpu::Buffer,
    /// Double-buffered staging buffers for readback.
    staging: [wgpu::Buffer; 2],
    /// Which staging buffer to use next (0 or 1).
    staging_idx: usize,
    /// Bind groups for voice pass (group 0).
    voice_bind_groups: [wgpu::BindGroup; 2],
    /// Bind groups for merge pass (group 1).
    merge_bind_groups: [wgpu::BindGroup; 2],
}

/// GPU-accelerated audio renderer with persistent buffers.
pub struct GpuAudioRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    voice_pipeline: wgpu::ComputePipeline,
    merge_pipeline: wgpu::ComputePipeline,
    voice_layout: wgpu::BindGroupLayout,
    merge_layout: wgpu::BindGroupLayout,
    buffers: Option<GpuBuffers>,
    /// Pending sample buffer (set by upload_samples, consumed by ensure_buffers).
    pending_samples: Option<wgpu::Buffer>,
    /// Max frame_count seen so far (for buffer sizing).
    frame_count: u32,
}

impl GpuAudioRenderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader_source = include_str!("shaders/voice_render.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voice_render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // ── Voice render pipeline ──
        let voice_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("voice_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let voice_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voice_pipeline_layout"),
                bind_group_layouts: &[Some(&voice_layout)],
                immediate_size: 0,
            });

        let voice_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voice_pipeline"),
            layout: Some(&voice_pipeline_layout),
            module: &shader_module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // ── Merge pipeline ──
        let merge_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("merge_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let merge_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("merge_pipeline_layout"),
                bind_group_layouts: &[None, Some(&merge_layout)],
                immediate_size: 0,
            });

        let merge_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("merge_pipeline"),
            layout: Some(&merge_pipeline_layout),
            module: &shader_module,
            entry_point: Some("merge_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            voice_pipeline,
            merge_pipeline,
            voice_layout,
            merge_layout,
            buffers: None,
            pending_samples: None,
            frame_count: 0,
        })
    }

    /// Upload soundfont sample data. Call once before rendering begins.
    /// The data is kept in a persistent GPU buffer for the renderer's lifetime.
    pub fn upload_samples(&mut self, sample_data: &[f32]) {
        let sample_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gpu_samples"),
                contents: bytemuck::cast_slice(sample_data),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // Invalidate existing buffers so they get rebuilt with the new sample data.
        self.buffers = None;
        self.pending_samples = Some(sample_buf);
    }

    /// Ensure persistent buffers exist and are sized correctly.
    /// Recreates buffers only when capacity is exceeded.
    fn ensure_buffers(&mut self, voice_count: u32, frame_count: u32) {
        // Determine if we need to recreate buffers.
        let needs_recreate = match (&self.buffers, &self.pending_samples) {
            (Some(b), _) => b.max_voices < voice_count || self.frame_count < frame_count,
            (None, Some(_)) => true,
            (None, None) => {
                // No buffers and no pending samples — this means upload_samples was never called.
                // This shouldn't happen in normal usage, but we'll just return.
                return;
            }
        };

        if !needs_recreate {
            return;
        }

        let device = &self.device;

        // Use pending samples if available, otherwise keep existing.
        let sample_buf = if let Some(buf) = self.pending_samples.take() {
            buf
        } else {
            // Capacity increase only — keep existing sample_buf.
            self.buffers.as_ref().unwrap().sample_buf.clone()
        };

        let voice_state_size =
            (voice_count.max(1) as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        let voice_output_size = (voice_count.max(1) as usize
            * frame_count.max(1) as usize
            * 2
            * std::mem::size_of::<f32>()) as u64;
        let final_output_size = (frame_count.max(1) as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let params_size = std::mem::size_of::<RenderParams>() as u64;

        let voice_state_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_voice_states"),
            size: voice_state_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let voice_output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_voice_outputs"),
            size: voice_output_size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let final_output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_final_output"),
            size: final_output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_params"),
            size: params_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_staging_0"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_staging_1"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Build bind groups for both staging slots (they share the same buffers
        // except params — but params_buf is the same, so we build once per slot).
        let voice_bind_group_0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voice_bg_0"),
            layout: &self.voice_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sample_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: voice_state_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: voice_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });
        let voice_bind_group_1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voice_bg_1"),
            layout: &self.voice_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sample_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: voice_state_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: voice_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let merge_bind_group_0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("merge_bg_0"),
            layout: &self.merge_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: voice_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: final_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });
        let merge_bind_group_1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("merge_bg_1"),
            layout: &self.merge_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: voice_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: final_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        self.buffers = Some(GpuBuffers {
            sample_buf,
            voice_state_buf,
            max_voices: voice_count.max(1),
            voice_output_buf,
            final_output_buf,
            params_buf,
            staging: [staging0, staging1],
            staging_idx: 0,
            voice_bind_groups: [voice_bind_group_0, voice_bind_group_1],
            merge_bind_groups: [merge_bind_group_0, merge_bind_group_1],
        });
        self.frame_count = frame_count;
    }

    /// Render a block of audio using the GPU with persistent buffers.
    ///
    /// `sample_data` is only used on the first call (via `upload_samples`).
    /// `voices` are uploaded each block via `queue.write_buffer` (no allocation).
    ///
    /// Returns a stereo f32 buffer of length `frame_count * 2`.
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

        // ── Upload voice states (only the dirty portion) ──
        let voice_state_size = (voice_count as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        self.queue
            .write_buffer(&buf.voice_state_buf, 0, bytemuck::cast_slice(voices));

        // ── Upload params ──
        let params = RenderParams {
            frame_count,
            voice_count,
            sample_rate,
            _pad: 0,
        };
        self.queue
            .write_buffer(&buf.params_buf, 0, bytemuck::bytes_of(&params));

        // ── Encode compute passes ──
        let idx = buf.staging_idx;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("audio_render"),
            });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voice_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.voice_pipeline);
            cpass.set_bind_group(0, &buf.voice_bind_groups[idx], &[]);
            let wg = (voice_count + 255) / 256;
            cpass.dispatch_workgroups(wg, 1, 1);
        }

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("merge_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.merge_pipeline);
            cpass.set_bind_group(1, &buf.merge_bind_groups[idx], &[]);
            let wg = (frame_count + 255) / 256;
            cpass.dispatch_workgroups(wg, 1, 1);
        }

        // ── Copy to staging buffer ──
        let final_output_size =
            (frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        encoder.copy_buffer_to_buffer(
            &buf.final_output_buf,
            0,
            &buf.staging[idx],
            0,
            final_output_size,
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // ── Readback ──
        let buffer_slice = buf.staging[idx].slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        buf.staging[idx].unmap();

        // ── Swap staging buffer for next call ──
        buf.staging_idx = 1 - buf.staging_idx;

        result
    }
}

/// CPU reference implementation of the same voice rendering logic.
/// Used for correctness validation and performance comparison.
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];

    for voice in voices.iter_mut() {
        for i in 0..frame_count as usize {
            if voice.env_stage >= 4 {
                continue;
            }
            match voice.env_stage {
                0 => {
                    voice.envelope += 0.01;
                    if voice.envelope >= voice.env_level {
                        voice.envelope = voice.env_level;
                        voice.env_stage = 1;
                    }
                }
                1 => {
                    voice.envelope -= 0.005;
                    if voice.envelope <= 0.7 {
                        voice.envelope = 0.7;
                        voice.env_stage = 2;
                    }
                }
                2 => {}
                3 => {
                    voice.envelope -= 0.02;
                    if voice.envelope <= 0.0 {
                        voice.envelope = 0.0;
                        voice.env_stage = 4;
                    }
                }
                _ => {}
            }
            let t = voice.time;
            let idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);
            let a = sample_data
                [voice.sample_offset as usize + (idx as usize).min(max_idx as usize)];
            let b = sample_data[voice.sample_offset as usize
                + ((idx + 1) as usize).min(max_idx as usize)];
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

    fn make_sine_samples(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin())
            .collect()
    }

    fn make_voices(sample_len: u32, count: u32, speed: f32) -> Vec<GpuVoiceState> {
        (0..count)
            .map(|i| GpuVoiceState {
                sample_offset: (i % 4) * sample_len,
                sample_length: sample_len,
                speed,
                gain: 0.5,
                time: 0.0,
                envelope: 0.0,
                env_stage: 0,
                env_level: 1.0,
                _pad: 0,
            })
            .collect()
    }

    fn setup_gpu() -> Option<(GpuAudioRenderer, Vec<f32>)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(a) => a,
            Err(_) => return None,
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            },
        ))
        .unwrap();

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let mut renderer = GpuAudioRenderer::new(device, queue).unwrap();

        let sample_len = 4096u32;
        let samples: Vec<f32> = (0..4)
            .flat_map(|inst| {
                make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
            })
            .collect();
        renderer.upload_samples(&samples);
        Some((renderer, samples))
    }

    #[test]
    fn phase15_smoke_test() {
        let (mut renderer, samples) = match setup_gpu() {
            Some(v) => v,
            None => {
                eprintln!("No GPU adapter, skipping");
                return;
            }
        };
        let sample_len = 4096u32;
        let voices = make_voices(sample_len, 16, 1.0);
        let result = renderer.render_block(&voices, 1024, 44100);
        assert_eq!(result.len(), 1024 * 2);
        let max_abs = result.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(max_abs > 0.0, "GPU output should be non-zero");
    }

    #[test]
    fn phase15_persistent_buffer_reuse() {
        let (mut renderer, samples) = match setup_gpu() {
            Some(v) => v,
            None => {
                eprintln!("No GPU adapter, skipping");
                return;
            }
        };
        let sample_len = 4096u32;

        // Render multiple blocks — buffers should be reused (no reallocation).
        for block in 0..5 {
            let voices = make_voices(sample_len, 64, 1.0);
            let result = renderer.render_block(&voices, 1024, 44100);
            assert_eq!(
                result.len(),
                1024 * 2,
                "Block {block} output length wrong"
            );
        }
    }

    #[test]
    fn phase15_benchmark() {
        let (mut renderer, _samples) = match setup_gpu() {
            Some(v) => v,
            None => {
                eprintln!("No GPU adapter, skipping benchmark");
                return;
            }
        };
        let sample_len = 4096u32;
        let frame_count = 4096u32;

        for &voice_count in &[4, 16, 64, 256, 1024, 4096] {
            // Warm up
            let voices = make_voices(sample_len, voice_count, 1.0);
            for _ in 0..3 {
                let _ = renderer.render_block(&voices, frame_count, 44100);
            }

            // GPU benchmark
            let gpu_start = std::time::Instant::now();
            for _ in 0..10 {
                let _ = renderer.render_block(&voices, frame_count, 44100);
            }
            let gpu_elapsed = gpu_start.elapsed();

            // CPU benchmark
            let cpu_start = std::time::Instant::now();
            for _ in 0..10 {
                let mut v = make_voices(sample_len, voice_count, 1.0);
                let _ = cpu_render_voices(&samples_for_bench(sample_len), &mut v, frame_count);
            }
            let cpu_elapsed = cpu_start.elapsed();

            let speedup = cpu_elapsed.as_secs_f64() / gpu_elapsed.as_secs_f64();
            eprintln!(
                "Voices={voice_count:>5}: CPU={:>8.2?} GPU={:>8.2?} speedup={speedup:.2}x",
                cpu_elapsed / 10,
                gpu_elapsed / 10,
            );
        }
    }

    fn samples_for_bench(sample_len: u32) -> Vec<f32> {
        (0..4)
            .flat_map(|inst| {
                make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
            })
            .collect()
    }
}
